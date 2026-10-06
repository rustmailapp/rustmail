import { expect, test, type Page } from "@playwright/test";
import { messageId, mockInbox, SERVER_INFO, summary } from "./inbox-fixture";
import type { Message } from "../src/lib/types";

/** Wide enough for the details rail to get a column of its own. */
const WIDE_VIEWPORT = { width: 1440, height: 900 };
/** Narrow enough that the rail becomes a drawer over the message body. */
const NARROW_VIEWPORT = { width: 1180, height: 800 };
const INBOX_SIZE = 60;
const SEARCH_INPUT = 'input[placeholder="Search emails..."]';
const HTML_TEXT = "Rendered in the preview";
const PLAIN_TEXT = "The plain text part";
const RAW_TEXT = "Raw source of the first message";

function details(page: Page) {
  return page.getByRole("complementary", { name: "Message details" });
}

function heading(page: Page, subject: string) {
  return page.getByRole("heading", { name: subject, exact: true });
}

function selectedOption(page: Page) {
  return page.locator('[role="option"][aria-selected="true"]');
}

function messageList(page: Page) {
  return page.getByRole("listbox", { name: "Messages" });
}

/** The path of each read of `suffix` for the given messages, in order. */
function readsOf(suffix: string, indexes: number[]): string[] {
  return indexes.map((index) => `/messages/${messageId(index)}/${suffix}`);
}

/** Serves the first message with an HTML part, and its raw source. */
async function serveRichFirstMessage(page: Page): Promise<void> {
  await page.route(/\/api\/v1\/messages\/msg-0000$/, (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const message: Message = {
      ...summary(0),
      text_body: PLAIN_TEXT,
      html_body: `<p>${HTML_TEXT}</p>`,
    };
    return route.fulfill({ json: message });
  });
  await page.route(/\/api\/v1\/messages\/msg-0000\/raw(\?.*)?$/, (route) =>
    route.fulfill({ body: RAW_TEXT }),
  );
}

test.describe("details rail", () => {
  test.use({ viewport: WIDE_VIEWPORT });

  test("reads the headers only once they are opened", async ({ page }) => {
    const backend = await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await expect(
      details(page).getByText("No authentication headers found."),
    ).toBeVisible();

    await page.keyboard.press("j");
    await expect(heading(page, "Message 1")).toBeVisible();
    expect(backend.calls.headers).toEqual([]);

    const toggle = page.getByRole("button", { name: "Show headers" });
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await toggle.click();

    await expect
      .poll(() => backend.calls.headers)
      .toEqual([`/messages/${messageId(1)}/headers`]);
    await expect(
      details(page).getByText("Subject", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Hide headers" }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  test("reads auth and attachments once per settled selection", async ({
    page,
  }) => {
    const backend = await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();

    await page.keyboard.press("j");
    await expect(heading(page, "Message 1")).toBeVisible();
    await expect(details(page)).toContainText("sender-1@example.test");

    await expect
      .poll(() => backend.calls.auth)
      .toEqual(readsOf("auth", [0, 1]));
    await expect
      .poll(() => backend.calls.attachments)
      .toEqual(readsOf("attachments", [0, 1]));
    expect(backend.calls.headers).toEqual([]);

    await page.getByRole("button", { name: "Show headers" }).click();
    await expect
      .poll(() => backend.calls.headers)
      .toEqual(readsOf("headers", [1]));
    expect(backend.calls.auth).toEqual(readsOf("auth", [0, 1]));
    expect(backend.calls.attachments).toEqual(readsOf("attachments", [0, 1]));
  });

  test("follows the selection", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await expect(details(page)).toContainText("sender-0@example.test");

    await page.keyboard.press("j");

    await expect(heading(page, "Message 1")).toBeVisible();
    await expect(details(page)).toContainText("sender-1@example.test");
    await expect(details(page)).not.toContainText("sender-0@example.test");
  });

  test("keeps the rail beside the message and ignores i", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await expect(details(page)).toBeVisible();

    await page.keyboard.press("i");

    await expect(details(page)).toBeVisible();
    await expect(page.getByRole("button", { name: "Details" })).toHaveCount(0);
  });
});

test.describe("details drawer", () => {
  test.use({ viewport: NARROW_VIEWPORT });

  test("i opens and closes it over the message body", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const button = page.getByRole("button", { name: "Details" });
    await expect(details(page)).toHaveCount(0);
    await expect(page.getByText("SPF", { exact: false })).toBeVisible();

    await page.keyboard.press("i");

    await expect(details(page)).toBeVisible();
    await expect(button).toHaveAttribute("aria-expanded", "true");
    const drawer = await details(page).boundingBox();
    const title = await heading(page, "Message 0").boundingBox();
    expect(drawer).not.toBeNull();
    expect(title).not.toBeNull();
    if (!drawer || !title) return;
    expect(drawer.y).toBeGreaterThan(title.y + title.height);

    const mounted = await details(page).elementHandle();
    await page.keyboard.press("j");
    await expect(heading(page, "Message 1")).toBeVisible();
    await expect(details(page)).toContainText("sender-1@example.test");
    expect(await mounted?.evaluate((el) => el.isConnected)).toBe(true);

    await page.keyboard.press("i");

    await expect(details(page)).toHaveCount(0);
    await expect(button).toHaveAttribute("aria-expanded", "false");
  });

  test("the Details button toggles it too", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    const button = page.getByRole("button", { name: "Details" });

    await button.click();
    await expect(details(page)).toBeVisible();
    await button.click();
    await expect(details(page)).toHaveCount(0);
  });

  test("Escape closes it before it clears the selection", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await page.keyboard.press("i");
    await expect(details(page)).toBeVisible();

    await page.keyboard.press("Escape");

    await expect(details(page)).toHaveCount(0);
    await expect(selectedOption(page)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(selectedOption(page)).toHaveCount(0);
  });

  test("takes focus and hands it back to the Details button", async ({
    page,
  }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const button = page.getByRole("button", { name: "Details" });

    await button.click();

    await expect(details(page)).toBeFocused();
    await expect(page.locator("[inert]")).toContainText("Body of Message 0");

    await page.keyboard.press("Escape");

    await expect(details(page)).toHaveCount(0);
    await expect(button).toBeFocused();
    await expect(page.locator("[inert]")).toHaveCount(0);
    await expect(selectedOption(page)).toHaveCount(1);
  });

  test("hands focus back to the list when i opened it", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await messageList(page).focus();

    await page.keyboard.press("i");
    await expect(details(page)).toBeFocused();
    await page.keyboard.press("i");

    await expect(details(page)).toHaveCount(0);
    await expect(messageList(page)).toBeFocused();
  });

  test("Escape in the tag input closes only the drawer", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    await page.locator(SEARCH_INPUT).fill("Message");
    await messageList(page).focus();
    await page.keyboard.press("i");
    const tagInput = details(page).getByPlaceholder("Add tag...");
    await tagInput.focus();

    await page.keyboard.press("Escape");

    await expect(details(page)).toHaveCount(0);
    await expect(selectedOption(page)).toHaveCount(1);
    await expect(page.locator(SEARCH_INPUT)).toHaveValue("Message");
    await expect(messageList(page)).toBeFocused();
  });

  test("i does nothing with a modifier or while typing", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();

    await page.keyboard.press("Control+i");
    await page.keyboard.press("Alt+i");
    await expect(details(page)).toHaveCount(0);

    await page.locator(SEARCH_INPUT).focus();
    await page.keyboard.press("i");
    await expect(page.locator(SEARCH_INPUT)).toHaveValue("i");
    await expect(details(page)).toHaveCount(0);
  });
});

test.describe("message views", () => {
  test.use({ viewport: WIDE_VIEWPORT });

  test("Preview, Text and Raw switch the body", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await serveRichFirstMessage(page);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const preview = page.getByRole("button", { name: "Preview", exact: true });
    const text = page.getByRole("button", { name: "Text", exact: true });
    const raw = page.getByRole("button", { name: "Raw", exact: true });
    const frame = page.frameLocator('iframe[title="Email HTML preview"]');

    await expect(preview).toHaveAttribute("aria-pressed", "true");
    await expect(frame.getByText(HTML_TEXT)).toBeVisible();

    await text.click();
    await expect(text).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("pre")).toHaveText(PLAIN_TEXT);
    await expect(page.locator("iframe")).toHaveCount(0);

    await raw.click();
    await expect(raw).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("pre")).toHaveText(RAW_TEXT);

    await preview.click();
    await expect(frame.getByText(HTML_TEXT)).toBeVisible();
    await expect(page.locator("pre")).toHaveCount(0);
  });
});

test.describe("status bar", () => {
  test("shows the SMTP address and the live connection", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    const bar = page.getByRole("contentinfo");

    await expect(bar).toContainText("Live");
    await expect(bar).toContainText(`127.0.0.1:${SERVER_INFO.smtp_port}`);
    await expect(bar).toContainText(`v${SERVER_INFO.version}`);
    await expect(bar).toContainText(`${INBOX_SIZE} messages`);
    await expect(bar).toContainText(`${INBOX_SIZE} unread`);
  });

  test("selects the SMTP address when the clipboard is out of reach", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      Object.defineProperty(Navigator.prototype, "clipboard", {
        value: undefined,
      });
    });
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    const bar = page.getByRole("contentinfo");

    await bar.getByRole("button", { name: "Copy the SMTP address" }).click();

    await expect(bar).toContainText("Press Cmd/Ctrl+C to copy");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe(
      `127.0.0.1:${SERVER_INFO.smtp_port}`,
    );
  });

  test("says the SMTP address is unknown when the server will not tell", async ({
    page,
  }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.route(/\/api\/v1\/info$/, (route) =>
      route.fulfill({ status: 500, json: { error: "nope" } }),
    );
    await page.goto("/");

    await expect(page.getByRole("contentinfo")).toContainText("SMTP unknown");
  });
});
