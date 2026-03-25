# Webhooks

RustMail can POST a JSON notification to an HTTP endpoint whenever a new email is received.

## Setup

```sh
rustmail serve --webhook-url https://hooks.example.com/email
```

Or via environment variable:

```sh
RUSTMAIL_WEBHOOK_URL=https://hooks.example.com/email rustmail serve
```

Or in `rustmail.toml`:

```toml
webhook_url = "https://hooks.example.com/email"
```

## Payload

Each webhook is an HTTP POST with `Content-Type: application/json`. The body is a message summary:

```json
{
  "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
  "sender": "user@example.com",
  "recipients": "[\"recipient@example.com\"]",
  "subject": "Hello World",
  "size": 1024,
  "has_attachments": false,
  "is_read": false,
  "is_starred": false,
  "tags": [],
  "created_at": "2026-03-23T10:30:45.123Z"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | ULID — use this to fetch the full message via the REST API |
| `sender` | string | Envelope sender address |
| `recipients` | string | JSON-encoded array of recipient addresses |
| `subject` | string \| null | Parsed subject line |
| `size` | integer | Raw message size in bytes |
| `has_attachments` | boolean | Whether the message has attachments |
| `is_read` | boolean | Always `false` for new messages |
| `is_starred` | boolean | Always `false` for new messages |
| `tags` | array | Always `[]` for new messages |
| `created_at` | string | ISO 8601 UTC timestamp |

## Behavior

- **Fire-and-forget** — the webhook is dispatched in a background task and does not block message processing.
- **5-second timeout** — if the endpoint doesn't respond within 5 seconds, the request is abandoned.
- **No retries** — failed deliveries are logged as warnings and not retried.
- **No queue** — webhooks are sent in real time as messages arrive. If the endpoint is down, those notifications are lost.

## Example: Slack Notification

Pair with a lightweight relay that transforms the payload into a Slack message:

```sh
rustmail serve --webhook-url http://localhost:3000/relay
```

The relay receives the JSON payload above and can POST a formatted message to Slack's Incoming Webhooks API.
