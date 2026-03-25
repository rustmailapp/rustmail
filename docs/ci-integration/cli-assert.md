# CLI Assert Mode

The `rustmail assert` subcommand starts an ephemeral SMTP server, waits for emails matching your criteria, and exits with code `0` (pass) or `1` (timeout/fail). No daemon required.

## Usage

```sh
rustmail assert [OPTIONS]
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--timeout` | `30s` | Maximum time to wait for matching emails |
| `--min-count` | `1` | Minimum number of matching emails required |
| `--subject` | — | Filter by subject substring |
| `--sender` | — | Filter by sender address substring |
| `--recipient` | — | Filter by recipient address substring |
| `--smtp-port` | `1025` | SMTP port to listen on |

## Examples

```sh
# Wait up to 30s for at least 1 email
rustmail assert

# Wait for 2 "Password Reset" emails within 60 seconds
rustmail assert --timeout=60s --min-count=2 --subject="Password Reset"

# Assert on sender
rustmail assert --sender=notifications@example.com --min-count=1
```

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Assertion passed — required emails were received |
| `1` | Assertion failed — timeout or criteria not met |

## How It Works

1. Starts an ephemeral SMTP server (in-memory, no database file)
2. Listens for incoming emails
3. Filters against your criteria (subject, sender, recipient)
4. Exits `0` as soon as `--min-count` matching emails arrive
5. Exits `1` if `--timeout` is reached first
