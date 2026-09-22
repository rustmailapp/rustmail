# Benchmarks

Runtime benchmark of RustMail against Mailpit and MailHog, from a single run
on one machine. The harness lives in `scripts/bench/` (`bench.py`,
`summarize.py`, `README.md`); see that README to reproduce or extend it.

## Method

- The harness prebuilds every SMTP payload before timing: `small` ≈ 2.1 KB
  multipart/alternative (text + HTML), `medium` ≈ 205 KB with a 150 KB
  random binary attachment in base64.
- Each SMTP connection runs in its own forked process over a raw socket,
  one persistent connection, `MAIL`/`RCPT`/`DATA` without pipelining.
- **Stored** throughput comes from polling each tool's API total every
  250 ms until it reaches the sent count. All three tools answer `250`
  only after the message is stored, so the drain after SMTP finished was
  ≤0.06 s everywhere and the accepted and stored rates match.
- CPU and memory were sampled every 1 s: native via `ps` RSS, containers
  via the cgroup's `memory.stat anon` and `cpu.stat usage_usec`. Native
  macOS RSS includes resident mmapped SQLite pages; container `anon` does
  not.
- HTTP latency: 200 sequential keep-alive GETs per endpoint after 5
  warm-ups, then `DELETE` all. Restart: SIGTERM/start (native) or
  `docker restart -t 30`, timed until the API reports the full count
  again, then idle memory after 10 s.
- Every run used a fresh temp dir or named volume, one tool at a time,
  ports 9025-9226 only.
- Each cell below is a **single run**. A profiled rerun of the native
  ingest case measured roughly ±10-15% noise run to run, more under
  profiling.

## Machine and versions

- Mac16,8, Apple M4 Pro, 12 cores, 24 GiB RAM, macOS 27.0. Loopback only.
- Docker Desktop 29.8.0, Linux VM with 12 vCPU and 11.67 GiB.
- **rustmail (native)**: built `--release` (fat LTO, 1 CGU, `panic=abort`).
- **rustmail-docker**: `ghcr.io/rustmailapp/rustmail:latest`, static musl
  arm64, running the same source as the native build.
- **rustmail-docker-glibc**: the same source cross-built for
  `aarch64-unknown-linux-gnu` and run in `debian:bookworm-slim`, to
  separate the libc/allocator effect from the Docker VM effect.
- **Mailpit** v1.31.2 (`axllent/mailpit`, arm64), run with
  `--max 0 --database /data/mailpit.db`.
- **MailHog** (`mailhog/mailhog:latest`, amd64-only, emulated on Apple
  Silicon). Stores messages in memory.

## Ingest (stored msgs/s)

| tool | S1 1 conn | S1 8 conns | S1 32 conns | S2 8 conns (205 KB) |
|---|---|---|---|---|
| rustmail native | 3 740 | 4 900 | 4 058 | 951 |
| rustmail-docker (musl) | 883 | 2 455 | 2 565 | 512 |
| rustmail-docker-glibc | – | 2 573 | – | – |
| Mailpit (docker) | 807 | 3 171 | 3 027 | 740 |
| MailHog (docker, amd64 emu) | – | 676 | – | 62 |

| S1 8 conns, 100k | time to stored (s) | peak mem during ingest (MB) | CPU % avg | disk (MB) | rejected / conn errors |
|---|---|---|---|---|---|
| rustmail native | 20.4 | 14.7 (RSS) | 128 | 445 | 0 / 0 |
| rustmail-docker | 40.7 | 12.6 (anon) | 117 | 446 | 0 / 0 |
| Mailpit | 31.5 | 45.5 (anon) | 154 | 210 | 0 / 0 |
| MailHog | 147.9 | 3 663 (anon) | 152 | memory only | 0 / 0 |

## HTTP latency with 100 000 stored (ms, p50 / p95 / p99)

| tool | list first page | list offset 90 000 | search rare (10 hits) | search common (100k hits) | get one | delete all (s) | disk after delete (MB) |
|---|---|---|---|---|---|---|---|
| rustmail native | 0.39 / 0.42 / 0.46 | 41.7 / 43.6 / 45.4 | 0.22 / 0.27 / 0.34 | 2.16 / 2.26 / 2.35 | 0.12 / 0.15 / 0.17 | 0.256 | 452 |
| rustmail-docker | 1.17 / 1.42 / 2.08 | 48.8 / 51.3 / 57.2 | 0.55 / 0.71 / 1.02 | 2.62 / 2.97 / 3.25 | 0.38 / 0.48 / 1.04 | 0.413 | 453 |
| Mailpit | 7.56 / 8.20 / 11.05 | 13.4 / 14.3 / 21.7 | 181 / 188 / 195 | 503 / 536 / 556 | 0.33 / 0.48 / 0.55 | 0.090 | 0.1 |
| MailHog | 2.22 / 3.54 / 5.90 | 2.18 / 3.40 / 5.34 | 1 228 / 1 273 / 1 392 | 359 / 414 / 500 | 0.38 / 0.55 / 1.26 | 0.001 | – |

Both search terms returned the expected totals on every tool (10 hits for
the rare term, 100 000 for the common one). RustMail's FTS5 search runs
100-800x faster than Mailpit and MailHog here, and its first-page list is
7-20x faster. The one endpoint where it loses is the deep, offset-based
page: 3x slower than Mailpit and 19x slower than MailHog, because it scans
past the skipped rows instead of using a keyset cursor.

## Reading the ingest numbers

- Native RustMail beats Mailpit-in-Docker by 1.5x at 8 connections, but
  that comparison is not fair to Mailpit, which pays the Docker Desktop
  VM tax that native RustMail does not.
- **Like for like, both inside the Docker VM, RustMail is currently about
  20-30% slower than Mailpit on ingest** (20% on small mail, 30% on
  205 KB mail). The glibc build is only about 5% faster than musl, so the
  allocator is not the main cause; a serial per-message commit path is.
- At 1 connection the Docker port-forward round trip dominates for both
  containerized tools, so both sit around 800-880 msg/s regardless of
  server-side differences.
- The 32-connection runs are slower than the 8-connection runs for every
  tool, because the load generator itself runs 32 Python processes on a
  12-core machine. Treat 32c as a contended-client number, not a server
  ceiling.
- MailHog holds every message in memory and loses them all on restart.

## Caveats

- **Docker on macOS runs in a VM.** Every containerized tool pays
  port-forward latency and VM disk I/O. Only the `rustmail-docker` row
  compares like for like with Mailpit and MailHog; the native row shows
  what a Homebrew/direct-binary user gets, not a Docker-vs-Docker
  comparison.
- MailHog here runs under amd64 emulation and stores messages in memory
  only, so its throughput and CPU numbers are pessimistic and its
  "successful" restart just means an empty mailbox.
- The load generator is Python running on the same machine as the
  server(s) under test; at higher connection counts it competes for CPU
  with the tool being measured.
- Each number above is a **single run**, not an average across repeated
  runs. Expect run-to-run noise on the order of 10-15%.

This is not a claim that RustMail is faster or slower than Mailpit or
MailHog in general, only what this harness measured on this machine, on
this day, for these scenarios.
