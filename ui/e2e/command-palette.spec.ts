import { expect, test, type Locator, type Page } from "@playwright/test";
import { mockInbox, optionSelector, SERVER_INFO } from "./inbox-fixture";

const MESSAGES = 50;
const SEARCH_INPUT = 'input[placeholder="Search emails..."]';
const RELAY = { host: "smtp.example.test", port: 465 };
const RELEASE = /\/api\/v1\/messages\/[^/]+\/release$/;
/**
 * The palette chord for the emulated browser, whatever the host runs.
 *
 * The "Desktop Chrome" device reports a Windows user agent, so the app takes
 * Control+K even on a Mac host, where `ControlOrMeta` would send Command.
 */
const CHORD = "Control+K";

async function openInbox(page: Page) {
  const backend = await mockInbox(page, MESSAGES);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
  return backend;
}

function palette(page: Page): Locator {
  return page.getByRole("dialog", { name: "Command palette" });
}

function combobox(page: Page): Locator {
  return palette(page).getByRole("combobox", { name: "Command" });
}

function activeOption(page: Page): Locator {
  return palette(page).locator('[role="option"][aria-selected="true"]');
}

async function openPalette(page: Page): Promise<void> {
  await page.keyboard.press(CHORD);
  await expect(combobox(page)).toBeFocused();
}

test("opens with the chord and closes with Escape, giving focus back", async ({
  page,
}) => {
  await openInbox(page);
  const list = page.getByRole("listbox", { name: "Messages" });
  await list.focus();

  await openPalette(page);
  await expect(palette(page)).toBeVisible();
  await expect(combobox(page)).toHaveAttribute(
    "aria-activedescendant",
    (await activeOption(page).getAttribute("id")) ?? "",
  );

  await page.keyboard.press("Escape");
  await expect(palette(page)).toBeHidden();
  await expect(list).toBeFocused();
});

test("opens from the search box and closes on a click outside", async ({
  page,
}) => {
  await openInbox(page);
  const search = page.locator(SEARCH_INPUT);
  await search.focus();

  await openPalette(page);
  await page.mouse.click(5, 5);

  await expect(palette(page)).toBeHidden();
  await expect(search).toBeFocused();
});

test("toggles closed with the chord again", async ({ page }) => {
  await openInbox(page);
  await openPalette(page);

  await page.keyboard.press(CHORD);

  await expect(palette(page)).toBeHidden();
});

test("keeps the page shortcuts quiet while it is open", async ({ page }) => {
  const backend = await openInbox(page);
  const selected = page.locator(`${optionSelector(0)}[aria-selected="true"]`);
  await expect(selected).toBeVisible();

  await openPalette(page);
  await page.keyboard.type("dj");

  await expect(combobox(page)).toHaveValue("dj");
  await expect(selected).toBeAttached();
  expect(backend.calls.deleted).toEqual([]);
});

test("keeps Tab inside the palette", async ({ page }) => {
  await openInbox(page);
  await openPalette(page);

  await page.keyboard.press("Tab");
  await expect(combobox(page)).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(combobox(page)).toBeFocused();
});

test("moves the active command with the arrow keys and wraps", async ({
  page,
}) => {
  await openInbox(page);
  await openPalette(page);
  const options = palette(page).getByRole("option");
  const first = await options.first().textContent();
  const second = await options.nth(1).textContent();

  await expect(activeOption(page)).toHaveText(first ?? "");
  await page.keyboard.press("ArrowDown");
  await expect(activeOption(page)).toHaveText(second ?? "");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await expect(activeOption(page)).toHaveText(
    (await options.last().textContent()) ?? "",
  );
  await expect(combobox(page)).toHaveAttribute(
    "aria-activedescendant",
    (await options.last().getAttribute("id")) ?? "",
  );
});

test("runs the command chosen with the arrow keys", async ({ page }) => {
  await openInbox(page);
  await openPalette(page);
  await page.keyboard.type("theme");
  const options = palette(page).getByRole("option");
  await expect(options.nth(0)).toContainText("Theme: ");
  await expect(options.nth(1)).toContainText("Theme: ");
  const second = (await options.nth(1).textContent()) ?? "";

  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");

  await expect(palette(page)).toBeHidden();
  expect(
    await page.evaluate(() => localStorage.getItem("rustmail-theme")),
  ).toBe(second.replace("Theme: ", "").toLowerCase());
});

test("jumps to a message found by subject", async ({ page }) => {
  await openInbox(page);
  await openPalette(page);

  await page.keyboard.type("message 42");
  await expect(activeOption(page)).toContainText("Message 42");
  await expect(activeOption(page)).toContainText("sender-42@example.test");
  await page.keyboard.press("Enter");

  await expect(palette(page)).toBeHidden();
  await expect(page.locator(optionSelector(42))).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(
    page.getByRole("heading", { name: "Message 42", level: 2 }),
  ).toBeVisible();
});

test("runs a filter", async ({ page }) => {
  const backend = await mockInbox(page, MESSAGES, (index) => ({
    is_starred: index % 10 === 0,
  }));
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();

  await openPalette(page);
  await page.keyboard.type("filter by starred");
  await expect(activeOption(page)).toHaveText("Filter by starred");
  await page.keyboard.press("Enter");

  await expect
    .poll(() => backend.calls.listed.at(-1))
    .toContain("starred=true");
  await expect(
    page.getByRole("listbox", { name: "Messages" }).getByRole("option"),
  ).toHaveCount(MESSAGES / 10);

  await openPalette(page);
  await page.keyboard.type("clear filters");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => backend.calls.listed.at(-1))
    .not.toContain("starred=true");
});

test("asks before clearing the inbox, even under a held Enter", async ({
  page,
}) => {
  const backend = await openInbox(page);
  await openPalette(page);

  await page.keyboard.type("clear inbox");
  await expect(activeOption(page)).toContainText("Clear inbox");
  await expect(activeOption(page).locator("kbd")).toHaveText("⇧D");
  await page.keyboard.press("Enter");

  const confirmDialog = page.getByRole("dialog", {
    name: "Clear all messages",
  });
  await expect(confirmDialog).toBeVisible();
  await page.evaluate(() =>
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        repeat: true,
        bubbles: true,
      }),
    ),
  );
  await expect(confirmDialog).toBeVisible();
  await confirmDialog.getByRole("button", { name: "Cancel" }).click();
  await expect(confirmDialog).toBeHidden();
  expect(backend.calls.deleted).toEqual([]);
});

test("opens the release confirmation and never sends on its own", async ({
  page,
}) => {
  await mockInbox(page, MESSAGES);
  await page.route(/\/api\/v1\/info$/, (route) =>
    route.fulfill({ json: { ...SERVER_INFO, release: RELAY } }),
  );
  const posted: string[] = [];
  await page.route(RELEASE, (route) => {
    posted.push(route.request().url());
    return route.fulfill({ json: { released: true } });
  });
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();

  await openPalette(page);
  await page.keyboard.type("release");
  await expect(activeOption(page)).toHaveText("Release message");
  await page.keyboard.press("Enter");

  const confirmDialog = page.getByRole("dialog", {
    name: "Release this message?",
  });
  await expect(confirmDialog).toBeVisible();
  await expect(
    confirmDialog.getByRole("button", { name: "Cancel" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(confirmDialog).toBeHidden();
  expect(posted).toEqual([]);
});

test("hides release while the server has no relay", async ({ page }) => {
  await openInbox(page);
  await openPalette(page);

  const options = palette(page).getByRole("option");
  await expect(options.filter({ hasText: "Delete message" })).toHaveCount(1);
  await expect(options.filter({ hasText: "Release message" })).toHaveCount(0);
});

test("deletes the selected message with undo", async ({ page }) => {
  const backend = await openInbox(page);
  await openPalette(page);

  await page.keyboard.type("delete message");
  await expect(activeOption(page).locator("kbd")).toHaveText("d");
  await page.keyboard.press("Enter");

  await expect(page.locator(optionSelector(0))).toHaveCount(0);
  await expect(page.locator(optionSelector(1))).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await openPalette(page);
  await page.keyboard.type("undo");
  await page.keyboard.press("Enter");
  await expect(page.locator(optionSelector(0))).toBeAttached();
  expect(backend.calls.deleted).toEqual([]);
});

test("lists the keyboard shortcuts and gives focus back on close", async ({
  page,
}) => {
  await openInbox(page);
  const list = page.getByRole("listbox", { name: "Messages" });
  await list.focus();
  await openPalette(page);

  await page.keyboard.type("shortcuts");
  await page.keyboard.press("Enter");

  const shortcuts = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(shortcuts).toBeVisible();
  await expect(shortcuts.getByRole("button", { name: "Close" })).toBeFocused();
  await expect(shortcuts).toContainText("Open the command palette");
  await page.keyboard.press("Escape");
  await expect(shortcuts).toBeHidden();
  await expect(list).toBeFocused();
});
