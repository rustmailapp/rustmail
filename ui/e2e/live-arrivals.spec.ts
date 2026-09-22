import { expect, test, type Page } from "@playwright/test";
import { messageId, mockInbox, summary } from "./inbox-fixture";

/** Indexes past the fixture's own messages, so arrivals never collide. */
const FIRST_ARRIVAL = 10_000;
const ARRIVALS = 30;
/** Far enough down that the first rows are out of view. */
const READING_OFFSET_PX = 1_500;
const READ_ROW = messageId(20);

function scroller(page: Page) {
  return page.getByRole("listbox", { name: "Messages" }).locator("..");
}

async function rowTop(page: Page, id: string): Promise<number> {
  const box = await page.locator(`[data-id="${id}"]`).boundingBox();
  if (box === null) throw new Error(`row ${id} is not on screen`);
  return box.y;
}

test("rows stay put under a scrolled reader while mail arrives", async ({
  page,
}) => {
  const backend = await mockInbox(page);
  await page.goto("/");
  await expect(page.locator('[role="option"]').first()).toBeVisible();
  await scroller(page).evaluate((el, top) => {
    el.scrollTop = top;
  }, READING_OFFSET_PX);
  await expect(page.locator(`[data-id="${READ_ROW}"]`)).toBeVisible();
  const before = await rowTop(page, READ_ROW);

  for (let i = 0; i < ARRIVALS; i += 1) {
    await backend.push({
      type: "message:new",
      data: summary(FIRST_ARRIVAL + i),
    });
  }
  const pill = page.getByRole("button", { name: `${ARRIVALS} new` });
  await expect(pill).toBeVisible();

  expect(await rowTop(page, READ_ROW)).toBe(before);
  await expect(page.locator("header")).toContainText(
    `${600 + ARRIVALS} messages`,
  );

  await pill.click();
  await expect(page.locator('[role="option"]').first()).toHaveAttribute(
    "data-id",
    messageId(FIRST_ARRIVAL + ARRIVALS - 1),
  );
  await expect(pill).toBeHidden();
});

test("live mail enters the list while the reader is at the top", async ({
  page,
}) => {
  const backend = await mockInbox(page);
  await page.goto("/");
  await expect(page.locator('[role="option"]').first()).toBeVisible();

  await backend.push({ type: "message:new", data: summary(FIRST_ARRIVAL) });

  await expect(page.locator('[role="option"]').first()).toHaveAttribute(
    "data-id",
    messageId(FIRST_ARRIVAL),
  );
  await expect(page.getByRole("button", { name: /new$/ })).toHaveCount(0);
});
