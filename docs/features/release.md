---
description: "Forward a captured email to a real SMTP server with RustMail release, to re-deliver a test email to an actual mailbox."
---

# Email Release

Forward a captured email to a real SMTP server. This is useful for re-delivering a test email to an actual mailbox.

## Setup

Release is **disabled by default**. Enable it by specifying the allowed target SMTP server:

```sh
rustmail serve --release-host smtp.example.com:587
```

Or via environment variable:

```sh
RUSTMAIL_RELEASE_HOST=smtp.example.com:587 rustmail serve
```

The `--release-host` value is an allowlist: only this exact host (and port, if specified) can be used as a release target. This prevents SSRF.

## Web UI

When release is enabled, the message toolbar shows a **Release** button (a paper plane) next to star, download and delete. Without `--release-host` the button is not shown.

Clicking it opens a confirmation that names the relay (`host:port`) and lists every envelope recipient the message will be sent to, Bcc recipients included: release reuses the captured envelope, not the `To` and `Cc` headers. Enter does not confirm and focus starts on **Cancel**; there is no keyboard shortcut for release.

While a release is in flight the button is disabled. The outcome appears as a notice:

- **Released**: the relay accepted the message.
- **Refused** (`400`, `403`): the server sent nothing.
- **Failed** (`502`): the relay could not be reached, failed the TLS handshake, or refused the message. Nothing was delivered.
- **Outcome unknown**: the relay did not confirm the message (`504`), the browser lost the connection, or the server answered any other error. The relay may already have the message, so check it before releasing the same message again. This notice stays until you dismiss it.

The UI reads the relay from [`GET /api/v1/info`](#server-info) and sends that exact host and port, so a server restarted with a different relay rejects the release instead of sending through a relay you did not confirm.

## Endpoint

```
POST /api/v1/messages/{id}/release
```

Request body:

```json
{
  "host": "smtp.example.com",
  "port": 587
}
```

`port` is optional. It defaults to the port specified in `--release-host`, or `587` if `--release-host` was set without a port (e.g., `--release-host smtp.example.com`).

Success response:

```json
{ "released": true }
```

RustMail first checks the relay with a connection that sends no message (greeting, EHLO, TLS, NOOP, QUIT). If that fails, nothing was sent and the release answers `502`. It then sends the message; an SMTP error reply is also a `502`. If the connection breaks off, or the whole exchange passes 30 seconds, after the message may have been handed over, the server answers `504` with `"code": "release_outcome_unknown"`: check the relay before retrying.

## Server info

`GET /api/v1/info` reports the release relay, or `null` while release is disabled:

```json
{
  "version": "0.8.1",
  "smtp_port": 1025,
  "release": { "host": "smtp.example.com", "port": 587 }
}
```

`port` is the configured port, or `587` when `--release-host` has none.

## Security Model

Release is locked down to prevent misuse:

1. **Disabled unless configured**: returns `403` if `--release-host` is not set.
2. **Host allowlist**: the `host` in the request body must exactly match the configured `--release-host` host. Mismatches return `403`.
3. **Port allowlist**: only standard SMTP ports are accepted: `25`, `465`, `587`, `2525`. Other ports return `400`.
4. **Port pinning**: if `--release-host` includes a port (e.g., `smtp.example.com:587`), the request must use that exact port. Mismatches return `403`.
5. **TLS required**: port `465` uses implicit TLS; ports `25`, `587` and `2525` must upgrade with `STARTTLS`, and the message is not sent if the relay cannot. Certificates are verified. TLS failures return `502`.

## Errors

| Status | Condition |
|--------|-----------|
| `400` | Port not in allowlist, or invalid envelope (unparseable sender/recipients) |
| `403` | Release disabled, host mismatch, or port mismatch |
| `404` | Message not found |
| `502` | Not delivered: TLS setup failure, relay unreachable, or refused by the relay |
| `504` | The relay did not confirm the message; it may have been delivered |
