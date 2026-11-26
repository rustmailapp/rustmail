import { expect, test, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const MESSAGES = 3;
/** Wide enough for the details rail, and its tag input, to show. */
const WIDE_VIEWPORT = { width: 1440, height: 900 };

test.use({ viewport: WIDE_VIEWPORT });

async function openInbox(page: Page): Promise<void> {
  await mockInbox(page, MESSAGES, (index) => ({ tags: [`tag-${index}`] }));
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
}

test("Settings names its close button and says which theme is on", async ({
  page,
}) => {
  await openInbox(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });

  const themes = dialog.getByRole("group", { name: "Theme" });
  await themes.getByRole("button", { name: "Dark" }).click();
  await expect(themes.getByRole("button", { name: "Dark" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  for (const other of ["Light", "System"]) {
    await expect(themes.getByRole("button", { name: other })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  }
  await expect(
    dialog
      .getByRole("group", { name: "Palette" })
      .getByRole("button", { pressed: true }),
  ).toHaveCount(1);

  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toBeHidden();
});

test("filter chips say whether they are on", async ({ page }) => {
  await openInbox(page);
  const starred = page.getByRole("button", { name: "Starred", exact: true });

  await expect(starred).toHaveAttribute("aria-pressed", "false");
  await starred.click();
  await expect(starred).toHaveAttribute("aria-pressed", "true");
});

test("the search box and the tag input have names", async ({ page }) => {
  await openInbox(page);

  await expect(
    page.getByRole("textbox", { name: "Search messages" }),
  ).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Add tag" })).toBeVisible();
});

test("the tag menu says it is open, takes focus and gives it back", async ({
  page,
}) => {
  await openInbox(page);
  const trigger = page.getByRole("button", { name: /^Tags/ });
  await expect(trigger).toHaveAttribute("aria-expanded", "false");

  await trigger.focus();
  await page.keyboard.press("Enter");
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const menu = page.getByRole("group", { name: "Tags" });
  const firstTag = menu.getByRole("button", { name: "tag-0" });
  await expect(firstTag).toBeFocused();

  await page.keyboard.press("Enter");
  await expect(firstTag).toHaveAttribute("aria-pressed", "true");

  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await expect(trigger).toBeFocused();
});
