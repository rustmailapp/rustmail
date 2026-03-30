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
    volumes:
      - rustmail-data:/data

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
