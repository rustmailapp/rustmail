import { expect, test, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const APP_BUNDLE = "**/assets/*.js";
const PALETTE_KEY = "rustmail-palette";
const INBOX_SIZE = 1;
const RETIRED_PALETTES = [
  ["copper", "tide"],
  ["classic", "graphite"],
  ["ember", "graphite"],
  ["dawn", "rustmail"],
] as const;

async function storePalette(page: Page, id: string): Promise<void> {
  await page.addInitScript(([key, value]) => localStorage.setItem(key, value), [
    PALETTE_KEY,
    id,
  ] as const);
}

test("applies the saved appearance before the app bundle runs", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("rustmail-theme", "dark");
    localStorage.setItem("rustmail-palette", "tide");
    localStorage.setItem("rustmail-rusted", "true");
  });
  await page.route(APP_BUNDLE, (route) => route.abort());

  await page.goto("/");

  const root = page.locator("html");
  await expect(root).toHaveClass(/\bdark\b/);
  await expect(root).toHaveClass(/\brusted\b/);
  await expect(root).toHaveAttribute("data-palette", "tide");
  expect(
    await page.evaluate(
      () => getComputedStyle(document.documentElement).colorScheme,
    ),
  ).toBe("dark");
});

test("follows the system scheme when no theme is saved", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.route(APP_BUNDLE, (route) => route.abort());

  await page.goto("/");

  const root = page.locator("html");
  await expect(root).toHaveClass(/\bdark\b/);
  await expect(root).toHaveAttribute("data-palette", "rustmail");
});

for (const [retired, replacement] of RETIRED_PALETTES) {
  test(`boot script and app both migrate ${retired} to ${replacement}`, async ({
    browser,
  }) => {
    const booted = await browser.newPage();
    await storePalette(booted, retired);
    await booted.route(APP_BUNDLE, (route) => route.abort());
    await booted.goto("/");
    await expect(booted.locator("html")).toHaveAttribute(
      "data-palette",
      replacement,
    );
    await booted.close();

    const loaded = await browser.newPage();
    await storePalette(loaded, retired);
    await mockInbox(loaded, INBOX_SIZE);
    await loaded.goto("/");
    await expect(loaded.locator("html")).toHaveAttribute(
      "data-palette",
      replacement,
    );
    expect(
      await loaded.evaluate((key) => localStorage.getItem(key), PALETTE_KEY),
    ).toBe(replacement);
    await loaded.close();
  });
}
