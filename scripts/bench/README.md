# Mail-catcher runtime benchmark

Python 3 stdlib only. One tool at a time, fresh storage per run, results
appended as JSON lines to `results/<tool>.jsonl` (gitignored; override with
`--output-dir`).

## Prerequisites

- A release RustMail binary: `make build` (or `cargo build --release -p rustmail-server`).
  Defaults to `target/release/rustmail` at the repo root; override with `--binary`.
- For `--tool rustmail-docker-glibc`: a Linux glibc build, e.g.

  ```sh
  docker run --rm -v "$PWD:/src" -v "$PWD/target-linux:/target" -w /src -e CARGO_TARGET_DIR=/target \
    rust:1-bookworm cargo build --release -p rustmail-server
  ```

  Defaults to `target-linux/release/rustmail`; override with `--linux-binary`.
- Docker, with `axllent/mailpit:latest`, `mailhog/mailhog:latest` (amd64, emulated
  on Apple Silicon) and `ghcr.io/rustmailapp/rustmail:latest` pulled, for the
  `mailpit`, `mailhog`, `rustmail-docker` and `rustmail-docker-glibc` tools.

## Ports

Defaults, override with `--smtp-port` / `--http-port`:

| tool | SMTP | HTTP |
|---|---|---|
| rustmail (native) / rustmail-docker / rustmail-docker-glibc | 9025 | 9026 |
| mailpit | 9125 | 9126 |
| mailhog | 9225 | 9226 |

Never 1025/8025 — those are RustMail's own dev-server defaults.

## Usage

```sh
python3 scripts/bench/bench.py --tool rustmail --scenario S1-8c --count 100000 --conns 8 --http --restart
python3 scripts/bench/summarize.py > scripts/bench/results/summary.md
```

`--tool` selects `rustmail`, `mailpit`, `mailhog`, `rustmail-docker` or
`rustmail-docker-glibc`. `--count`, `--conns`, `--kind` (`small` or `medium`)
and `--binary` all have defaults; see `--help` for the full list.

A comparative run across tools, matching the RustMail v0.7.0-era report:

```sh
for t in rustmail mailpit mailhog rustmail-docker rustmail-docker-glibc; do
  python3 scripts/bench/bench.py --tool $t --scenario S1-8c  --count 100000 --conns 8 --http --restart
done
for t in rustmail mailpit rustmail-docker; do
  python3 scripts/bench/bench.py --tool $t --scenario S1-1c  --count 100000 --conns 1
  python3 scripts/bench/bench.py --tool $t --scenario S1-32c --count 100000 --conns 32
  python3 scripts/bench/bench.py --tool $t --scenario S2-8c  --kind medium --count 2000 --conns 8
done
python3 scripts/bench/bench.py --tool mailhog --scenario S2-8c --kind medium --count 2000 --conns 8
python3 scripts/bench/summarize.py > scripts/bench/results/summary.md
```

## What is measured

- Payloads are built before timing (`small` ≈ 2.1 KB multipart/alternative; `medium` ≈ 205 KB with a 150 KB random binary attachment, base64).
- One forked process per SMTP connection, raw sockets, one persistent connection each, `MAIL`/`RCPT`/`DATA` without pipelining.
- **Stored** throughput: the harness polls each tool's API total (`limit=1` list call) every 250 ms until it reaches the sent count; `stored_per_s = stored / (t_count_reached - t_start)`. `accepted_per_s` is the SMTP-side rate.
- CPU and memory sampled every 1 s: native via `ps -o rss,time`; containers via the cgroup's `memory.stat anon` and `cpu.stat usage_usec` (anon ≈ RSS without file-backed pages). Native RSS on macOS includes resident mmapped SQLite pages, so `footprint` is also recorded for rustmail at the restart step.
- `--http`: 200 sequential keep-alive GETs per endpoint after 5 warm-ups: list first page (50), list at offset 90 000, search rare term (10 hits), search common term (every message), get single message (50 ids spread over the mailbox); then `DELETE` all.
- `--restart`: stop and start the same storage (SIGTERM / `docker restart`), time until the API reports the full count, then idle memory after 10 s. MailHog stores in memory, so its restart loses everything.
- Disk: size of `rustmail.db*` (native) or `du -sk /data` inside the container's named volume.

See `docs/benchmarks.md` for a full report from a fixed machine and RustMail revision.
