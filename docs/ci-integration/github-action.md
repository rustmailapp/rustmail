# GitHub Action

RustMail provides a GitHub Action for email assertion in CI pipelines.

## Quick Start

```yaml
- name: Start RustMail
  uses: rustmailapp/rustmail-action@v1

- name: Run tests (your app sends emails to localhost:1025)
  run: npm test

- name: Assert emails were sent
  uses: rustmailapp/rustmail-action@v1
  with:
    mode: assert
    assert-count: 1
    assert-subject: "Welcome"
```

The action has two modes:

1. **`start`** (default) — Downloads the RustMail binary, starts an ephemeral SMTP server in the background, and waits until it's ready.
2. **`assert`** — Checks captured emails against your filters using the REST assertion API.

## Inputs

### Start Mode

| Input | Default | Description |
|-------|---------|-------------|
| `mode` | `start` | Set to `start` (or omit — it's the default) |
| `smtp-port` | `1025` | SMTP port to listen on |
| `http-port` | `8025` | HTTP/API port |
| `version` | `latest` | RustMail version (e.g., `v0.1.0`) |

### Assert Mode

| Input | Default | Description |
|-------|---------|-------------|
| `mode` | — | Set to `assert` |
| `assert-count` | `1` | Minimum number of matching emails |
| `assert-subject` | — | Filter by subject substring |
| `assert-sender` | — | Filter by sender address |
| `assert-recipient` | — | Filter by recipient address |

## Outputs

| Output | Description |
|--------|-------------|
| `http-port` | HTTP port the server is listening on |
| `smtp-port` | SMTP port the server is listening on |

## Full Workflow Example

```yaml
name: E2E Tests

on: [push]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Start RustMail
        uses: rustmailapp/rustmail-action@v1
        with:
          smtp-port: 1025
          http-port: 8025

      - name: Run tests
        run: npm test
        env:
          SMTP_HOST: 127.0.0.1
          SMTP_PORT: 1025

      - name: Assert welcome email
        uses: rustmailapp/rustmail-action@v1
        with:
          mode: assert
          assert-count: 1
          assert-subject: "Welcome"

      - name: Assert password reset email
        uses: rustmailapp/rustmail-action@v1
        with:
          mode: assert
          assert-count: 1
          assert-subject: "Password Reset"
```

## Multiple Assertions

Chain as many assert steps as you need. Each one checks independently against the same running server:

```yaml
- uses: rustmailapp/rustmail-action@v1
  with:
    mode: assert
    assert-count: 1
    assert-subject: "Order Confirmation"

- uses: rustmailapp/rustmail-action@v1
  with:
    mode: assert
    assert-sender: "noreply@example.com"
    assert-count: 3
```

## Pinning a Version

By default the action downloads the latest RustMail release. Pin to a specific version for reproducibility:

```yaml
- uses: rustmailapp/rustmail-action@v1
  with:
    version: v0.2.1  # pin to a release tag
```

Check [GitHub Releases](https://github.com/rustmailapp/rustmail/releases) for available versions.

::: tip
For non-GitHub CI systems, use the [CLI Assert Mode](/ci-integration/cli-assert) or [REST Assertions](/ci-integration/rest-assertions) directly.
:::
