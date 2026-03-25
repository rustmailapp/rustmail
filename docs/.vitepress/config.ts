import { defineConfig } from "vitepress";
import { useSidebar } from "vitepress-openapi";
import { parsedSpec } from "./spec";

const { generateSidebarGroups } = useSidebar({
  spec: parsedSpec,
  linkPrefix: "/api/",
});

export default defineConfig({
  transformPageData(pageData) {
    if (pageData.params?.pageTitle) {
      pageData.title = pageData.params.pageTitle;
    }
  },

  title: "RustMail",
  description:
    "A self-hosted SMTP mail catcher built in Rust — capture, inspect, and test outbound email",
  head: [
    ["link", { rel: "icon", type: "image/png", href: "/favicon.png" }],
    ["meta", { name: "theme-color", content: "#3b82f6" }],
    ["meta", { property: "og:title", content: "RustMail" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Self-hosted SMTP mail catcher with a modern web UI, full-text search, and CI-native assertions",
      },
    ],
    ["meta", { name: "twitter:card", content: "summary" }],
  ],
  cleanUrls: true,
  appearance: "dark",
  lastUpdated: true,

  themeConfig: {
    logo: "/logo.png",
    siteTitle: "RustMail",

    nav: [
      { text: "Guide", link: "/getting-started/introduction" },
      { text: "Configuration", link: "/configuration/cli-flags" },
      { text: "CI Integration", link: "/ci-integration/rest-assertions" },
      { text: "Features", link: "/features/webhooks" },
      { text: "Integrations", link: "/integrations/neovim" },
      { text: "API Reference", link: "/api/" },
      {
        text: "Changelog",
        link: "https://github.com/rustmailapp/rustmail/releases",
      },
    ],

    sidebar: {
      "/getting-started/": [
        {
          text: "Getting Started",
          items: [
            { text: "Introduction", link: "/getting-started/introduction" },
            { text: "Installation", link: "/getting-started/installation" },
            { text: "Quick Start", link: "/getting-started/quick-start" },
            { text: "Docker", link: "/docker" },
          ],
        },
      ],
      "/configuration/": [
        {
          text: "Configuration",
          items: [
            { text: "CLI Flags & Env Vars", link: "/configuration/cli-flags" },
            { text: "TOML Config File", link: "/configuration/toml-config" },
          ],
        },
      ],
      "/ci-integration/": [
        {
          text: "CI Integration",
          items: [
            {
              text: "REST Assertions",
              link: "/ci-integration/rest-assertions",
            },
            { text: "CLI Assert Mode", link: "/ci-integration/cli-assert" },
            { text: "GitHub Action", link: "/ci-integration/github-action" },
          ],
        },
      ],
      "/api/": [
        {
          text: "API Reference",
          items: [{ text: "Overview", link: "/api/" }],
        },
        ...generateSidebarGroups(),
      ],
      "/features/": [
        {
          text: "Features",
          items: [
            { text: "Webhooks", link: "/features/webhooks" },
            { text: "Email Release", link: "/features/release" },
            { text: "Export", link: "/features/export" },
            { text: "WebSocket", link: "/features/websocket" },
            { text: "Keyboard Shortcuts", link: "/features/keyboard-shortcuts" },
            { text: "Email Authentication", link: "/features/email-auth" },
          ],
        },
      ],
      "/integrations/": [
        {
          text: "Integrations",
          items: [
            { text: "Neovim Plugin", link: "/integrations/neovim" },
          ],
        },
      ],
      "/": [
        {
          text: "Other",
          items: [
            { text: "Architecture", link: "/architecture" },
          ],
        },
      ],
    },

    socialLinks: [
      {
        icon: "github",
        link: "https://github.com/rustmailapp/rustmail",
      },
    ],

    search: {
      provider: "local",
    },

    editLink: {
      pattern:
        "https://github.com/rustmailapp/rustmail/edit/master/docs/:path",
      text: "Edit this page on GitHub",
    },

    footer: {
      message: "Released under the MIT / Apache 2.0 License.",
      copyright: "Copyright © 2026 Davide Tacchini",
    },
  },
});
