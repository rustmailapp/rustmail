import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

/** Mirrors the policy the server sends in `crates/rustmail-api/src/lib.rs`, so E2E runs under it. */
const CONTENT_SECURITY_POLICY =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: http: https:; connect-src 'self' ws: wss:; frame-src 'self'";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  server: {
    port: 3000,
    proxy: {
      "/api": "http://localhost:8025",
      "/api/v1/ws": {
        target: "ws://localhost:8025",
        ws: true,
      },
    },
  },
  preview: {
    headers: { "Content-Security-Policy": CONTENT_SECURITY_POLICY },
  },
});
