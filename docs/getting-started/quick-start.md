# Quick Start

## Start the Server

### Docker

```sh
docker run -p 1025:1025 -p 8025:8025 -e RUSTMAIL_BIND=0.0.0.0 ghcr.io/rustmailapp/rustmail:latest
```

### Binary

```sh
# Default — SMTP on 1025, UI on 8025, SQLite at ./rustmail.db
rustmail

# Ephemeral mode for CI — in-memory, nothing written to disk
rustmail serve --ephemeral --smtp-port 1025 --http-port 8025
```

## Open the UI

Navigate to `http://localhost:8025` in your browser.

## Send a Test Email

Configure your application to send mail to `localhost:1025`. To verify the SMTP receiver is working:

### Using swaks (recommended)

```sh
swaks --to test@example.com --server localhost --port 1025
```

### Using nc

```sh
printf "EHLO test\r\nMAIL FROM:<sender@example.com>\r\nRCPT TO:<test@example.com>\r\nDATA\r\nSubject: Hello\r\n\r\nTest body\r\n.\r\nQUIT\r\n" | nc localhost 1025
```

The email appears in the UI in real time via WebSocket push — no page refresh needed.

## What's Next?

- [Configure](/configuration/cli-flags) ports, retention, and other options
- [Set up CI assertions](/ci-integration/rest-assertions) in your test pipeline
- [Browse the API](/api/) to integrate programmatically
