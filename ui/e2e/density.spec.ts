import { expect, test, type Locator, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const VIEWPORT = { width: 1440, height: 900 };
const STORAGE_KEY = "rustmail-density";
/** Enough to cross two page boundaries while walking the list. */
const INBOX_SIZE = 320;
/** Past the second page, so the walk measures rows loaded on scroll. */
const WALK_TARGET_POSITION = 260;
const WALK_BACK_STEPS = 30;
const COMPACT_ROW_PX = 32;
const COMFORTABLE_ROW_PX = 56;
const MIN_TARGET_PX = 24;
const WALK_TIMEOUT_MS = 120_000;
/** Mirrors the first page the inbox loads. */
const FIRST_PAGE_SIZE = 100;

function list(page: Page): Locator {
  return page.getByRole("listbox", { name: "Messages" });
}

function selectedOption(page: Page): Locator {
  return page.locator('[role="option"][aria-selected="true"]');
}

async function position(page: Page): Promise<number> {
  return Number(await selectedOption(page).getAttribute("aria-posinset"));
}

function densityButton(page: Page, label: string): Locator {
  return page.getByRole("button", { name: label, exact: true });
}

async function openInbox(page: Page, total = INBOX_SIZE): Promise<void> {
  await mockInbox(page, total, (i) => ({
    has_attachments: i % 3 === 0,
    tags: i % 4 === 0 ? ["signup", "welcome"] : [],
    is_starred: i % 5 === 0,
  }));
  await page.goto("/");
  await expect(list(page)).toBeVisible();
  await expect(selectedOption(page)).toHaveCount(1);
}

async function openLayoutSettings(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("tab", { name: "Layout", exact: true }).click();
}

function firstRowHeight(page: Page): Promise<number> {
  return list(page)
    .locator('[role="presentation"]')
    .first()
    .evaluate((el) => el.getBoundingClientRect().height);
}

/**
 * How the rendered rows sit against the compact grid.
 *
 * Every row is expected exactly `index * COMPACT_ROW_PX` from the top of the
 * list, at the compact height, and the list as tall as all its rows: any
 * drift here is what makes a virtualized list jump while scrolling.
 */
function gridOffsets(page: Page) {
  return list(page).evaluate((listbox) => {
    const rows = Array.from(
      listbox.querySelectorAll<HTMLElement>('[role="presentation"]'),
    );
    const listTop = listbox.getBoundingClientRect().top;
    const options = listbox.querySelectorAll('[role="option"]');
    const setSize = Number(options[0]?.getAttribute("aria-setsize"));
    return {
      listHeight: listbox.getBoundingClientRect().height,
      rendered: rows.length,
      setSize,
      rows: rows.map((row) => {
        const box = row.getBoundingClientRect();
        return {
          index: Number(row.dataset.index),
          top: Math.round(box.top - listTop),
          height: box.height,
        };
      }),
    };
  });
}

/** Whether the selected row is fully inside the scrolling list's viewport. */
function selectedInView(page: Page): Promise<boolean> {
  return selectedOption(page).evaluate((option) => {
    const scroller = option.closest(".inbox-list");
    if (!scroller) return false;
    const view = scroller.getBoundingClientRect();
    const box = option.getBoundingClientRect();
    return box.top >= view.top && box.bottom <= view.bottom;
  });
}

/** The list's scroll offset once the frame after the last keypress has painted. */
function scrollTop(page: Page): Promise<number> {
  return list(page).evaluate(
    (el) =>
      new Promise<number>((resolve) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() =>
            resolve(el.parentElement?.scrollTop ?? 0),
          ),
        ),
      ),
  );
}

test.describe("inbox density", () => {
  test.use({ viewport: VIEWPORT });

  test("defaults to Comfortable", async ({ page }) => {
    await openInbox(page);
    await openLayoutSettings(page);

    await expect(densityButton(page, "Comfortable")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(await firstRowHeight(page)).toBe(COMFORTABLE_ROW_PX);
  });

  test("Settings tabs move with the arrow keys and keep one tab stop", async ({
    page,
  }) => {
    await openInbox(page);
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const appearance = page.getByRole("tab", { name: "Appearance" });
    const layout = page.getByRole("tab", { name: "Layout" });
    await appearance.focus();

    await page.keyboard.press("ArrowRight");

    await expect(layout).toBeFocused();
    await expect(layout).toHaveAttribute("aria-selected", "true");
    await expect(appearance).toHaveAttribute("tabindex", "-1");
    await expect(page.getByRole("tabpanel", { name: "Layout" })).toContainText(
      "Density",
    );

    await page.keyboard.press("ArrowRight");
    await expect(appearance).toBeFocused();
    await expect(appearance).toHaveAttribute("aria-selected", "true");
  });

  test("Compact shrinks the rows and survives a reload", async ({ page }) => {
    await openInbox(page);
    await openLayoutSettings(page);

    await densityButton(page, "Compact").click();

    await expect(densityButton(page, "Compact")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect.poll(() => firstRowHeight(page)).toBe(COMPACT_ROW_PX);
    expect(
      await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY),
    ).toBe("compact");

    await page.reload();
    await expect(selectedOption(page)).toHaveCount(1);
    expect(await firstRowHeight(page)).toBe(COMPACT_ROW_PX);

    await openLayoutSettings(page);
    await expect(densityButton(page, "Compact")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await densityButton(page, "Comfortable").click();
    await expect.poll(() => firstRowHeight(page)).toBe(COMFORTABLE_ROW_PX);
  });

  test("falls back to Comfortable for an unknown stored density", async ({
    page,
  }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "cozy"),
      STORAGE_KEY,
    );
    await openInbox(page);

    expect(await firstRowHeight(page)).toBe(COMFORTABLE_ROW_PX);
  });

  test("keeps a 24px star target in a compact row", async ({ page }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "compact"),
      STORAGE_KEY,
    );
    await openInbox(page);

    const star = selectedOption(page).getByRole("button", { name: /star/i });
    const box = await star.boundingBox();

    expect(box?.width).toBeGreaterThanOrEqual(MIN_TARGET_PX);
    expect(box?.height).toBeGreaterThanOrEqual(MIN_TARGET_PX);
  });

  test("sizes rows that were never mounted at the compact height", async ({
    page,
  }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "compact"),
      STORAGE_KEY,
    );
    await openInbox(page);

    const grid = await gridOffsets(page);

    expect(grid.rendered).toBeLessThan(FIRST_PAGE_SIZE);
    expect(grid.listHeight).toBe(FIRST_PAGE_SIZE * COMPACT_ROW_PX);
  });

  test("keeps the selected row in view when the density changes", async ({
    page,
  }) => {
    await openInbox(page);
    for (let i = 0; i < 40; i++) await page.keyboard.press("j");
    await expect.poll(() => position(page)).toBe(41);

    await openLayoutSettings(page);
    await densityButton(page, "Compact").click();
    await page.keyboard.press("Escape");

    await expect.poll(() => selectedInView(page)).toBe(true);
    await expect.poll(() => firstRowHeight(page)).toBe(COMPACT_ROW_PX);
  });

  test("walks 300+ compact rows with j and k without the scroll jumping", async ({
    page,
  }) => {
    test.setTimeout(WALK_TIMEOUT_MS);
    await page.addInitScript(
      (key) => localStorage.setItem(key, "compact"),
      STORAGE_KEY,
    );
    await openInbox(page);

    let previousTop = await scrollTop(page);
    while ((await position(page)) < WALK_TARGET_POSITION) {
      await page.keyboard.press("j");
      const top = await scrollTop(page);
      expect(top).toBeGreaterThanOrEqual(previousTop);
      previousTop = top;
    }
    expect(await selectedInView(page)).toBe(true);

    const grid = await gridOffsets(page);
    expect(grid.setSize).toBe(INBOX_SIZE);
    expect(grid.rendered).toBeGreaterThan(0);
    for (const row of grid.rows) {
      expect(row.height).toBe(COMPACT_ROW_PX);
      expect(row.top).toBe(row.index * COMPACT_ROW_PX);
    }
    expect(grid.listHeight % COMPACT_ROW_PX).toBe(0);

    const start = await position(page);
    for (let i = 0; i < WALK_BACK_STEPS; i++) {
      await page.keyboard.press("k");
      const top = await scrollTop(page);
      expect(top).toBeLessThanOrEqual(previousTop);
      previousTop = top;
    }
    expect(await position(page)).toBe(start - WALK_BACK_STEPS);
    expect(await selectedInView(page)).toBe(true);
  });
});
