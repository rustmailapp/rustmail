# Production Readiness Checklist

Tracks gaps between what RustMail claims and what is verified. Work through tier by tier.

Last audited: 2026-04-21

---

## Tier 1 — Correctness (COMPLETE)

Claims that are false or unverified. Fix before any public release.

### False Claims

- [x] **STARTTLS**: originally marked "Yes" in RUSTMAIL.md with no implementation, then downgraded to "Planned" (2026-03-24). **Shipped in v0.3.0** via PR #22 (2026-04-18): full RFC 3207 upgrade with state reset, both-or-neither config validation, 6 integration tests. (2026-04-21)
- [x] **CSV export**: Phase 4 in RUSTMAIL.md §11 listed CSV — never implemented. **Fixed**: removed CSV from §8 table and Phase 4 list. (2026-03-24)

### Untested Endpoints

- [x] **Auth endpoint** (`GET /api/v1/messages/:id/auth`): 3 tests added — `auth_results_parsed_correctly` (all 4 header types), `auth_results_empty_for_plain_email`, `auth_results_not_found`. (2026-03-24)
- [x] **Inline images** (`GET /api/v1/messages/:id/inline/:cid`): 2 tests added — `inline_image_by_content_id` (verifies content-type, CSP, nosniff, body), `inline_image_not_found`. (2026-03-24)
- [x] **Webhook notifications**: Integration test `webhook_fires_on_new_message` — axum mock server verifies payload shape (id, sender, subject, is_read, is_starred, tags, created_at). No new deps needed. (2026-03-24)
- [x] **CLI assert subcommand**: 3 tests — `cli_assert_passes_when_email_arrives` (spawns binary, sends email, asserts exit 0), `cli_assert_fails_on_timeout` (no email, asserts exit 1), `cli_assert_filters_by_subject` (sends decoy + match, asserts filter works). Uses `env!("CARGO_BIN_EXE_rustmail")` + `portpicker`. (2026-03-24)

---

## Tier 2 — Hardening

Things that work but lack automated verification.

### Backend

- [x] **SMTP oversized message rejection**: `smtp_rejects_oversized_message` — sets 256-byte limit, sends 512-byte body, asserts 552 rejection. Also verifies EHLO SIZE reflects the configured limit. (2026-03-24)
- [x] **Config precedence**: 2 tests — `config_env_overrides_toml` (writes TOML with port A, sets env to port B, asserts server binds to B) and `config_toml_used_when_no_env` (TOML only, asserts server uses TOML port). (2026-03-24)
- [x] **Retention background task**: extracted `run_retention_tick` from the scheduler loop (PR #23, 2026-04-18) with `now` injection for deterministic tests. 5 tests cover no-op gating, purge, preserve, trim, and combined-policy emission. (2026-04-21)
- [ ] **Embedded UI serving**: `rust-embed` integration + SPA fallback routing. Requires a built UI (`make build`) to test meaningfully — the test binary doesn't embed assets.
- [x] **SMTP concurrent session limit**: `smtp_session_limit_rejects_excess` — holds 100 TCP connections open via `SmtpServer::run()`, verifies 101st is silently dropped (no banner, connection closed or times out). (2026-03-24)
- [x] **WebSocket connection limit**: `ws_connection_limit_returns_503` — opens 50 WebSocket connections via `tokio-tungstenite`, verifies 51st fails to upgrade. (2026-03-24)

### Frontend

- [ ] **Zero frontend tests**: No automated tests for UI features. Recommend Vitest for store/logic and Playwright for critical E2E flows (inbox load, search, star/tag, keyboard nav). Independent effort from backend hardening.

---

## Tier 3 — Spec & Doc Hygiene (COMPLETE)

All doc-vs-code inconsistencies have been fixed.

### RUSTMAIL.md

- [x] §2 comparison table: STARTTLS "Yes" → "Planned" (2026-03-24)
- [x] §2 comparison table: Message tagging "Planned" → "Yes" (2026-03-24)
- [x] §8 feature table: "Message tagging and starring" "Planned" → "Done" (2026-03-24)
- [x] §8 feature table: Export row — removed CSV, now "Done" (2026-03-24)
- [x] §11 Phase 4: "Export: EML, JSON, CSV" → "Export: EML, JSON" (2026-03-24)
- [x] §5 workspace layout: added `rustmail-tui` crate to tree diagram (2026-03-24)

### docs/features/websocket.md

- [x] Added `message:starred` event documentation (2026-03-24)
- [x] Added `message:tags` event documentation (2026-03-24)

### docs/features/webhooks.md

- [x] Added `is_starred` and `tags` fields to payload example and field table (2026-03-24)

### docs/ci-integration/rest-assertions.md

- [x] Response example: `"passed": true` → `"ok": true` to match actual handler code (2026-03-24)

### docs/architecture.md

- [x] "four crates" → "five crates", added `rustmail-tui` row (2026-03-24)

### README.md

- [x] Comparison table now includes STARTTLS row (README.md:89). (2026-04-21)

### OpenAPI spec (docs/api.yaml)

- [x] `/messages/:id/inline/:cid` endpoint — already documented (lines 214–241). (2026-03-24)
- [x] PATCH `/messages/:id` — added `is_starred` and `tags` to `MessageUpdate` schema. (2026-03-24)
- [x] Added `is_starred` and `tags` to `MessageSummary` schema. (2026-03-24)
- [x] Added `message:starred` and `message:tags` to WebSocket event table. (2026-03-24)
- [x] `AssertResult` schema — renamed `passed` → `ok` to match actual handler code. (2026-03-24)

---

## Tier 4 — Future Features (acknowledged, not blocking)

These are in the spec as "Planned" and don't need action now. Listed for completeness.

- HTML email preview with client compatibility hints
- Spam score analysis (SpamAssassin rules subset)
- Multi-instance sync (SQLite replication)
- Plugin system (WASM-based)
- CSV export format
