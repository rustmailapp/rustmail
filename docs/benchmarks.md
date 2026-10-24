---
description: "Runtime benchmark of RustMail against Mailpit and MailHog: ingest rate, HTTP latency and disk use from one reproducible run."
---

# Benchmarks

Runtime benchmark of RustMail against Mailpit and MailHog, from one session
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
  ≤0.03 s everywhere and the accepted and stored rates match.
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
- The S1 8-connection ingest rate is the **median of 3 runs**, interleaved
  across tools. Every other cell is a single run. Run to run, the native
  rate moved by about ±15% and the Docker rates by about ±20%.

## Machine and versions

- Mac16,8, Apple M4 Pro, 12 cores, 24 GiB RAM, macOS 27.0.1, on AC power.
  Loopback only. Load average 3-15 during the runs, with other apps open.
- Docker Desktop 4.96.3 (engine 29.9.2), Linux VM with 12 vCPU and
  11.7 GiB.
- **rustmail (native)**: commit `c4b80da`, built `--release` (fat LTO,
  1 CGU, `panic=abort`).
- **rustmail-docker**: the same commit cross-built for
  `aarch64-unknown-linux-gnu` and run in `debian:bookworm-slim`
  (`--tool rustmail-docker-glibc`). The published image still carries the
  previous release, so it does not measure this code; an earlier run put
  the glibc build within 5% of the musl image.
- **Mailpit** v1.32.0 (`axllent/mailpit`, arm64), run with
  `--max 0 --database /data/mailpit.db`.
- **MailHog** (`mailhog/mailhog:latest`, amd64-only, emulated on Apple
  Silicon). Stores messages in memory.

## Ingest (stored msgs/s)

| tool | S1 1 conn | S1 8 conns | S1 32 conns | S2 8 conns (205 KB) |
|---|---|---|---|---|
| rustmail native | 3 846 | 10 403 | 14 878 | 1 236 |
| rustmail-docker | 884 | 2 853 | 5 235 | 674 |
| Mailpit (docker) | 686 | 2 779 | 2 191 | 551 |
| MailHog (docker, amd64 emu) | – | 475 | – | 42 |

| S1 8 conns, 100k | time to stored (s) | peak mem during ingest (MB) | CPU % avg | disk (MB) | rejected / conn errors |
|---|---|---|---|---|---|
| rustmail native | 9.6 | 54.9 (RSS) | 159 | 110 | 0 / 0 |
| rustmail-docker | 35.1 | 20.9 (anon) | 99 | 109 | 0 / 0 |
| Mailpit | 32.4 | 44.5 (anon) | 146 | 210 | 0 / 0 |
| MailHog | 210.6 | 3 663 (anon) | 125 | memory only | 0 / 0 |

These rows are the first of the three S1 8-connection runs, the one that
also measured HTTP latency and restart; the ingest table above gives the
median.

## Disk

| 100k small messages (S1) | 2 000 messages of 205 KB (S2) |
|---|---|
| rustmail 109-110 MB, Mailpit 210 MB | rustmail 305-316 MB, Mailpit 400 MB |

RustMail keeps each message's source once, compressed with zstd, and parses
the bodies from it when a message is opened. The bench's small mail repeats
its filler text, so it compresses better than typical mail does. The S2
attachments are random bytes and barely compress; the saving there comes
from storing each message once.

## HTTP latency with 100 000 stored (ms, p50 / p95 / p99)

| tool | list first page | list offset 90 000 | search rare (10 hits) | search common (100k hits) | get one | delete all (s) | disk after delete (MB) |
|---|---|---|---|---|---|---|---|
| rustmail native | 0.36 / 0.42 / 0.47 | 0.86 / 0.91 / 0.95 | 0.21 / 0.25 / 0.33 | 2.08 / 2.16 / 2.23 | 0.12 / 0.15 / 0.17 | 0.470 | 0.2 |
| rustmail-docker | 0.84 / 1.28 / 1.76 | 1.46 / 1.76 / 3.28 | 0.44 / 0.74 / 0.85 | 2.58 / 2.80 / 3.28 | 0.31 / 0.41 / 0.52 | 0.558 | 0.2 |
| Mailpit | 8.27 / 9.23 / 10.21 | 15.1 / 16.5 / 16.9 | 205 / 215 / 229 | 498 / 534 / 813 | 0.32 / 0.69 / 1.42 | 0.249 | 0.1 |
| MailHog | 2.47 / 4.69 / 5.56 | 2.36 / 4.77 / 23.6 | 1 271 / 1 401 / 1 576 | 420 / 658 / 759 | 0.38 / 0.45 / 0.83 | 0.001 | – |

Both search terms returned the expected totals on every tool (10 hits for
the rare term, 100 000 for the common one). Docker to Docker, RustMail's
FTS5 search answers 160-2 900x faster than Mailpit and MailHog, and its
first-page list is 3-10x faster. The deep page at offset 90 000 is 10x
faster than Mailpit: listing reads only the message metadata, never the
stored source.
Getting one message parses its bodies from the compressed source, and still
answers in about 0.1 ms natively.

## Restart and idle memory

| tool | idle mem, empty (MB) | restart to full count (s) | idle mem after restart (MB) |
|---|---|---|---|
| rustmail native | 8.6 (RSS) | 0.08 | 19.3 (RSS) |
| rustmail-docker | 8.3 (anon) | 0.30 | 8.1 (anon) |
| Mailpit | 20.8 (anon) | 0.59 | 21.9 (anon) |
| MailHog | 24.5 (anon) | 0.50 | 22.7 (anon), mailbox empty |

## Reading the ingest numbers

- **Through Docker Desktop's port-forward, RustMail and Mailpit both sit
  around 2 800 msg/s at 8 connections**, and RustMail pulls ahead at 32
  (5 235 against 2 191). At those rates the port-forward, not either
  server, sets the ceiling, which is why the runs swing by ±20%.
- **To compare the servers themselves, the client ran inside the same VM.**
  `bench.py` ran in one `python:3.14-slim-bookworm` container against the
  Linux RustMail binary and Mailpit's own binary, both as plain processes,
  so no traffic crossed the port-forward. This is a manual variant of the
  harness, not one of its tools. Over 3 interleaved runs of S1 at 8
  connections the medians were:

  | in one Docker VM, no port-forward | stored msgs/s (3 runs) | median |
  |---|---|---|
  | rustmail | 6 239 / 6 025 / 3 142 | 6 025 |
  | Mailpit | 3 698 / 2 367 / 4 132 | 3 698 |

- Native RustMail beats Mailpit-in-Docker by 3.7x at 8 connections, but
  that comparison is not fair to Mailpit, which pays the Docker Desktop VM
  tax that native RustMail does not.
- At 1 connection the Docker port-forward round trip dominates for both
  containerized tools, so both sit around 700-900 msg/s regardless of
  server-side differences.
- The 32-connection runs use 32 Python load-generator processes on a
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
- Apart from the S1 8-connection ingest medians, each number above is a
  **single run**, not an average across repeated runs.

This is not a claim that RustMail is faster or slower than Mailpit or
MailHog in general, only what this harness measured on this machine, on
this day, for these scenarios.
