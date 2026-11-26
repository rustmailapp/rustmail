import { expect, test, type Page, type Route } from "@playwright/test";
import { mockInbox, SERVER_INFO } from "./inbox-fixture";

const MESSAGES = 3;
const RELAY = { host: "smtp.example.test", port: 465 };
const RECIPIENTS = [
  "to@example.test",
  "cc@example.test",
  "hidden-bcc@example.test",
];
const RELEASE = /\/api\/v1\/messages\/[^/]+\/release$/;
const GATEWAY_TIMEOUT = 504;

async function openInbox(
  page: Page,
  release: typeof RELAY | null,
): Promise<void> {
  await mockInbox(page, MESSAGES, () => ({ recipients: RECIPIENTS }));
  await page.route(/\/api\/v1\/info$/, (route) =>
    route.fulfill({ json: { ...SERVER_INFO, release } }),
  );
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
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
