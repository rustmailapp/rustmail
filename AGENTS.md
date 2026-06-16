# AGENTS.md

This document provides guidance for AI coding agents working on the RustMail codebase.

## Repository Overview

RustMail is a self-hosted SMTP mail catcher built in Rust. It captures outbound emails from dev/test environments and displays them in a web UI.

- **Primary Language**: Rust (2024 edition)
- **Frontend**: SolidJS + TypeScript + Tailwind CSS v4
- **Toolchain**: stable (see `rust-toolchain.toml`)

## Repository Structure

```text
.
├── crates/
│   ├── rustmail-smtp/       # SMTP server (tokio TCP + mail-parser)
│   ├── rustmail-storage/    # SQLite storage (sqlx + FTS5)
│   ├── rustmail-api/        # Axum HTTP + WebSocket API
│   ├── rustmail-tui/        # Terminal UI (optional feature)
│   └── rustmail-server/     # Binary entry point (clap CLI, rust-embed)
├── ui/                      # SolidJS frontend (Vite)
├── docker/                  # Dockerfile + docker-compose.yml
├── docs/
│   └── api.yaml             # OpenAPI 3.1 spec
└── Cargo.toml               # Workspace root
```

Data flow: SMTP client → `rustmail-smtp` → broadcast channel → `rustmail-storage` → `rustmail-api` → HTTP/WS clients

## Building

```bash
# Build entire workspace
cargo build --workspace

# Build a specific crate
cargo build -p rustmail-server

# Build the frontend (required before embedding in binary)
cd ui && pnpm install --frozen-lockfile && pnpm build
```

## Testing

```bash
# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p rustmail-smtp
```

## Linting and Formatting

Always run after modifying `.rs` files. Zero warnings policy — CI enforces `-D warnings`.

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

For the frontend:

```bash
cd ui && pnpm exec tsc -b
```

## Coding Conventions

### Rust

- No `unwrap()` in library crates; `unwrap()` only in tests and the binary entrypoint
- Error handling: `thiserror` in library crates, `anyhow` in the binary crate
- Async everywhere — no blocking calls on the tokio runtime
- Crate-level `lib.rs` re-exports the public API
- IDs use ULID (time-sortable, globally unique)

### Code Style

- No comments in the code body; docstrings and public-API docs only. WHY goes in commits/docs.
- No TODOs in committed code
- Descriptive variable/function names over comments
- Small, focused functions

### Frontend

- TypeScript strict mode
- SolidJS reactive primitives (signals, stores)
- Tailwind CSS v4 for styling

### Dependencies

Any new dependency must be justified. Prefer well-maintained crates with minimal transitive dependencies. Workspace dependencies are defined in the root `Cargo.toml`.

## Restricted Actions

- Do not add dependencies without justification
- Do not use `unwrap()` in library crates
- Do not introduce blocking calls in async code
- Do not commit secrets, credentials, or `.env` files
- Do not force push to master

## Default Ports

- SMTP: `1025`
- HTTP/WebSocket: `8025`

## CI/CD

Pull requests trigger:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cd ui && pnpm exec tsc -b && pnpm build`
- `cargo audit`

## Cross-References

- **API Reference**: `docs/api.yaml`
- **Contributing**: `CONTRIBUTING.md`

---

**Canonical Spec**: <https://agents.md>
