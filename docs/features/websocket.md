# WebSocket

RustMail pushes real-time events over a WebSocket connection. The UI uses this for live inbox updates. You can use the same endpoint for custom integrations.

## Endpoint

```
ws://localhost:8025/api/v1/ws
```

The connection is **one-way push**: the server sends events to the client. Messages sent by the client are ignored.

## Events

All events are JSON objects with a `type` field and an optional `data` field:

```json
{ "type": "<event-type>", "data": <payload> }
```

### `message:new`

Fired when a new email is received and stored.

```json
{
  "type": "message:new",
  "data": {
    "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    "sender": "user@example.com",
    "recipients": "[\"recipient@example.com\"]",
    "subject": "Hello World",
    "size": 1024,
    "has_attachments": false,
    "is_read": false,
    "created_at": "2026-03-23T10:30:45.123Z"
  }
}
```

### `message:delete`

Fired when a single message is deleted.

```json
{
  "type": "message:delete",
  "data": { "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV" }
}
```

### `message:read`

Fired when a message's read state changes.

```json
{
  "type": "message:read",
  "data": { "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "is_read": true }
}
```

### `message:starred`

Fired when a message's starred state changes.

```json
{
  "type": "message:starred",
  "data": { "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "is_starred": true }
}
```

### `message:tags`

Fired when a message's tags are updated.

```json
{
  "type": "message:tags",
  "data": { "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "tags": ["important", "review"] }
}
```

### `messages:clear`

Fired when all messages are deleted. This event has no `data` field.

```json
{ "type": "messages:clear" }
```

## Origin Check

The WebSocket handshake is not covered by CORS, so RustMail checks it itself. A handshake that carries an `Origin` header is answered `403 Forbidden` unless the origin is either:

- the one RustMail is reached at — the address in the browser's own `Host` header, which is what the bundled UI sends; or
- one passed to `--allowed-origin` (repeatable, or comma-separated in `RUSTMAIL_ALLOWED_ORIGINS`).

Without this, any page open in the same browser could subscribe to the event stream and read sender, recipients and subject of every incoming email.

The comparison is against the address the browser dialled, so it does not by itself stop DNS rebinding — a page whose own hostname is re-pointed at the machine running RustMail keeps a matching origin. RustMail is a development tool and does not defend against that.

Clients that send **no** `Origin` header at all — the TUI, `websocat`, CI scripts, anything that is not a browser — are unaffected. Browsers do not let a page omit or forge the header, so its absence is only ever a non-browser client.

Behind a reverse proxy, name the public origin explicitly unless the proxy forwards the browser's `Host`:

```sh
rustmail serve --allowed-origin https://mail.example.com
```

## Connection Limits

A maximum of **50 concurrent WebSocket connections** is enforced. New connections beyond this limit receive `503 Service Unavailable`.

## Reconnection

The built-in UI reconnects automatically with exponential backoff (2s initial, 30s cap), resetting on successful connection. If you're building a custom client, implement similar retry logic.

## Example: Node.js Client

```js
const ws = new WebSocket("ws://localhost:8025/api/v1/ws");

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);

  switch (msg.type) {
    case "message:new":
      console.log(`New email from ${msg.data.sender}: ${msg.data.subject}`);
      break;
    case "message:delete":
      console.log(`Message ${msg.data.id} deleted`);
      break;
    case "messages:clear":
      console.log("All messages cleared");
      break;
  }
};
```
