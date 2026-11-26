import { expect, test, type Page } from "@playwright/test";
import { SERVER_INFO, mockInbox, summary } from "./inbox-fixture";

const ADDRESS = `127.0.0.1:${SERVER_INFO.smtp_port}`;
const FILTERED_INBOX = 3;

test.use({ permissions: ["clipboard-read", "clipboard-write"] });

function inbox(page: Page) {
  return page.getByRole("complementary", { name: "Inbox" });
}

function connectApp(page: Page) {
  return page.getByRole("region", { name: "Connect your app" });
}

function clipboard(page: Page): Promise<string> {
  return page.evaluate(() => navigator.clipboard.readText());
}

test("an empty inbox shows where to send mail and copies it", async ({
  page,
}) => {
  await mockInbox(page, 0);
  await page.goto("/");

  await expect(inbox(page).getByTestId("smtp-address")).toHaveText(ADDRESS);
  await expect(inbox(page)).toContainText("As reached from this browser");

  await inbox(page)
    .getByRole("button", { name: "Copy the SMTP address" })
    .click();

  await expect(
    inbox(page).getByRole("button", { name: "Copied" }),
  ).toBeVisible();
  expect(await clipboard(page)).toBe(ADDRESS);
});

test("the snippets carry the host and port and copy the selected one", async ({
  page,
}) => {
  await mockInbox(page, 0);
  await page.goto("/");
  const panel = connectApp(page);

  await expect(panel.getByRole("tab", { name: "Node.js" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(panel.getByRole("tabpanel")).toContainText('host: "127.0.0.1"');
  await expect(panel.getByRole("tabpanel")).toContainText(
    `port: ${SERVER_INFO.smtp_port}`,
  );

  await panel.getByRole("tab", { name: "Laravel" }).click();
  const code = panel.getByRole("tabpanel").locator("pre");
  await expect(code).toHaveText(
    `MAIL_MAILER=smtp\nMAIL_HOST=127.0.0.1\nMAIL_PORT=${SERVER_INFO.smtp_port}`,
  );

  await panel
    .getByRole("button", { name: "Copy the Laravel settings" })
    .click();
  expect(await clipboard(page)).toBe(await code.textContent());

  await panel.getByRole("tab", { name: "Laravel" }).press("ArrowRight");
  await expect(panel.getByRole("tab", { name: "Rails" })).toBeFocused();
  await expect(panel.getByRole("tabpanel")).toContainText("address:");
});

test("the onboarding gives way to the first message", async ({ page }) => {
  const backend = await mockInbox(page, 0);
  await page.goto("/");
  await expect(connectApp(page)).toBeVisible();

  await backend.push({ type: "message:new", data: summary(0) });

  await expect(page.locator('[role="option"]')).toHaveCount(1);
  await expect(inbox(page).getByTestId("smtp-address")).toHaveCount(0);
  await expect(connectApp(page)).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Message" })).toBeVisible();
});

test("a filter with no matches keeps its own empty state", async ({ page }) => {
  await mockInbox(page, FILTERED_INBOX);
  await page.goto("/");
  await expect(page.locator('[role="option"]').first()).toBeVisible();

  await page.getByRole("button", { name: "Starred" }).click();

  await expect(inbox(page)).toContainText("No matching messages");
  await expect(inbox(page).getByTestId("smtp-address")).toHaveCount(0);
  await expect(connectApp(page)).toHaveCount(0);

  await inbox(page).getByRole("button", { name: "Clear filters" }).click();
  await expect(page.locator('[role="option"]')).toHaveCount(FILTERED_INBOX);
});

test("without the server's port the empty inbox keeps the plain hint", async ({
  page,
}) => {
  await mockInbox(page, 0);
  await page.route(/\/api\/v1\/info$/, (route) =>
    route.fulfill({ status: 500, json: { error: "nope" } }),
  );
  await page.goto("/");

  await expect(inbox(page)).toContainText(
    "Send an email to the SMTP port to get started",
  );
  await expect(connectApp(page)).toHaveCount(0);
});
