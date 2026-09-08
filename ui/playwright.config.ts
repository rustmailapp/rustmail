import { defineConfig, devices } from "@playwright/test";

const HOST = "127.0.0.1";
const PORT = Number(process.env.E2E_PORT ?? 4319);
const BASE_URL = `http://${HOST}:${PORT}`;
const SERVER_TIMEOUT_MS = 180_000;
/** Above the specs' own stall deadline, so their diagnostics surface first. */
const TEST_TIMEOUT_MS = 60_000;

/**
 * Browser-level tests for the inbox.
 *
 * These serve the built assets rather than the dev server, whose proxy would
 * point `/api` at a local rustmail; the suite must never depend on one being
 * up. `reuseExistingServer` stays off because adopting a server that already
 * holds the port makes the whole suite assert against a different application
 * while still reporting green. The host is pinned because vite's default
 * `localhost` binds only `::1` on some machines, which never answers the IPv4
 * readiness probe.
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: TEST_TIMEOUT_MS,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: { baseURL: BASE_URL, trace: "on-first-retry" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `pnpm build && pnpm exec vite preview --host ${HOST} --port ${PORT} --strictPort`,
    url: BASE_URL,
    reuseExistingServer: false,
    timeout: SERVER_TIMEOUT_MS,
  },
});
