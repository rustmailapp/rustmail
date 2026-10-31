---
description: "Install RustMail with Docker, Homebrew or a pre-built binary for Linux and macOS."
---

# Installation

## Docker

```sh
docker run -p 1025:1025 -p 8025:8025 smyile/rustmail:latest
```

See the [Docker guide](/getting-started/docker) for Compose, persistence, and configuration.

## Homebrew

```sh
brew install rustmailapp/rustmail/rustmail
```

If you run RustMail as a service (`brew services start rustmail`), upgrading to 0.8 or 0.10 migrates the database unattended on the next restart. See [Upgrading](/getting-started/upgrading).

## Pre-built Binaries

Download from [GitHub Releases](https://github.com/rustmailapp/rustmail/releases/latest): Linux x86_64/aarch64/armv7 (glibc and musl builds), macOS (Intel + Apple Silicon).

## From Source

```sh
git clone https://github.com/rustmailapp/rustmail
cd rustmail
make build
./target/release/rustmail
```

## Default Ports

| Service | Port |
|---------|------|
| SMTP | `1025` |
| HTTP / WebSocket | `8025` |
