import { defineConfig } from "vitest/config";

/**
 * Vitest setup for the store tests.
 *
 * Store logic is plain reactive TypeScript with no JSX and no DOM, so it runs
 * in `node`. The conditions matter: node resolution would otherwise pick
 * solid-js's SSR build, whose reactive primitives are non-tracking stubs, and
 * a frozen memo makes every "does nothing" assertion pass for the wrong
 * reason. It is `ssr.resolve.conditions` rather than the top-level key because
 * a `node` environment runs through Vite's SSR pipeline.
 */
export default defineConfig({
  ssr: {
    resolve: {
      conditions: ["development", "browser"],
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
