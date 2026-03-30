# Introduction

RustMail is a local SMTP mail catcher. Point your app's outbound email at it during development or testing, and every message lands in a web UI instead of a real inbox.

It ships as a single binary — the frontend is compiled in, so there's nothing else to install or run.

## Why RustMail?

Most alternatives are either unmaintained (MailHog), minimal (MailCrab), or cloud-only (Mailtrap). RustMail is a self-hosted option that doesn't cut corners:

- **Emails stick around** — SQLite by default, survives restarts. Or `--ephemeral` for throwaway CI runs.
- **Search that works** — FTS5 across subject, body, sender, and recipients.
- **Real-time UI** — WebSocket push, dark mode, keyboard shortcuts. Feels like a proper email client.
- **CI-first** — REST assertion endpoints, a CLI assert mode, and a GitHub Action.
- **Zero runtime deps** — one binary, frontend baked in. Also ships as a ~8 MB Docker image.

## How It Works

```
SMTP Client (your app)
    │
    ▼
rustmail-smtp  ─── tokio TCP listener, ESMTP handshake
    │
    ▼  broadcast channel
rustmail-storage  ─── sqlx + SQLite + FTS5
    │
    ▼  event broadcast
rustmail-api  ─── axum
    ├── REST endpoints  →  HTTP clients / CI pipelines
    └── WebSocket       →  Browser (SolidJS UI)
```

Configure your application to send mail to `localhost:1025`. Open `http://localhost:8025` to see captured emails in real time.
