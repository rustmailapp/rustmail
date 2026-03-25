# TOML Configuration File

RustMail supports an optional TOML configuration file. Pass it with:

```sh
rustmail serve --config rustmail.toml
```

## Format

```toml
bind = "127.0.0.1"
smtp_port = 1025
http_port = 8025
db_path = "/var/lib/rustmail/rustmail.db"
ephemeral = false
retention = 48
max_messages = 5000
max_message_size = 10485760
log_level = "info"
webhook_url = "https://hooks.example.com/email"
release_host = "smtp.example.com:587"
```

## Precedence

Configuration is resolved in this order (highest wins):

1. **CLI flags** — `--smtp-port 2525`
2. **Environment variables** — `RUSTMAIL_SMTP_PORT=2525`
3. **TOML config file** — `smtp_port = 2525`
4. **Defaults** — `1025`

This means you can set baseline config in a TOML file and override specific values with environment variables or CLI flags per-deployment.
