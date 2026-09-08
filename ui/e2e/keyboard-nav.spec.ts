import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  messageId,
  mockInbox,
  optionSelector,
  TOTAL_MESSAGES,
} from "./inbox-fixture";

/**
 * Rows past this point were never in the DOM at load: the initial page holds
 * 100 messages and the virtualizer mounts roughly 20 of them.
 */
const DEEP_TARGET_POSITION = 420;
const SELECTION_TIMEOUT_MS = 30_000;
const SEARCH_INPUT = 'input[placeholder="Search emails..."]';
const MAX_TAB_STOPS = 12;
/**
 * Roughly the OS key-repeat interval, which is the cadence this feature gets
 * used at when someone holds the arrow key down.
 */
const KEY_REPEAT_MS = 35;
/** A walk should settle into one load, with headroom for a slow runner. */
const MAX_SETTLED_FETCHES = 4;
/**
 * Long enough to close a deletion's undo window, short enough to stay inside
 * the request deadline behind it, which would otherwise abort the pane's reads
 * while the clock is being wound forward.
 */
const PAST_UNDO_WINDOW_MS = 6_000;

function list(page: Page): Locator {
  return page.getByRole("listbox", { name: "Messages" });
}

function selectedOption(page: Page): Locator {
  return page.locator('[role="option"][aria-selected="true"]');
}

async function position(page: Page): Promise<number> {
  return Number(await selectedOption(page).getAttribute("aria-posinset"));
}

function activeDescription(page: Page): Promise<string> {
  return page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return "none";
    return `${el.tagName.toLowerCase()}[${el.getAttribute("role") ?? ""}]`;
  });
}

function activeRole(page: Page): Promise<string> {
  return page.evaluate(
    () => document.activeElement?.getAttribute("role") ?? "none",
  );
}

async function openInbox(page: Page, total?: number) {
  const backend = await mockInbox(page, total);
  await page.goto("/");
  await expect(list(page)).toBeVisible();
  await expect(selectedOption(page)).toHaveCount(1);
  return backend;
}

/**
 * Opens the inbox with time stopped, so a test drives the clock itself.
 *
 * Deletion waits out an undo window before it writes anything, and a suite
 * that waited for that in real seconds would spend most of its run asleep.
 */
async function openInboxAtRest(page: Page, total?: number) {
  const now = new Date("2026-09-08T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now);
  return openInbox(page, total);
}

function row(page: Page, id: string | null): Locator {
  return page.locator(`[role="option"][data-id="${id}"]`);
}

/** Tabs forward until the list has focus, recording every role passed through. */
async function tabToList(page: Page): Promise<string[]> {
  const seen: string[] = [];
  await page.locator(SEARCH_INPUT).focus();
  for (let i = 0; i < MAX_TAB_STOPS; i++) {
    await page.keyboard.press("Tab");
    const role = await activeRole(page);
    seen.push(role);
    if (role === "listbox") break;
  }
  return seen;
}

/**
 * Presses `key` until the selection reaches `target`.
 *
 * A press is a no-op while the page holding the next row is still in flight,
 * so progress is driven by the observed position rather than by press count.
 * Waiting for the position to merely stop changing would settle early, on a
 * page boundary, while the next request is still open.
 */
async function pressUntil(
  page: Page,
  key: string,
  target: number,
): Promise<void> {
  const deadline = Date.now() + SELECTION_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if ((await position(page)) >= target) return;
    await page.keyboard.press(key);
  }
  throw new Error(
    `selection stalled at ${await position(page)} before reaching ${target}`,
  );
}

function arrowDownTo(page: Page, target: number): Promise<void> {
  return pressUntil(page, "ArrowDown", target);
}

test.describe("inbox keyboard navigation", () => {
  test("puts the list in the tab order and keeps rows out of it", async ({
    page,
  }) => {
    await openInbox(page);

    const seen = await tabToList(page);

    expect(seen).toContain("listbox");
    expect(seen).not.toContain("option");
    await expect(page.locator('[role="option"][tabindex]')).toHaveCount(0);
  });

  test("costs a single tab stop to pass the whole list", async ({ page }) => {
    await openInbox(page);
    await tabToList(page);

    const inside: string[] = [];
    for (let i = 0; i < MAX_TAB_STOPS; i++) {
      await page.keyboard.press("Tab");
      const withinList = await page.evaluate(() =>
        Boolean(
          document.activeElement?.closest('[role="listbox"]') ??
          document.activeElement?.matches('[role="listbox"]'),
        ),
      );
      if (!withinList) break;
      inside.push(await activeDescription(page));
    }

    expect(inside).toEqual([]);
  });

  test("keeps the list focused across a clear and refill", async ({ page }) => {
    const backend = await openInbox(page);
    await tabToList(page);
    await arrowDownTo(page, 4);

    await backend.push({ type: "messages:clear" });
    await expect(page.locator('[role="option"]')).toHaveCount(0);

    await backend.push({
      type: "message:new",
      data: {
        id: "msg-fresh",
        sender: "fresh@example.test",
        recipients: ["inbox@example.test"],
        subject: "Arrived after the clear",
        size: 512,
        has_attachments: false,
        is_read: false,
        is_starred: false,
        tags: [],
        created_at: "2026-02-01T00:00:00Z",
      },
    });
    await expect(page.locator('[role="option"]')).toHaveCount(1);

    expect(await activeRole(page)).toBe("listbox");
    await page.keyboard.press("ArrowDown");
    expect(await position(page)).toBe(1);
  });

  test("never points activedescendant at a row that is gone", async ({
    page,
  }) => {
    await openInbox(page);
    await tabToList(page);
    await expect(page.locator(optionSelector(0))).toHaveCount(1);

    await page.mouse.move(200, 400);
    await page.mouse.wheel(0, 6000);
    await expect.poll(() => selectedOption(page).count()).toBe(0);

    expect(await list(page).getAttribute("aria-activedescendant")).toBeNull();
    expect(await activeRole(page)).toBe("listbox");

    await page.keyboard.press("ArrowDown");
    await expect(selectedOption(page)).toBeInViewport();
    expect(await list(page).getAttribute("aria-activedescendant")).toBe(
      await selectedOption(page).getAttribute("id"),
    );
  });

  test("reaches a row that was never rendered, keeping focus on the list", async ({
    page,
  }) => {
    await openInbox(page);
    await tabToList(page);
    await expect(page.locator(optionSelector(0))).toHaveCount(1);

    await arrowDownTo(page, DEEP_TARGET_POSITION);

    expect(await position(page)).toBe(DEEP_TARGET_POSITION);
    await expect(page.locator(optionSelector(0))).toHaveCount(0);
    expect(await activeRole(page)).toBe("listbox");
    await expect(selectedOption(page)).toBeInViewport();
  });

  test("keeps activedescendant on the one selected row", async ({ page }) => {
    await openInbox(page);
    await tabToList(page);
    await arrowDownTo(page, 30);

    await expect(selectedOption(page)).toHaveCount(1);
    expect(await list(page).getAttribute("aria-activedescendant")).toBe(
      await selectedOption(page).getAttribute("id"),
    );
  });

  test("reports the whole inbox size, not the loaded page", async ({
    page,
  }) => {
    await openInbox(page);

    expect(
      Number(await selectedOption(page).getAttribute("aria-setsize")),
    ).toBe(TOTAL_MESSAGES);
    expect(await page.locator('[role="option"]').count()).toBeLessThan(
      TOTAL_MESSAGES,
    );
  });

  test("Home and End reach both ends of a single-page inbox", async ({
    page,
  }) => {
    const total = 40;
    await openInbox(page, total);
    await tabToList(page);
    await arrowDownTo(page, 12);

    await page.keyboard.press("End");
    expect(await position(page)).toBe(total);
    await expect(selectedOption(page)).toBeInViewport();

    await page.keyboard.press("Home");
    expect(await position(page)).toBe(1);
    await expect(selectedOption(page)).toBeInViewport();
  });

  test("End walks through every page to the true last message", async ({
    page,
  }) => {
    await openInbox(page);
    await tabToList(page);

    await pressUntil(page, "End", TOTAL_MESSAGES);

    expect(await position(page)).toBe(TOTAL_MESSAGES);
    await expect(selectedOption(page)).toBeInViewport();
  });

  test("holds still at both ends of the list", async ({ page }) => {
    await openInbox(page);
    await tabToList(page);

    await page.keyboard.press("ArrowUp");
    expect(await position(page)).toBe(1);

    await pressUntil(page, "End", TOTAL_MESSAGES);
    await page.keyboard.press("ArrowDown");
    expect(await position(page)).toBe(TOTAL_MESSAGES);
  });

  test("hands focus to the list when a row is clicked", async ({ page }) => {
    await openInbox(page);

    await page.locator('[role="option"]').nth(3).click();
    expect(await activeRole(page)).toBe("listbox");
    const clicked = await position(page);

    await page.keyboard.press("ArrowDown");

    expect(await position(page)).toBe(clicked + 1);
  });

  test("ignores the arrow keys when focus is outside the list", async ({
    page,
  }) => {
    await openInbox(page);
    await tabToList(page);
    await arrowDownTo(page, 5);
    const before = await position(page);

    await page.getByTitle("Settings").focus();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");

    expect(await activeRole(page)).not.toBe("listbox");
    expect(await position(page)).toBe(before);
  });

  test("loads every page while walking to the end", async ({ page }) => {
    await openInbox(page);
    await tabToList(page);

    await arrowDownTo(page, DEEP_TARGET_POSITION);

    expect(
      Number(await selectedOption(page).getAttribute("aria-setsize")),
    ).toBeLessThanOrEqual(TOTAL_MESSAGES);
    expect(await position(page)).toBe(DEEP_TARGET_POSITION);
  });
});

test.describe("unread filter", () => {
  test("walks forward instead of snapping back to the top", async ({
    page,
  }) => {
    await openInbox(page);
    await page.getByRole("button", { name: "Unread" }).click();
    await tabToList(page);
    const steps = 5;
    const before = Number(
      await selectedOption(page).getAttribute("aria-setsize"),
    );

    for (let i = 1; i <= steps; i++) {
      await page.keyboard.press("ArrowDown");
      await expect
        .poll(() => selectedOption(page).getAttribute("data-id"))
        .toBe(messageId(i));
    }

    const readBehind = steps - 1;
    await expect(page.locator(optionSelector(1))).toHaveCount(0);
    await expect(page.locator(optionSelector(steps))).toHaveCount(1);
    expect(
      Number(await selectedOption(page).getAttribute("aria-setsize")),
    ).toBe(before - readBehind);
  });
});

test.describe("detail pane loading", () => {
  test("does not fetch a message for every row walked past", async ({
    page,
  }) => {
    const backend = await openInbox(page);
    await tabToList(page);
    const steps = 29;
    backend.calls.fetched.length = 0;

    for (let i = 0; i < steps; i++) {
      await page.keyboard.press("ArrowDown");
      await page.waitForTimeout(KEY_REPEAT_MS);
    }

    expect(await position(page)).toBe(steps + 1);
    const landed = `/messages/${messageId(steps)}`;
    await expect.poll(() => backend.calls.fetched.at(-1)).toBe(landed);
    expect(backend.calls.fetched.length).toBeLessThan(MAX_SETTLED_FETCHES);
  });

  test("loads the message a deliberate selection lands on", async ({
    page,
  }) => {
    const backend = await openInbox(page);
    backend.calls.fetched.length = 0;

    await page.locator('[role="option"]').nth(2).click();

    await expect
      .poll(() => backend.calls.fetched)
      .toEqual([`/messages/${messageId(2)}`]);
  });
});

test.describe("virtualizer measurement", () => {
  test("mounts and scrolls without console warnings", async ({ page }) => {
    const noise: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "warning" || m.type() === "error") noise.push(m.text());
    });
    page.on("pageerror", (e) => noise.push(e.message));

    await openInbox(page);
    await tabToList(page);
    await arrowDownTo(page, 40);

    expect(noise).toEqual([]);
  });

  test("stacks rows on measured heights, not the estimate", async ({
    page,
  }) => {
    await openInbox(page);

    const rows = await page
      .locator('[role="presentation"]')
      .evaluateAll((els) =>
        els.slice(0, 5).map((el) => {
          const box = el.getBoundingClientRect();
          return { top: box.top, height: box.height };
        }),
      );

    expect(rows.length).toBeGreaterThan(2);
    for (const [i, row] of rows.slice(1).entries()) {
      const previous = rows[i];
      expect(previous).toBeDefined();
      if (!previous) continue;
      expect(row.top - previous.top).toBeCloseTo(previous.height, 0);
    }
  });
});

test.describe("global shortcuts", () => {
  test("j and k walk the list", async ({ page }) => {
    await openInbox(page);

    await page.keyboard.press("j");
    expect(await position(page)).toBe(2);

    await page.keyboard.press("k");
    expect(await position(page)).toBe(1);
  });

  test("d takes the message out of the list before deleting it", async ({
    page,
  }) => {
    const backend = await openInboxAtRest(page);
    const id = await selectedOption(page).getAttribute("data-id");

    await page.keyboard.press("d");

    await expect(row(page, id)).toHaveCount(0);
    await expect(page.getByRole("status")).toContainText("Message deleted");
    expect(backend.calls.deleted).toEqual([]);
  });

  test("d deletes the message once its undo window closes", async ({
    page,
  }) => {
    const backend = await openInboxAtRest(page);
    const id = await selectedOption(page).getAttribute("data-id");

    await page.keyboard.press("d");
    await page.clock.runFor(PAST_UNDO_WINDOW_MS);

    await expect.poll(() => backend.calls.deleted).toEqual([`/messages/${id}`]);
    await expect(page.getByRole("status")).toBeHidden();
  });

  test("u brings the message back and calls the delete off", async ({
    page,
  }) => {
    const backend = await openInboxAtRest(page);
    const id = await selectedOption(page).getAttribute("data-id");

    await page.keyboard.press("d");
    await expect(row(page, id)).toHaveCount(0);
    await page.keyboard.press("u");

    await expect(row(page, id)).toHaveCount(1);
    await expect(selectedOption(page)).toHaveAttribute("data-id", String(id));
    await page.clock.runFor(PAST_UNDO_WINDOW_MS);
    expect(backend.calls.deleted).toEqual([]);
  });

  test("a second d commits the deletion before it", async ({ page }) => {
    const backend = await openInboxAtRest(page);
    const first = await selectedOption(page).getAttribute("data-id");

    await page.keyboard.press("d");
    const second = await selectedOption(page).getAttribute("data-id");
    await page.keyboard.press("d");

    await expect
      .poll(() => backend.calls.deleted)
      .toEqual([`/messages/${first}`]);
    await page.clock.runFor(PAST_UNDO_WINDOW_MS);
    await expect
      .poll(() => backend.calls.deleted)
      .toEqual([`/messages/${first}`, `/messages/${second}`]);
  });

  test("u does nothing once the undo window has closed", async ({ page }) => {
    const backend = await openInboxAtRest(page);
    const id = await selectedOption(page).getAttribute("data-id");

    await page.keyboard.press("d");
    await page.clock.runFor(PAST_UNDO_WINDOW_MS);
    await expect.poll(() => backend.calls.deleted).toEqual([`/messages/${id}`]);
    await page.keyboard.press("u");

    await expect(row(page, id)).toHaveCount(0);
  });

  test("Shift still reaches the clear-all shortcut", async ({ page }) => {
    const backend = await openInbox(page);

    await page.keyboard.press("Shift+D");

    await expect(
      page.getByText(`All ${TOTAL_MESSAGES} messages will be permanently`, {
        exact: false,
      }),
    ).toBeVisible();
    expect(backend.calls.deleted).toEqual([]);
  });

  test("shortcuts stay out of the way while a confirmation is open", async ({
    page,
  }) => {
    const backend = await openInboxAtRest(page);
    const before = await position(page);

    await page.keyboard.press("Shift+D");
    await expect(
      page.getByText("will be permanently deleted", { exact: false }),
    ).toBeVisible();
    await page.keyboard.press("d");
    await page.keyboard.press("j");

    expect(await position(page)).toBe(before);
    await page.clock.runFor(PAST_UNDO_WINDOW_MS);
    expect(backend.calls.deleted).toEqual([]);
  });

  test("modifier chords never reach the shortcuts", async ({ page }) => {
    const backend = await openInbox(page);
    const before = await position(page);

    for (const chord of ["Meta+d", "Control+d", "Meta+j", "Control+k"]) {
      await page.keyboard.press(chord);
    }

    expect(backend.calls.deleted).toEqual([]);
    expect(await position(page)).toBe(before);
  });
});
