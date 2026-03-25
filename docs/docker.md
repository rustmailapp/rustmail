# Docker

RustMail ships a multi-stage Docker image supporting `linux/amd64` and `linux/arm64`.

## Quick Start

```sh
docker run -p 1025:1025 -p 8025:8025 -e RUSTMAIL_BIND=0.0.0.0 ghcr.io/rustmailapp/rustmail:latest
```

## Docker Compose

```yaml
services:
  rustmail:
    image: ghcr.io/rustmailapp/rustmail:latest
    ports:
      - "1025:1025"
      - "8025:8025"
    environment:
      RUSTMAIL_BIND: "0.0.0.0"
      RUSTMAIL_DB_PATH: "/data/rustmail.db"
    volumes:
      - rustmail-data:/data

volumes:
  rustmail-data:
```

::: warning Bind Address
When running in Docker, you must set `RUSTMAIL_BIND=0.0.0.0` (or `--bind 0.0.0.0`) so the server listens on all interfaces inside the container. The default `127.0.0.1` only accepts connections from within the container itself.
:::

## Persistence

Mount a volume to `/data` and set `RUSTMAIL_DB_PATH=/data/rustmail.db` to persist emails across container restarts.

For ephemeral (CI) usage, skip the volume:

```sh
docker run -p 1025:1025 -p 8025:8025 \
  -e RUSTMAIL_BIND=0.0.0.0 \
  -e RUSTMAIL_EPHEMERAL=true \
  ghcr.io/rustmailapp/rustmail:latest
```

## Environment Variables

All [CLI flags](/configuration/cli-flags) have corresponding environment variables prefixed with `RUSTMAIL_`. These work in Docker Compose `environment` blocks, `.env` files, or `docker run -e` flags.
