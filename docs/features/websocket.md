# WebSocket

RustMail pushes real-time events over a WebSocket connection. The UI uses this for live inbox updates — you can use the same endpoint for custom integrations.

## Endpoint

```
ws://localhost:8025/api/v1/ws
```

The connection is **one-way push** — the server sends events to the client. Messages sent by the client are ignored.

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
