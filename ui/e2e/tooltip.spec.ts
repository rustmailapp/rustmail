import { expect, test, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const MESSAGES = 5;
const NARROW_VIEWPORT = { width: 420, height: 800 };

async function openInbox(page: Page): Promise<void> {
  await mockInbox(page, MESSAGES);
  await page.goto("/");
  await expect(page.locator('[role="option"]').first()).toBeVisible();
}

function clearAll(page: Page) {
  return page.getByRole("button", { name: "Clear all", exact: true }).first();
}

test("shows the clear inbox tooltip on hover with its shortcut", async ({
  page,
}) => {
  await openInbox(page);

  await clearAll(page).hover();

  const tooltip = page.getByRole("tooltip", { name: /Clear inbox/ });
  await expect(tooltip).toBeVisible();
  await expect(tooltip.locator("kbd")).toHaveText("⇧D");
  await expect(clearAll(page)).toHaveAccessibleDescription(/^Clear inbox/);

  await page.mouse.move(0, 0);
  await expect(tooltip).toBeHidden();
});

test("shows a toolbar tooltip on keyboard focus and hides it on Escape", async ({
  page,
}) => {
  await openInbox(page);
  const del = page.getByRole("button", { name: "Delete", exact: true });

  await del.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");

  const tooltip = page.getByRole("tooltip", { name: /Delete/ });
  await expect(tooltip).toBeVisible();
  await expect(tooltip.locator("kbd")).toHaveText("d");

  await page.keyboard.press("Escape");
  await expect(tooltip).toBeHidden();
  await expect(del).toBeFocused();
  await expect(
    page.locator('[role="option"][aria-selected="true"]'),
  ).toHaveCount(1);
});

test("keeps the tooltip inside a narrow viewport at its edge", async ({
  page,
}) => {
  await page.setViewportSize(NARROW_VIEWPORT);
  await openInbox(page);

  await clearAll(page).hover();

  const tooltip = page.getByRole("tooltip", { name: /Clear inbox/ });
  await expect(tooltip).toBeVisible();
  const box = await tooltip.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width);
});

test("clears the inbox from the icon button", async ({ page }) => {
  await openInbox(page);

  await clearAll(page).click();
  await page
    .getByRole("button", { name: "Clear all", exact: true })
    .last()
    .click();

  await expect(page.locator('[role="option"]')).toHaveCount(0);
  await expect(clearAll(page)).toHaveCount(0);
});
