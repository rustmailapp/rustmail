<p align="center">
  <img src="https://raw.githubusercontent.com/rustmailapp/rustmail/master/docs/public/logo.png" alt="RustMail" width="96" />
</p>

# RustMail

A local SMTP mail catcher for development and CI, built in Rust. Your app sends email to RustMail instead of a real server, and every message lands in a web UI where you can inspect it, search it and assert on it in tests.

**Website:** [rustmail.app](https://rustmail.app) · **Docs:** [docs.rustmail.app](https://docs.rustmail.app) · **GitHub:** [rustmailapp/rustmail](https://github.com/rustmailapp/rustmail)

![RustMail web UI: the inbox, an HTML email in desktop preview, and the details rail listing its links](https://rustmail.app/og.png)

## Quick Start

```sh
docker run -p 1025:1025 -p 8025:8025 smyile/rustmail:latest
```

Point your app's SMTP at `localhost:1025`, then open [localhost:8025](http://localhost:8025). Emails show up in real time.

Coming from MailHog or Mailpit? RustMail uses the same default ports, SMTP `1025` and HTTP `8025`, so your app's SMTP settings stay as they are.

## Docker Compose

```yaml
services:
  rustmail:
    image: smyile/rustmail:latest
    ports:
      - "1025:1025"
      - "8025:8025"
    security_opt:
      - no-new-privileges:true
    volumes:
      - rustmail-data:/data
    restart: unless-stopped

volumes:
  rustmail-data:
```

## Supported Architectures

| Architecture | Tag |
|---|---|
| `linux/amd64` | `latest` |
| `linux/arm64` | `latest` |
| `linux/arm/v7` | `latest` |

Multi-arch manifest: Docker automatically pulls the correct image for your platform.

## Persistence

The image stores emails at `/data/rustmail.db` by default. Mount a volume to `/data` to persist emails across container restarts.

For ephemeral (CI) usage, skip the volume:

```sh
docker run -p 1025:1025 -p 8025:8025 -e RUSTMAIL_EPHEMERAL=true smyile/rustmail:latest
```

## Environment Variables

All configuration is done via `RUSTMAIL_*` environment variables:

| Variable | Default | Description |
|---|---|---|
| `RUSTMAIL_BIND` | `0.0.0.0` | IP address to bind listeners to |
| `RUSTMAIL_SMTP_PORT` | `1025` | SMTP listener port |
| `RUSTMAIL_HTTP_PORT` | `8025` | HTTP and WebSocket port |
| `RUSTMAIL_DB_PATH` | `/data/rustmail.db` | Path to SQLite database file |
| `RUSTMAIL_RETENTION` | `0` | Auto-delete messages after N hours (`0` = keep forever) |
| `RUSTMAIL_MAX_MESSAGES` | `0` | Max messages to retain (`0` = unlimited) |
| `RUSTMAIL_MAX_MESSAGE_SIZE` | `10485760` | Max accepted message size in bytes (10 MB) |
| `RUSTMAIL_SMTP_TLS_CERT` | unset | Path to a PEM certificate for optional SMTP STARTTLS |
| `RUSTMAIL_SMTP_TLS_KEY` | unset | Path to a PEM private key for optional SMTP STARTTLS |
| `RUSTMAIL_EPHEMERAL` | `false` | Use in-memory SQLite (no data written to disk) |
| `RUSTMAIL_WEBHOOK_URL` | unset | URL to POST on every new message |
| `RUSTMAIL_LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `RUSTMAIL_RELEASE_HOST` | unset | Allowed SMTP target for email release (`host:port`) |
| `RUSTMAIL_ALLOWED_HOSTS` | unset | Comma-separated host names browsers may reach RustMail on, such as `mail.example.com`. IP addresses and `localhost` are always answered |
| `RUSTMAIL_ALLOWED_ORIGINS` | unset | Comma-separated extra origins allowed to open the WebSocket, as `scheme://host[:port]` |
| `RUSTMAIL_WS_BUFFER` | `4096` | WebSocket events buffered for each client |

See the [configuration reference](https://docs.rustmail.app/configuration/cli-flags) for every option.

## Ports

| Port | Protocol | Description |
|---|---|---|
| `1025` | TCP | SMTP server |
| `8025` | TCP | HTTP API, WebSocket, and Web UI |

## Optional STARTTLS

STARTTLS uses the normal SMTP port via explicit upgrade. Both `RUSTMAIL_SMTP_TLS_CERT` and `RUSTMAIL_SMTP_TLS_KEY` are required; setting only one fails startup, and `STARTTLS` is advertised only when both are set.

Mount your TLS files read-only and set both environment variables:

```sh
docker run \
  -p 1025:1025 -p 8025:8025 \
  -v "$PWD/certs:/certs:ro" \
  -e RUSTMAIL_SMTP_TLS_CERT=/certs/smtp-cert.pem \
  -e RUSTMAIL_SMTP_TLS_KEY=/certs/smtp-key.pem \
  smyile/rustmail:latest
```

Clients must issue `EHLO`, then `STARTTLS`, and then `EHLO` again after the TLS handshake completes.

## Test emails in CI

Run the image as a service container, or use the [RustMail GitHub Action](https://github.com/marketplace/actions/rustmail-action), then fail the build when an email is missing:

```sh
curl -f "localhost:8025/api/v1/assert/count?min=1&subject=Welcome"
```

The endpoint answers `200` when the condition holds and `417` otherwise. See [CI integration](https://docs.rustmail.app/ci-integration/rest-assertions).

## Features

- **Persistent storage**: SQLite-backed, emails survive restarts
- **Full-text search**: FTS5 across subject, body, sender, and recipients
- **Real-time updates**: WebSocket pushes new emails to the UI instantly
- **Modern UI**: dark-mode-first, looks and feels like a real email client
- **DKIM/SPF/DMARC/ARC display**: parses authentication headers with color-coded badges
- **REST assertion endpoints**: `GET /api/v1/assert/count?min=1&subject=Welcome`
- **Webhook notifications**: fire-and-forget POST on new email
- **Email release**: forward captured emails to a real SMTP server
- **Export**: download as EML or JSON
- **Retention policies**: auto-purge by age or count

## Benchmarks

RustMail and Mailpit, both in Docker, with 100,000 messages stored. Medians of 6 runs; latencies are p50.

| | RustMail | Mailpit |
|---|---|---|
| Search, rare term | **0.66 ms** | 203 ms |
| Inbox list, first page | **0.94 ms** | 8.3 ms |
| Disk, 100,000 messages | **110 MB** | 210 MB |
| Idle memory | **9.4 MB** | 19.6 MB |

Method, MailHog and every other number: [docs.rustmail.app/benchmarks](https://docs.rustmail.app/benchmarks).

## Image Details

- **Base image:** `alpine:3.21`
- **Runs as:** non-root user `rustmail`
- **Healthcheck:** built-in (HTTP check every 30s)
- **Volume:** `/data`
- **Security:** `no-new-privileges` recommended

## License

Licensed under either of [MIT](https://github.com/rustmailapp/rustmail/blob/master/LICENSE-MIT) or [Apache 2.0](https://github.com/rustmailapp/rustmail/blob/master/LICENSE-APACHE), at your option.
