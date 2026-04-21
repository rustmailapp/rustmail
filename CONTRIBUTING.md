# Contributing to RustMail

Thank you for your interest in contributing to RustMail! This document provides guidelines and instructions for contributing.

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) 22+ with [pnpm](https://pnpm.io/)
- SQLite 3

### Development Setup

```bash
git clone https://github.com/rustmailapp/rustmail.git
cd rustmail

# Install UI dependencies
cd ui && pnpm install && cd ..

# Build everything
make build

# Or run in development mode (auto-reload)
make dev
```

### Useful Commands

| Command | Description |
|---------|-------------|
| `make dev` | Run backend + frontend in dev mode |
| `make build` | Production build (UI + Rust) |
| `make test` | Run all Rust tests |
| `make lint` | Clippy + TypeScript type check |
| `make fmt` | Format Rust + TypeScript code |
| `make check` | Cargo check + TypeScript check |
| `make clean` | Remove build artifacts |

## Reporting Bugs

Before filing a bug report, please check [existing issues](https://github.com/rustmailapp/rustmail/issues). Then use the [Bug Report](https://github.com/rustmailapp/rustmail/issues/new?template=bug_report.yml) template — it will guide you through the required information.

## Submitting Changes

### Branch Naming

- `feat/short-description` — New features
- `fix/short-description` — Bug fixes
- `refactor/short-description` — Code restructuring
- `docs/short-description` — Documentation changes

### Code Style

**Rust:**
- Run `cargo fmt` before committing
- Zero clippy warnings: `cargo clippy --all-targets -- -D warnings`
- No `unwrap()` in library crates (`crates/rustmail-smtp`, `crates/rustmail-storage`, `crates/rustmail-api`)
- `unwrap()` is acceptable in tests and `rustmail-server` (the binary)
- Error handling: `thiserror` in libraries, `anyhow` in the binary
- Async everywhere — no blocking calls on the tokio runtime

**TypeScript (UI):**
- Run `pnpm exec prettier --write .` before committing
- Type checking must pass: `pnpm exec tsc -b`

### Commit Messages

Write atomic commits with descriptive messages:

```
feat: add webhook retry with exponential backoff

fix: prevent FTS5 injection via unsanitized search input

refactor: extract SMTP session handling into dedicated module

docs: add reverse proxy deployment guide
```

### Pull Request Process

1. Fork the repository and create your branch from `master`
2. Make your changes with tests where applicable
3. Ensure all checks pass: `make lint && make test`
4. Open a pull request with a clear description of the change
5. Link any related issues

### What Makes a Good PR

- **One logical change** per pull request
- Tests for new functionality
- No unrelated formatting or refactoring changes
- Clear description of *why* the change is needed

### Adding a User-Facing Feature

When a PR introduces a new user-visible feature (CLI flag, API endpoint, UI capability), tick each item that applies:

- [ ] Row added to the features table in `README.md`
- [ ] Feature page under `docs/features/` (or relevant subsection)
- [ ] `docs/api.yaml` updated if the feature exposes an HTTP endpoint
- [ ] `docs/configuration/cli-flags.md` updated if a CLI flag changed
- [ ] Feature card added to `rustmailapp/rustmail-www` (homepage grid) if it's a headline capability
- [ ] Commit message uses the `feat:` Conventional Commit prefix so it shows up in the release changelog

## Architecture

See the [Architecture](https://docs.rustmail.app/architecture) page for crate layout, data flow, and design decisions.

## Dependencies

Adding a new dependency requires justification. Prefer:
- Well-maintained crates (recent activity, >100 stars or part of a known ecosystem)
- Crates with minimal transitive dependencies

## License

By contributing to RustMail, you agree that your contributions will be licensed under the [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) license, at the user's choice.
