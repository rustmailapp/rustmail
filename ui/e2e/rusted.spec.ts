import { expect, test, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const INBOX_SIZE = 3;
const STORAGE_KEY = "rustmail-rusted";
const OXIDIZE_PRESSES = 5;

function rootIsRusted(page: Page): Promise<boolean> {
  return page.evaluate(() =>
    document.documentElement.classList.contains("rusted"),
  );
}

function logo(page: Page) {
  return page.getByRole("button", { name: "RustMail", exact: true });
}

function turnOff(page: Page) {
  return page.getByRole("button", { name: "Turn off" });
}

async function openInbox(page: Page): Promise<void> {
  await mockInbox(page, INBOX_SIZE);
  await page.goto("/");
  await expect(logo(page)).toBeVisible();
}

test.describe("rusted easter egg", () => {
  test("toggles from the keyboard on the logo", async ({ page }) => {
    await openInbox(page);

    await logo(page).focus();
    await expect(logo(page)).toBeFocused();
    for (let i = 0; i < OXIDIZE_PRESSES; i++) {
      await page.keyboard.press("Enter");
    }

    await expect(page.getByText("Oxidized")).toBeVisible();
    expect(await rootIsRusted(page)).toBe(true);
    expect(
      await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY),
    ).toBe("true");
  });

  test("stays out of Settings while off", async ({ page }) => {
    await openInbox(page);

    await page.getByTitle("Settings").click();

    await expect(page.getByText("Palette")).toBeVisible();
    await expect(page.getByText("Rusted · On")).toHaveCount(0);
    await expect(turnOff(page)).toHaveCount(0);
  });

  test("turns off from Settings when persisted on", async ({ page }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "true"),
      STORAGE_KEY,
    );
    await openInbox(page);
    expect(await rootIsRusted(page)).toBe(true);

    await page.getByTitle("Settings").click();
    await expect(page.getByText("Rusted · On")).toBeVisible();
    await turnOff(page).click();

    expect(await rootIsRusted(page)).toBe(false);
    await expect(page.getByText("Rusted · On")).toHaveCount(0);
    await expect(turnOff(page)).toHaveCount(0);
    expect(
      await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY),
    ).toBe("false");
  });
});
