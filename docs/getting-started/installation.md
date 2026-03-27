# Installation

## Docker

```sh
docker run -p 1025:1025 -p 8025:8025 ghcr.io/rustmailapp/rustmail:latest
```

See the [Docker guide](/docker) for Compose, persistence, and configuration.

## Homebrew

```sh
brew install rustmailapp/rustmail/rustmail
```

## Arch Linux (AUR)

```sh
yay -S rustmail-bin
```

Or with any AUR helper (`paru -S rustmail-bin`, `pacman -S rustmail-bin` via chaotic-aur, etc.).

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
