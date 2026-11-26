import { expect, test, type Locator, type Page } from "@playwright/test";
import { mockInbox, SERVER_INFO, type InboxBackend } from "./inbox-fixture";

const MESSAGES = 5;
const RELAY = { host: "smtp.example.test", port: 465 };
const RELEASE = /\/api\/v1\/messages\/[^/]+\/release$/;
/** Wide enough for the details rail, and its Release button, to show. */
const WIDE_VIEWPORT = { width: 1440, height: 900 };
/** Enough presses to wrap around every dialog here at least once. */
const TAB_PRESSES = 12;
/** The palette chord for the emulated browser; see command-palette.spec.ts. */
const CHORD = "Control+K";

test.use({ viewport: WIDE_VIEWPORT });

async function openInbox(page: Page): Promise<InboxBackend> {
  const backend = await mockInbox(page, MESSAGES);
  await page.route(/\/api\/v1\/info$/, (route) =>
    route.fulfill({ json: { ...SERVER_INFO, release: RELAY } }),
  );
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
  return backend;
}

function hasFocusInside(dialog: Locator): Promise<boolean> {
  return dialog.evaluate((element) => element.contains(document.activeElement));
}

/** Tabs forwards then backwards, checking focus never leaves `dialog`. */
async function expectFocusTrapped(page: Page, dialog: Locator): Promise<void> {
  for (const key of ["Tab", "Shift+Tab"]) {
    for (let press = 0; press < TAB_PRESSES; press++) {
      await page.keyboard.press(key);
      expect(await hasFocusInside(dialog), `${key} #${press + 1}`).toBe(true);
    }
  }
}

/** Opens a dialog with Enter on `opener`, as a keyboard-only user would. */
async function openWithKeyboard(page: Page, opener: Locator): Promise<void> {
  await opener.focus();
  await page.keyboard.press("Enter");
}

test("Settings takes focus, keeps it, and gives it back on Escape", async ({
  page,
}) => {
  await openInbox(page);
  const opener = page.getByRole("button", { name: "Settings", exact: true });
  await openWithKeyboard(page, opener);

  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  expect(await hasFocusInside(dialog)).toBe(true);
  await expectFocusTrapped(page, dialog);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
});

test("Clear all starts on its confirm button and gives focus back on Escape", async ({
  page,
}) => {
  const backend = await openInbox(page);
  const opener = page.getByRole("button", { name: "Clear all", exact: true });
  await openWithKeyboard(page, opener);

  const dialog = page.getByRole("dialog", { name: "Clear all messages" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(
    dialog.getByRole("button", { name: "Clear all", exact: true }),
  ).toBeFocused();
  await expectFocusTrapped(page, dialog);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
  expect(backend.calls.deleted).toEqual([]);
});

test("Clear all confirms with Enter on the focused button", async ({
  page,
}) => {
  const backend = await openInbox(page);
  await openWithKeyboard(
    page,
    page.getByRole("button", { name: "Clear all", exact: true }),
  );

  const dialog = page.getByRole("dialog", { name: "Clear all messages" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Enter");

  await expect(dialog).toBeHidden();
  await expect.poll(() => backend.calls.deleted).toEqual(["/messages"]);
});

test("a held Enter that opens Clear all never confirms it", async ({
  page,
}) => {
  const backend = await openInbox(page);
  await page.getByRole("button", { name: "Clear all", exact: true }).focus();
  await page.keyboard.down("Enter");

  const dialog = page.getByRole("dialog", { name: "Clear all messages" });
  await expect(
    dialog.getByRole("button", { name: "Clear all", exact: true }),
  ).toBeFocused();
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");

  await expect(dialog).toBeVisible();
  expect(backend.calls.deleted).toEqual([]);
});

test("Clear all from the palette gives focus back to the list", async ({
  page,
}) => {
  const backend = await openInbox(page);
  const list = page.getByRole("listbox", { name: "Messages" });
  await list.focus();
  await page.keyboard.press(CHORD);
  await page.keyboard.type("clear inbox");
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog", { name: "Clear all messages" });
  await expect(
    dialog.getByRole("button", { name: "Clear all", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(list).toBeFocused();
  expect(backend.calls.deleted).toEqual([]);
});

test("Release starts on Cancel, keeps focus, and gives it back on Escape", async ({
  page,
}) => {
  await openInbox(page);
  const posted: string[] = [];
  await page.route(RELEASE, (route) => {
    posted.push(route.request().url());
    return route.fulfill({ json: { released: true } });
  });
  const opener = page.getByRole("button", { name: "Release", exact: true });
  await openWithKeyboard(page, opener);

  const dialog = page.getByRole("dialog", { name: "Release this message?" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await expectFocusTrapped(page, dialog);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
  expect(posted).toEqual([]);
});

test("Release never confirms on Enter, wherever focus is", async ({ page }) => {
  await openInbox(page);
  const posted: string[] = [];
  await page.route(RELEASE, (route) => {
    posted.push(route.request().url());
    return route.fulfill({ json: { released: true } });
  });
  await openWithKeyboard(
    page,
    page.getByRole("button", { name: "Release", exact: true }),
  );

  const dialog = page.getByRole("dialog", { name: "Release this message?" });
  await dialog.focus();
  await page.keyboard.press("Enter");
  await expect(dialog).toBeVisible();

  await dialog.getByRole("button", { name: "Cancel" }).focus();
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();
  expect(posted).toEqual([]);
});

test("Keyboard shortcuts keeps focus and gives it back on Escape", async ({
  page,
}) => {
  await openInbox(page);
  const list = page.getByRole("listbox", { name: "Messages" });
  await list.focus();
  await page.keyboard.press(CHORD);
  await page.keyboard.type("shortcuts");
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog.getByRole("button", { name: "Close" })).toBeFocused();
  await expectFocusTrapped(page, dialog);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(list).toBeFocused();
});

test("Command palette keeps focus in its search box", async ({ page }) => {
  await openInbox(page);
  const list = page.getByRole("listbox", { name: "Messages" });
  await list.focus();
  await page.keyboard.press(CHORD);

  const dialog = page.getByRole("dialog", { name: "Command palette" });
  const combobox = dialog.getByRole("combobox", { name: "Command" });
  await expect(combobox).toBeFocused();
  await expectFocusTrapped(page, dialog);
  await expect(combobox).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(list).toBeFocused();
});
