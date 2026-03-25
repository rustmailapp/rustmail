---
layout: home
hero:
  name: RustMail
  text: SMTP Mail Catcher
  tagline: A local mail catcher for dev and CI. Single binary, nothing to install.
  image:
    src: /logo.png
    alt: RustMail
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/introduction
    - theme: alt
      text: API Reference
      link: /api/
    - theme: alt
      text: GitHub
      link: https://github.com/rustmailapp/rustmail

features:
  - icon: 💾
    title: Emails survive restarts
    details: SQLite-backed by default. Use --ephemeral when you just need a throwaway inbox for CI.
  - icon: 🔍
    title: Find anything
    details: Full-text search (FTS5) across subject, body, sender, and recipients.
  - icon: ⚡
    title: Real-time inbox
    details: WebSocket push — new mail shows up the moment it arrives. Dark mode included.
  - icon: 🧪
    title: Built for CI
    details: REST assertion endpoints, a CLI assert mode, and a GitHub Action for your test pipelines.
  - icon: 📦
    title: One binary, no deps
    details: The frontend is embedded at compile time. Drop it on a server and run it.
  - icon: 🔑
    title: REST API + OpenAPI
    details: Full API with an OpenAPI 3.1 spec. Export emails, release to real SMTP, get webhook notifications.
---
