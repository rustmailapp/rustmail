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

## Arch Linux (AUR)

```sh
yay -S rustmail-bin
```

Or with any AUR helper (`paru -S rustmail-bin`, `pacman -S rustmail-bin` via chaotic-aur, etc.).

## Pre-built Binaries

Download from [GitHub Releases](https://github.com/rustmailapp/rustmail/releases/latest) — Linux (x86_64, aarch64, armv7 — glibc + musl), macOS (Intel + Apple Silicon).

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
