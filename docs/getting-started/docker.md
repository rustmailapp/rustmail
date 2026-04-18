# Docker

RustMail ships a multi-arch Docker image supporting `linux/amd64`, `linux/arm64`, and `linux/arm/v7`.

## Quick Start

```sh
docker run -p 1025:1025 -p 8025:8025 smyile/rustmail:latest
```

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

## Persistence

Mount a volume to `/data` to persist emails across container restarts. The image sets `RUSTMAIL_DB_PATH=/data/rustmail.db` and `RUSTMAIL_BIND=0.0.0.0` by default.

For ephemeral (CI) usage, skip the volume:

```sh
docker run -p 1025:1025 -p 8025:8025 -e RUSTMAIL_EPHEMERAL=true smyile/rustmail:latest
```

## Environment Variables

All [CLI flags](/configuration/cli-flags) have corresponding environment variables prefixed with `RUSTMAIL_`. These work in Docker Compose `environment` blocks, `.env` files, or `docker run -e` flags.

## Optional STARTTLS

RustMail supports explicit SMTP `STARTTLS` on the existing SMTP listener port. To enable it in Docker, mount a certificate and key, then set both `RUSTMAIL_SMTP_TLS_CERT` and `RUSTMAIL_SMTP_TLS_KEY`.

```sh
docker run \
  -p 1025:1025 -p 8025:8025 \
  -v "$PWD/certs:/certs:ro" \
  -e RUSTMAIL_SMTP_TLS_CERT=/certs/smtp-cert.pem \
  -e RUSTMAIL_SMTP_TLS_KEY=/certs/smtp-key.pem \
  smyile/rustmail:latest
```

RustMail advertises `STARTTLS` only when both files are configured. After a client upgrades the connection, it must send `EHLO` again.
