---
description: "RustMail REST API reference: list, read, export, release and delete captured emails, run CI assertions and stream events over WebSocket."
aside: false
outline: false
title: API Reference
---

# API Reference

Base path: `/api/v1`. All responses are JSON. All IDs are [ULIDs](https://github.com/ulid/spec) (time-sortable, globally unique).

The full OpenAPI 3.1 spec is also served at runtime at `GET /api/v1/openapi.yaml`.

`GET /api/v1/info` reports the running server's version and SMTP port, which is handy for checking which build a pipeline is talking to.

<OASpec />
