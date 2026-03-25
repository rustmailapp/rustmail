# Architecture

RustMail is a Cargo workspace with five crates, each with a single responsibility.

## Crate Overview

| Crate | Responsibility |
|-------|---------------|
| `rustmail-smtp` | TCP listener, ESMTP handshake, emits parsed messages over a tokio broadcast channel |
| `rustmail-storage` | sqlx + SQLite repository, FTS5 index, retention enforcement |
| `rustmail-api` | Axum routes, WebSocket broadcast, bridges HTTP to storage and SMTP channel |
| `rustmail-server` | Binary entry point — parses config, wires all crates, embeds UI assets |
| `rustmail-tui` | Terminal UI client (optional, connects to a running RustMail instance) |

## Data Flow

```
SMTP Client (your app)
    │
    ▼
rustmail-smtp
    │  tokio broadcast channel (ReceivedMessage)
    ▼
rustmail-storage
    │  event broadcast
    ▼
rustmail-api
    ├── REST endpoints  →  HTTP clients / CI pipelines
    └── WebSocket       →  Browser (SolidJS UI)
```

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Custom SMTP over samotop | A mail catcher needs minimal ESMTP — samotop is over-engineered for this use case |
| sqlx over rusqlite | Async-native, compile-time checked queries |
| ULID over UUID/integer | Time-sortable without needing a `created_at` index |
| SolidJS over Leptos/Yew | Contributor-friendly (JS/TS), smaller bundles, better ecosystem |
| rust-embed for UI | Single binary distribution — no separate static file server needed |
| Ephemeral mode via `--ephemeral` | Same sqlx code path, just `sqlite::memory:` connection string |

## Frontend

The UI is a SolidJS + TypeScript + Tailwind CSS v4 application in the `ui/` directory. At build time, Vite produces static assets that are embedded into the Rust binary via `rust-embed`. The server binary serves these at `/` with SPA fallback routing.

Build output: ~28 KB JS + ~13 KB CSS (gzipped).
