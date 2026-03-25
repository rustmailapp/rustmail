# Export

Download captured emails in EML or JSON format via the REST API or the UI download button.

## Endpoint

```
GET /api/v1/messages/{id}/export?format=eml|json
```

## Formats

### EML (default)

Returns the raw RFC 822 message — the original bytes as received by the SMTP server.

```sh
curl -O -J "localhost:8025/api/v1/messages/01ARZ3NDEKTSV4RRFFQ69G5FAV/export"
```

Response headers:

```
Content-Type: message/rfc822
Content-Disposition: attachment; filename="01ARZ3NDEKTSV4RRFFQ69G5FAV.eml"
```

### JSON

Returns the parsed message object (same shape as `GET /api/v1/messages/{id}`).

```sh
curl -O -J "localhost:8025/api/v1/messages/01ARZ3NDEKTSV4RRFFQ69G5FAV/export?format=json"
```

Response headers:

```
Content-Type: application/json
Content-Disposition: attachment; filename="01ARZ3NDEKTSV4RRFFQ69G5FAV.json"
```

## Errors

| Status | Condition |
|--------|-----------|
| `400` | Invalid format (must be `eml` or `json`) |
| `404` | Message not found |

## UI

The message detail panel includes a download button that exports the currently viewed email as `.eml`.
