# RustMail

A fast, feature-rich SMTP mail catcher built in Rust. Single binary. Persistent storage. Modern UI. CI-ready.

**GitHub:** [rustmailapp/rustmail](https://github.com/rustmailapp/rustmail) · **Docs:** [docs.rustmail.app](https://docs.rustmail.app)

## Quick Start

```sh
docker run -p 1025:1025 -p 8025:8025 smyile/rustmail:latest
```

Point your app's SMTP at `localhost:1025`, then open [localhost:8025](http://localhost:8025). Emails show up in real time.

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

Multi-arch manifest — Docker automatically pulls the correct image for your platform.

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
| `RUSTMAIL_SMTP_TLS_CERT` | — | Path to a PEM certificate for optional SMTP STARTTLS |
| `RUSTMAIL_SMTP_TLS_KEY` | — | Path to a PEM private key for optional SMTP STARTTLS |
| `RUSTMAIL_EPHEMERAL` | `false` | Use in-memory SQLite (no data written to disk) |
| `RUSTMAIL_WEBHOOK_URL` | — | URL to POST on every new message |
| `RUSTMAIL_LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `RUSTMAIL_RELEASE_HOST` | — | Allowed SMTP target for email release (`host:port`) |

## Ports

| Port | Protocol | Description |
|---|---|---|
| `1025` | TCP | SMTP server |
| `8025` | TCP | HTTP API, WebSocket, and Web UI |

`STARTTLS` uses the normal SMTP port and is advertised only when both `RUSTMAIL_SMTP_TLS_CERT` and `RUSTMAIL_SMTP_TLS_KEY` are set.

## Optional STARTTLS

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

## Features

- **Persistent storage** — SQLite-backed, emails survive restarts
- **Full-text search** — FTS5 across subject, body, sender, and recipients
- **Real-time updates** — WebSocket pushes new emails to the UI instantly
- **Modern UI** — dark-mode-first, looks and feels like a real email client
- **DKIM/SPF/DMARC/ARC display** — parses authentication headers with color-coded badges
- **REST assertion endpoints** — `GET /api/v1/assert/count?min=1&subject=Welcome`
- **Webhook notifications** — fire-and-forget POST on new email
- **Email release** — forward captured emails to a real SMTP server
- **Export** — download as EML or JSON
- **Retention policies** — auto-purge by age or count

## Image Details

- **Base image:** `alpine:3.21`
- **Runs as:** non-root user `rustmail`
- **Healthcheck:** built-in (HTTP check every 30s)
- **Volume:** `/data`
- **Security:** `no-new-privileges` recommended

## License

Licensed under either of [MIT](https://github.com/rustmailapp/rustmail/blob/master/LICENSE-MIT) or [Apache 2.0](https://github.com/rustmailapp/rustmail/blob/master/LICENSE-APACHE), at your option.
