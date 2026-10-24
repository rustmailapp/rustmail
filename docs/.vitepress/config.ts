import type { DefaultTheme } from "vitepress";
import { defineConfig } from "vitepress";
import { useSidebar } from "vitepress-openapi";
import { parsedSpec } from "./spec";

const { generateSidebarGroups } = useSidebar({
  spec: parsedSpec,
  linkPrefix: "/api/",
});

const guide: DefaultTheme.SidebarItem[] = [
  {
    text: "Getting Started",
    collapsed: false,
    items: [
      { text: "Introduction", link: "/getting-started/introduction" },
      { text: "Installation", link: "/getting-started/installation" },
      { text: "Quick Start", link: "/getting-started/quick-start" },
      { text: "Docker", link: "/getting-started/docker" },
      { text: "Upgrading", link: "/getting-started/upgrading" },
      { text: "Architecture", link: "/getting-started/architecture" },
    ],
  },
  {
    text: "CI Integration",
    collapsed: false,
    items: [
      { text: "REST Assertions", link: "/ci-integration/rest-assertions" },
      { text: "CLI Assert Mode", link: "/ci-integration/cli-assert" },
      { text: "GitHub Action", link: "/ci-integration/github-action" },
    ],
  },
  {
    text: "Integrations",
    collapsed: false,
    items: [
      { text: "Neovim Plugin", link: "/integrations/neovim" },
    ],
  },
  { text: "Configuration Reference", link: "/configuration/cli-flags" },
];

const openApiItems = generateSidebarGroups().flatMap((g) => g.items ?? []);

const reference: DefaultTheme.SidebarItem[] = [
  {
    text: "Configuration",
    collapsed: false,
    items: [
      { text: "CLI Flags & Env Vars", link: "/configuration/cli-flags" },
      { text: "TOML Config File", link: "/configuration/toml-config" },
    ],
  },
  {
    text: "Features",
    collapsed: false,
    items: [
      { text: "Web UI", link: "/features/web-ui" },
      { text: "HTML Preview", link: "/features/html-preview" },
      { text: "Webhooks", link: "/features/webhooks" },
      { text: "Email Release", link: "/features/release" },
      { text: "Export", link: "/features/export" },
      { text: "WebSocket", link: "/features/websocket" },
      { text: "Keyboard Shortcuts", link: "/features/keyboard-shortcuts" },
      { text: "Email Authentication", link: "/features/email-auth" },
      { text: "Terminal UI", link: "/features/tui" },
    ],
  },
  {
    text: "API Reference",
    collapsed: false,
    items: [
      { text: "Overview", link: "/api/" },
      ...openApiItems,
    ],
  },
  { text: "Benchmarks", link: "/benchmarks" },
  { text: "Getting Started Guide", link: "/getting-started/introduction" },
];

const SITE_URL = "https://docs.rustmail.app";
const OG_IMAGE = "https://rustmail.app/og.png";
const OG_IMAGE_WIDTH = "1200";
const OG_IMAGE_HEIGHT = "630";
const OG_IMAGE_ALT =
  "RustMail web UI: the inbox, an HTML email in desktop preview, and the details rail listing its links";

function pageUrl(relativePath: string): string {
  const path = relativePath.replace(/(^|\/)index\.md$/, "$1").replace(/\.md$/, "");
  return `${SITE_URL}/${path}`;
}

export default defineConfig({
  transformPageData(pageData) {
    if (pageData.params?.pageTitle) {
      pageData.title = pageData.params.pageTitle;
    }
    if (pageData.params?.pageDescription) {
      pageData.description = pageData.params.pageDescription;
    }
  },

  transformHead({ pageData, title, description }) {
    const url = pageUrl(pageData.relativePath);
    return [
      ["link", { rel: "canonical", href: url }],
      ["meta", { property: "og:type", content: "website" }],
      ["meta", { property: "og:site_name", content: "RustMail" }],
      ["meta", { property: "og:url", content: url }],
      ["meta", { property: "og:title", content: title }],
      ["meta", { property: "og:description", content: description }],
      ["meta", { property: "og:image", content: OG_IMAGE }],
      ["meta", { property: "og:image:width", content: OG_IMAGE_WIDTH }],
      ["meta", { property: "og:image:height", content: OG_IMAGE_HEIGHT }],
      ["meta", { property: "og:image:alt", content: OG_IMAGE_ALT }],
      ["meta", { name: "twitter:card", content: "summary_large_image" }],
    ];
  },

  title: "RustMail",
  description:
    "RustMail documentation: install, configure and use the local SMTP mail catcher in dev and CI.",
  head: [
    ["link", { rel: "icon", type: "image/png", href: "/favicon.png" }],
    ["meta", { name: "theme-color", content: "#e6653c" }],
  ],
  sitemap: {
    hostname: SITE_URL,
  },
  cleanUrls: true,
  appearance: "dark",
  lastUpdated: true,

  themeConfig: {
    logo: "/logo.png",
    siteTitle: "RustMail",

    nav: [
      {
        text: "Guide",
        link: "/getting-started/introduction",
        activeMatch:
          "/getting-started/|/ci-integration/|/integrations/",
      },
      {
        text: "Reference",
        link: "/configuration/cli-flags",
        activeMatch: "/configuration/|/features/|/api/|/benchmarks",
      },
      {
        text: "Changelog",
        link: "https://github.com/rustmailapp/rustmail/releases",
      },
    ],

    sidebar: {
      "/getting-started/": guide,
      "/ci-integration/": guide,
      "/integrations/": guide,
      "/configuration/": reference,
      "/features/": reference,
      "/api/": reference,
      "/benchmarks": reference,
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
      copyright: "Copyright © 2026 Smyile",
    },
  },
});
