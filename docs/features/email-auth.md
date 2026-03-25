# Email Authentication Results

RustMail parses email authentication headers and displays them in a dedicated **Auth** tab in the message detail view. No other local mail catcher offers this.

## What It Shows

The Auth tab extracts and displays results from four header types:

| Header | Standard | What It Tells You |
|--------|----------|-------------------|
| `Authentication-Results` | RFC 7601 | Aggregated DKIM, SPF, and DMARC verdicts from the receiving MTA |
| `DKIM-Signature` | RFC 6376 | Signing domain (`d=`), selector (`s=`), and algorithm (`a=`) |
| `Received-SPF` | RFC 7208 | SPF check result (pass, fail, softfail, neutral, etc.) |
| `ARC-Authentication-Results` | RFC 8617 | Authenticated Received Chain results for forwarded messages |

Each result is shown with a color-coded status badge:

| Status | Color |
|--------|-------|
| `pass` | Green |
| `fail` / `hardfail` | Red |
| `softfail` | Amber |
| `neutral` / `temperror` / `permerror` | Orange |
| `none` | Gray |
| `info` (DKIM signature details) | Blue |

## Endpoint

```
GET /api/v1/messages/{id}/auth
```

### Response

```json
{
  "dkim": [
    { "status": "pass", "details": "dkim=pass header.d=example.com header.s=selector1" },
    { "status": "info", "details": "domain=example.com selector=selector1 algorithm=rsa-sha256" }
  ],
  "spf": [
    { "status": "pass", "details": "spf=pass smtp.mailfrom=alice@example.com" }
  ],
  "dmarc": [
    { "status": "pass", "details": "dmarc=pass header.from=example.com" }
  ],
  "arc": []
}
```

### Errors

| Status | Condition |
|--------|-----------|
| `404` | Message not found |

## How It Works

RustMail does **not** perform cryptographic DKIM verification or DNS-based SPF/DMARC checks. It reads and parses the authentication headers that upstream mail servers have already added to the message. This makes it useful for:

- Verifying that your mail infrastructure adds correct authentication headers
- Debugging DKIM signing configuration (domain, selector, algorithm)
- Checking SPF alignment in staging environments
- Inspecting ARC chains on forwarded messages

If no authentication headers are present (common for locally-generated test emails), the Auth tab shows an empty state.
