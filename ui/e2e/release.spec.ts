import { expect, test, type Page, type Route } from "@playwright/test";
import { mockInbox, SERVER_INFO, type InboxBackend } from "./inbox-fixture";
import type { MessageSummary } from "../src/lib/types";

const MESSAGES = 3;
const RELAY = { host: "smtp.example.test", port: 465 };
const RECIPIENTS = [
  "to@example.test",
  "cc@example.test",
  "hidden-bcc@example.test",
];
const RELEASE = /\/api\/v1\/messages\/[^/]+\/release$/;
const GATEWAY_TIMEOUT = 504;
/** Wide enough for the details rail to sit beside the message. */
const WIDE_VIEWPORT = { width: 1440, height: 900 };
/** The page's clock, pinned so "today" and the times it shows are fixed. */
const NOW = new Date("2026-11-26T15:00:00Z");
const RELEASED_TODAY = "2026-11-26T13:05:09Z";
const RELEASED_YESTERDAY = "2026-11-25T13:05:09.250Z";

test.use({ locale: "en-US", timezoneId: "UTC", viewport: WIDE_VIEWPORT });

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(NOW);
});

async function openInbox(
  page: Page,
  release: typeof RELAY | null,
  recorded: Partial<MessageSummary> = {},
): Promise<InboxBackend> {
  const backend = await mockInbox(page, MESSAGES, () => ({
    recipients: RECIPIENTS,
    ...recorded,
  }));
  await page.route(/\/api\/v1\/info$/, (route) =>
    route.fulfill({ json: { ...SERVER_INFO, release } }),
  );
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
  return backend;
}

function details(page: Page) {
  return page.getByRole("complementary", { name: "Message details" });
}

function warning(page: Page) {
  return page
    .getByRole("dialog", { name: "Release this message?" })
    .getByText(/released at|A release at/);
}

async function openMessageId(page: Page): Promise<string> {
  const id = await page
    .locator('[role="option"][aria-selected="true"]')
    .getAttribute("data-id");
  if (id === null) throw new Error("no message is open");
  return id;
}

function releaseButton(page: Page) {
  return page.getByRole("button", { name: "Release", exact: true });
}

function dialog(page: Page) {
  return page.getByRole("heading", { name: "Release this message?" });
}

async function confirmRelease(page: Page): Promise<void> {
  await releaseButton(page).click();
  await page
    .getByRole("dialog", { name: "Release this message?" })
    .getByRole("button", { name: "Release", exact: true })
    .click();
}

test("hides the release button while the server has no relay", async ({
  page,
}) => {
  await openInbox(page, null);

  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
  await expect(releaseButton(page)).toHaveCount(0);
});

test("names the relay and every envelope recipient before sending", async ({
  page,
}) => {
  await openInbox(page, RELAY);
  const posted: string[] = [];
  await page.route(RELEASE, (route) => {
    posted.push(route.request().url());
    return route.fulfill({ json: { released: true } });
  });

  await releaseButton(page).click();

  await expect(dialog(page)).toBeVisible();
  await expect(
    page.getByText("as captured through smtp.example.test:465"),
  ).toBeVisible();
  await expect(
    page
      .getByRole("list", { name: "Envelope recipients" })
      .getByRole("listitem"),
  ).toHaveText(RECIPIENTS);
  await expect(page.getByRole("button", { name: "Cancel" })).toBeFocused();

  await page.keyboard.press("Enter");
  await expect(dialog(page)).toBeHidden();
  expect(posted).toEqual([]);
});

test("sends the confirmed relay and reports the release", async ({ page }) => {
  await openInbox(page, RELAY);
  let answer!: () => void;
  const answered = new Promise<void>((resolve) => (answer = resolve));
  const bodies: unknown[] = [];
  await page.route(RELEASE, async (route: Route) => {
    bodies.push(route.request().postDataJSON());
    await answered;
    return route.fulfill({ json: { released: true } });
  });

  await confirmRelease(page);

  await expect(releaseButton(page)).toHaveAttribute("aria-disabled", "true");
  await releaseButton(page).dispatchEvent("click");
  await expect(dialog(page)).toBeHidden();
  answer();

  await expect(page.getByRole("alert")).toHaveText(
    "Released to 3 recipients through smtp.example.test:465.",
  );
  await expect(releaseButton(page)).toHaveAttribute("aria-disabled", "false");
  expect(bodies).toEqual([RELAY]);
  await expect(page.getByRole("alert")).toHaveCount(1);
});

test("says the outcome is unknown when the relay timed out", async ({
  page,
}) => {
  await openInbox(page, RELAY);
  await page.route(RELEASE, (route) =>
    route.fulfill({
      status: GATEWAY_TIMEOUT,
      json: { error: "timed out", code: "release_outcome_unknown" },
    }),
  );

  await confirmRelease(page);

  await expect(page.getByRole("alert")).toContainText(
    "may have been delivered",
  );
  await expect(page.getByRole("alert")).toContainText(
    "Check smtp.example.test:465 before releasing it again.",
  );
});

test("asks without a warning for a message never released", async ({
  page,
}) => {
  await openInbox(page, RELAY);

  await releaseButton(page).click();

  await expect(dialog(page)).toBeVisible();
  await expect(warning(page)).toHaveCount(0);
  await expect(details(page).getByText("Released")).toHaveCount(0);
});

test("warns that a message was already released today", async ({ page }) => {
  await openInbox(page, RELAY, {
    released_at: RELEASED_TODAY,
    release_outcome: "sent",
  });

  await expect(details(page).getByText("1:05 PM, sent")).toBeVisible();
  await releaseButton(page).click();

  await expect(warning(page)).toHaveText("Already released at 1:05 PM");
  await expect(
    page
      .getByRole("dialog", { name: "Release this message?" })
      .getByRole("button", { name: "Release", exact: true }),
  ).toBeEnabled();
});

test("warns that a release on another day may have been delivered", async ({
  page,
}) => {
  await openInbox(page, RELAY, {
    released_at: RELEASED_YESTERDAY,
    release_outcome: "unknown",
  });

  await expect(
    details(page).getByText("Nov 25, 2026, 1:05 PM, outcome unknown"),
  ).toBeVisible();
  await releaseButton(page).click();

  await expect(warning(page)).toHaveText(
    "A release at Nov 25, 2026, 1:05 PM may have been delivered",
  );
});

test("records this tab's release from the answer, before any broadcast", async ({
  page,
}) => {
  await openInbox(page, RELAY);
  await page.route(RELEASE, (route) =>
    route.fulfill({
      json: {
        released: true,
        released_at: RELEASED_TODAY,
        release_outcome: "sent",
      },
    }),
  );

  await confirmRelease(page);

  await expect(details(page).getByText("1:05 PM, sent")).toBeVisible();
  await releaseButton(page).click();
  await expect(warning(page)).toHaveText("Already released at 1:05 PM");
});

test("records the release an unknown outcome left behind", async ({ page }) => {
  await openInbox(page, RELAY);
  await page.route(RELEASE, (route) =>
    route.fulfill({
      status: GATEWAY_TIMEOUT,
      json: {
        error: "timed out",
        code: "release_outcome_unknown",
        released_at: RELEASED_TODAY,
        release_outcome: "unknown",
      },
    }),
  );

  await confirmRelease(page);

  await expect(
    details(page).getByText("1:05 PM, outcome unknown"),
  ).toBeVisible();
});

test("shows a release another tab made, from the broadcast", async ({
  page,
}) => {
  const backend = await openInbox(page, RELAY);
  const id = await openMessageId(page);

  await backend.push({
    type: "message:released",
    data: { id, released_at: RELEASED_TODAY, release_outcome: "unknown" },
  });

  await expect(
    details(page).getByText("1:05 PM, outcome unknown"),
  ).toBeVisible();
  await releaseButton(page).click();
  await expect(warning(page)).toHaveText(
    "A release at 1:05 PM may have been delivered",
  );
});
