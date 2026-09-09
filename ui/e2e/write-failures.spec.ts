import { expect, test, type Page } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";
import { NOTICE_TTL_MS } from "../src/stores/notices";

const SINGLE_MESSAGE = /\/api\/v1\/messages\/[^/]+$/;
const WHOLE_INBOX = /\/api\/v1\/messages$/;
const SERVER_ERROR = 500;
/**
 * Long enough to close a deletion's undo window, short enough to stay inside
 * the request deadline behind it.
 */
const PAST_UNDO_WINDOW_MS = 6_000;

function selectedOption(page: Page) {
  return page.locator('[role="option"][aria-selected="true"]');
}

function row(page: Page, id: string | null) {
  return page.locator(`[role="option"][data-id="${id}"]`);
}

/** Opens the inbox with the clock stopped, so a test drives it itself. */
async function openInboxAtRest(page: Page): Promise<void> {
  const now = new Date("2026-09-08T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now);
  await page.goto("/");
  await expect(selectedOption(page)).toHaveCount(1);
}

/** Fails one method against the fixture, leaving every other call alone. */
async function breakWrite(
  page: Page,
  pattern: RegExp,
  method: string,
  matches: (body: Record<string, unknown>) => boolean = () => true,
): Promise<void> {
  await page.route(pattern, async (route) => {
    const request = route.request();
    if (request.method() !== method) return route.fallback();
    const body =
      method === "PATCH"
        ? (request.postDataJSON() as Record<string, unknown>)
        : {};
    if (!matches(body)) return route.fallback();
    return route.fulfill({ status: SERVER_ERROR, json: { error: "nope" } });
  });
}

test("says the message is back when its deletion fails", async ({ page }) => {
  await mockInbox(page, 5);
  await breakWrite(page, SINGLE_MESSAGE, "DELETE");
  await openInboxAtRest(page);
  const id = await selectedOption(page).getAttribute("data-id");

  await page.keyboard.press("d");
  await expect(row(page, id)).toHaveCount(0);
  await page.clock.runFor(PAST_UNDO_WINDOW_MS);

  await expect(page.getByRole("alert")).toContainText(
    "Could not delete \u201CMessage 0\u201D. It is back in the inbox.",
  );
  await expect(row(page, id)).toHaveCount(1);
});

test("says the star did not land", async ({ page }) => {
  await mockInbox(page, 5);
  await breakWrite(
    page,
    SINGLE_MESSAGE,
    "PATCH",
    (body) => "is_starred" in body,
  );
  await openInboxAtRest(page);

  await page.keyboard.press("s");

  await expect(page.getByRole("alert")).toContainText(
    "Could not star \u201CMessage 0\u201D.",
  );
});

test("says the inbox did not clear", async ({ page }) => {
  await mockInbox(page, 5);
  await breakWrite(page, WHOLE_INBOX, "DELETE");
  await openInboxAtRest(page);

  await page.getByRole("button", { name: "Clear all" }).click();
  await page
    .getByRole("button", { name: "Clear all", exact: true })
    .last()
    .click();

  await expect(page.getByRole("alert")).toContainText(
    "Could not clear the inbox. The messages are still here.",
  );
  await expect(page.locator('[role="option"]').first()).toBeVisible();
});

test("takes the notice down on its own", async ({ page }) => {
  await mockInbox(page, 5);
  await breakWrite(
    page,
    SINGLE_MESSAGE,
    "PATCH",
    (body) => "is_starred" in body,
  );
  await openInboxAtRest(page);
  await page.keyboard.press("s");
  await expect(page.getByRole("alert")).toBeVisible();

  await page.clock.runFor(NOTICE_TTL_MS);

  await expect(page.getByRole("alert")).toBeHidden();
});

test("takes the notice down when it is dismissed", async ({ page }) => {
  await mockInbox(page, 5);
  await breakWrite(
    page,
    SINGLE_MESSAGE,
    "PATCH",
    (body) => "is_starred" in body,
  );
  await openInboxAtRest(page);
  await page.keyboard.press("s");

  await page.getByRole("button", { name: "Dismiss notice" }).click();

  await expect(page.getByRole("alert")).toBeHidden();
});

test("keeps a notice and the undo toast clear of each other", async ({
  page,
}) => {
  await mockInbox(page, 5);
  await breakWrite(
    page,
    SINGLE_MESSAGE,
    "PATCH",
    (body) => "is_starred" in body,
  );
  await openInboxAtRest(page);

  await page.keyboard.press("d");
  await page.keyboard.press("s");

  const toast = page.getByRole("status");
  const notice = page.getByRole("alert");
  await expect(toast).toBeVisible();
  await expect(notice).toBeVisible();

  const toastBox = await toast.boundingBox();
  const noticeBox = await notice.boundingBox();
  expect(toastBox).not.toBeNull();
  expect(noticeBox).not.toBeNull();
  if (!toastBox || !noticeBox) return;
  expect(noticeBox.y + noticeBox.height).toBeLessThanOrEqual(toastBox.y);
});

test("keeps the socket and the shortcuts alive when the first load fails", async ({
  page,
}) => {
  const backend = await mockInbox(page, 5);
  await page.route(/\/api\/v1\/messages(\?|$)/, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    return route.fulfill({ status: SERVER_ERROR, json: { error: "nope" } });
  });

  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText(
    "Could not load the inbox.",
  );

  await backend.push({
    type: "message:new",
    data: {
      id: "msg-live",
      sender: "live@example.test",
      recipients: ["inbox@example.test"],
      subject: "Arrived over the socket",
      size: 512,
      has_attachments: false,
      is_read: false,
      is_starred: false,
      tags: [],
      created_at: "2026-02-01T00:00:00Z",
    },
  });
  await expect(page.locator('[role="option"]')).toHaveCount(1);

  await page.keyboard.press("j");

  await expect(selectedOption(page)).toHaveCount(1);
});
