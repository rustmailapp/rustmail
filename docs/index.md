---
layout: home
title: Documentation
description: "RustMail documentation: install the local SMTP mail catcher, inspect emails in the web UI or terminal, and assert on them in CI."
hero:
  name: RustMail
  text: Documentation
  tagline: Install RustMail, catch email in dev, and assert on it in CI.
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
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/></svg>'
    title: Quick Start
    details: "Start RustMail with Docker, Homebrew or a binary, point your app at localhost:1025 and open the inbox."
    link: /getting-started/quick-start
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/></svg>'
    title: Test emails in CI
    details: "The GitHub Action, rustmail assert and REST assertions that fail the build when an email is wrong."
    link: /ci-integration/github-action
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 16h.01"/><path d="M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><path d="M21.946 12.013H2.054"/><path d="M6 16h.01"/></svg>'
    title: Web UI
    details: "Live inbox, HTML preview at desktop or phone width, auth results, attachments and flagged links."
    link: /features/web-ui
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 12h.01"/><path d="M15 12h.01"/><path d="M10 16c.5.3 1.2.5 2 .5s1.5-.2 2-.5"/><path d="M19.5 10c.5 0 1 .5 1 1v3c0 .5-.5 1-1 1"/><path d="M4.5 10c-.5 0-1 .5-1 1v3c0 .5.5 1 1 1"/><path d="M2 15.5V6.5C2 5.1 3.1 4 4.5 4h15C20.9 4 22 5.1 22 6.5v9c0 1.4-1.1 2.5-2.5 2.5h-15C3.1 18 2 16.9 2 15.5z"/></svg>'
    title: Terminal UI
    details: "rustmail tui and the Neovim plugin: the same inbox and search without leaving the terminal."
    link: /features/tui
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z"/><path d="M12 22V12"/><polyline points="3.29 7 12 12 20.71 7"/><path d="m7.5 4.27 9 5.15"/></svg>'
    title: Configuration
    details: "CLI flags, RUSTMAIL_* environment variables and the TOML file, with their precedence."
    link: /configuration/cli-flags
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/></svg>'
    title: API Reference
    details: "Every REST endpoint for listing, reading, exporting, releasing and deleting captured emails."
    link: /api/
---
