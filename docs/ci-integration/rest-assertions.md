# REST Assertions

RustMail exposes a purpose-built assertion endpoint for CI pipelines. It returns `200 OK` when conditions are met and `417 Expectation Failed` otherwise — designed for `curl -f`.

## Endpoint

```
GET /api/v1/assert/count
```

## Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `min` | integer | Minimum number of matching messages (inclusive) |
| `max` | integer | Maximum number of matching messages (inclusive) |
| `subject` | string | Filter by subject substring (case-insensitive) |
| `sender` | string | Filter by sender address substring |
| `recipient` | string | Filter by recipient address substring |

## Examples

```sh
# At least 1 email was received
curl -f "localhost:8025/api/v1/assert/count?min=1"

# At least 1 email with "Welcome" in the subject
curl -f "localhost:8025/api/v1/assert/count?min=1&subject=Welcome"

# Exactly 2 emails from notifications@example.com
curl -f "localhost:8025/api/v1/assert/count?min=2&max=2&sender=notifications@example.com"

# At least 1 email sent to admin@example.com
curl -f "localhost:8025/api/v1/assert/count?min=1&recipient=admin@example.com"
```

## Response

```json
// 200 OK — assertion passed
{ "ok": true, "count": 2 }

// 417 Expectation Failed — assertion failed
{ "ok": false, "count": 0, "expected_min": 1, "expected_max": null }
```

## In a CI Script

```sh
#!/bin/bash
set -e

# Start RustMail in the background
rustmail serve --ephemeral &
RUSTMAIL_PID=$!

# Run your test suite
npm test

# Assert emails were sent
curl -f "localhost:8025/api/v1/assert/count?min=1&subject=Welcome"
curl -f "localhost:8025/api/v1/assert/count?min=1&subject=Password%20Reset"

kill $RUSTMAIL_PID
```
