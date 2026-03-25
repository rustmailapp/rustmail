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

The `--release-host` value is an allowlist — only this exact host (and port, if specified) can be used as a release target. This prevents SSRF.

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

## Security Model

Release is locked down to prevent misuse:

1. **Disabled unless configured** — returns `403` if `--release-host` is not set.
2. **Host allowlist** — the `host` in the request body must exactly match the configured `--release-host` host. Mismatches return `403`.
3. **Port allowlist** — only standard SMTP ports are accepted: `25`, `465`, `587`, `2525`. Other ports return `400`.
4. **Port pinning** — if `--release-host` includes a port (e.g., `smtp.example.com:587`), the request must use that exact port. Mismatches return `403`.
5. **TLS required** — connections use `lettre::relay()` with certificate verification. TLS failures return `502`.

## Errors

| Status | Condition |
|--------|-----------|
| `400` | Port not in allowlist, or invalid envelope (unparseable sender/recipients) |
| `403` | Release disabled, host mismatch, or port mismatch |
| `404` | Message not found |
| `502` | TLS setup failure or SMTP delivery failure |
