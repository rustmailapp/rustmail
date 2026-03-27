# CLI Flags & Environment Variables

Configuration is resolved in this precedence order: **CLI flags > environment variables > TOML file > defaults**.

## Reference

| CLI Flag | Environment Variable | Default | Description |
|---|---|---|---|
| `--bind` | `RUSTMAIL_BIND` | `127.0.0.1` | IP address to bind SMTP and HTTP listeners to. Docker images default to `0.0.0.0`. Use `0.0.0.0` for remote access outside Docker. |
| `--smtp-port` | `RUSTMAIL_SMTP_PORT` | `1025` | SMTP listener port |
| `--http-port` | `RUSTMAIL_HTTP_PORT` | `8025` | HTTP and WebSocket port |
| `--db-path` | `RUSTMAIL_DB_PATH` | `./rustmail.db` | Path to the SQLite database file |
| `--retention` | `RUSTMAIL_RETENTION` | `0` | Auto-delete messages after N hours. `0` = keep forever. |
| `--max-messages` | `RUSTMAIL_MAX_MESSAGES` | `0` | Maximum messages to retain. Oldest are purged when exceeded. `0` = unlimited. |
| `--max-message-size` | `RUSTMAIL_MAX_MESSAGE_SIZE` | `10485760` | Maximum accepted message size in bytes (default: 10 MB). |
| `--ephemeral` | `RUSTMAIL_EPHEMERAL` | `false` | Use in-memory SQLite. No data is written to disk. |
| `--webhook-url` | `RUSTMAIL_WEBHOOK_URL` | — | HTTP endpoint to POST to on every new message. |
| `--log-level` | `RUSTMAIL_LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error`. |
| `--release-host` | `RUSTMAIL_RELEASE_HOST` | — | Allowed SMTP target for email release in `host:port` format (e.g. `smtp.example.com:587`). Release is disabled unless set. |
| `--config` | — | — | Path to an optional TOML configuration file. |

## Examples

```sh
# Bind to all interfaces on custom ports
rustmail serve --bind 0.0.0.0 --smtp-port 2525 --http-port 9025

# Keep only the last 24 hours of email, max 1000 messages
rustmail serve --retention 24 --max-messages 1000

# Enable webhook notifications
rustmail serve --webhook-url https://hooks.example.com/email

# Allow releasing emails to a specific SMTP server
rustmail serve --release-host smtp.mailgun.org:587
```
