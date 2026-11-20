import { expect, test } from "@playwright/test";

const APP_BUNDLE = "**/assets/*.js";

test("applies the saved appearance before the app bundle runs", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("rustmail-theme", "dark");
    localStorage.setItem("rustmail-palette", "ember");
    localStorage.setItem("rustmail-rusted", "true");
  });
  await page.route(APP_BUNDLE, (route) => route.abort());

  await page.goto("/");

  const root = page.locator("html");
  await expect(root).toHaveClass(/\bdark\b/);
  await expect(root).toHaveClass(/\brusted\b/);
  await expect(root).toHaveAttribute("data-palette", "ember");
  expect(
    await page.evaluate(
      () => getComputedStyle(document.documentElement).colorScheme,
    ),
  ).toBe("dark");
});

test("follows the system scheme when no theme is saved", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.route(APP_BUNDLE, (route) => route.abort());

  await page.goto("/");

  const root = page.locator("html");
  await expect(root).not.toHaveClass(/\bdark\b/);
  await expect(root).toHaveAttribute("data-palette", "rustmail");
});
