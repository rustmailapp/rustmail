import { expect, test, type Page } from "@playwright/test";
import { mockInbox, summary } from "./inbox-fixture";
import type { Message } from "../src/lib/types";

const VIEWPORT = { width: 1440, height: 900 };
const INBOX_SIZE = 20;
const MOBILE_WIDTH_PX = 375;
const STORAGE_KEY = "rustmail-preview-width";
const PREVIEW_FRAME = 'iframe[title="Email HTML preview"]';

/** Serves every message with an HTML part, and a raw source for each. */
async function serveHtmlMessages(page: Page): Promise<void> {
  await page.route(/\/api\/v1\/messages\/msg-(\d{4})$/, (route, request) => {
    if (request.method() !== "GET") return route.fallback();
    const match = /msg-(\d{4})$/.exec(new URL(request.url()).pathname);
    const index = Number(match?.[1]);
    const message: Message = {
      ...summary(index),
      text_body: `Text of Message ${index}`,
      html_body: `<p>HTML of Message ${index}</p>`,
    };
    return route.fulfill({ json: message });
  });
  await page.route(/\/api\/v1\/messages\/msg-\d{4}\/raw(\?.*)?$/, (route) =>
    route.fulfill({ body: "Raw source" }),
  );
}

function heading(page: Page, subject: string) {
  return page.getByRole("heading", { name: subject, exact: true });
}

function widthSwitch(page: Page) {
  return page.getByRole("group", { name: "Preview width" });
}

function frameClientWidth(page: Page): Promise<number> {
  return page.locator(PREVIEW_FRAME).evaluate((frame) => frame.clientWidth);
}

async function openInbox(page: Page): Promise<void> {
  await mockInbox(page, INBOX_SIZE);
  await serveHtmlMessages(page);
  await page.goto("/");
  await expect(heading(page, "Message 0")).toBeVisible();
  await expect(
    page.frameLocator(PREVIEW_FRAME).getByText("HTML of Message 0"),
  ).toBeVisible();
}

test.describe("preview width", () => {
  test.use({ viewport: VIEWPORT });

  test("starts on Desktop at the full preview width", async ({ page }) => {
    await openInbox(page);

    await expect(
      widthSwitch(page).getByRole("button", { name: "Desktop" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(await frameClientWidth(page)).toBeGreaterThan(MOBILE_WIDTH_PX);
  });

  test("Mobile narrows the preview and survives a new selection and a reload", async ({
    page,
  }) => {
    await openInbox(page);
    const mobile = widthSwitch(page).getByRole("button", { name: "Mobile" });

    await mobile.click();

    await expect(mobile).toHaveAttribute("aria-pressed", "true");
    await expect.poll(() => frameClientWidth(page)).toBe(MOBILE_WIDTH_PX);
    expect(
      await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY),
    ).toBe("mobile");

    await page.keyboard.press("j");
    await expect(heading(page, "Message 1")).toBeVisible();
    await expect(
      page.frameLocator(PREVIEW_FRAME).getByText("HTML of Message 1"),
    ).toBeVisible();
    await expect(mobile).toHaveAttribute("aria-pressed", "true");
    await expect.poll(() => frameClientWidth(page)).toBe(MOBILE_WIDTH_PX);

    await page.reload();
    await expect(heading(page, "Message 0")).toBeVisible();
    await expect(mobile).toHaveAttribute("aria-pressed", "true");
    await expect.poll(() => frameClientWidth(page)).toBe(MOBILE_WIDTH_PX);
  });

  test("falls back to Desktop for an unknown stored width", async ({
    page,
  }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "tablet"),
      STORAGE_KEY,
    );
    await openInbox(page);

    await expect(
      widthSwitch(page).getByRole("button", { name: "Desktop" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  test("is shown only while Preview is active", async ({ page }) => {
    await openInbox(page);
    await expect(widthSwitch(page)).toBeVisible();

    await page.getByRole("button", { name: "Text", exact: true }).click();
    await expect(page.locator("pre")).toHaveText("Text of Message 0");
    await expect(widthSwitch(page)).toHaveCount(0);

    await page.getByRole("button", { name: "Raw", exact: true }).click();
    await expect(page.locator("pre")).toHaveText("Raw source");
    await expect(widthSwitch(page)).toHaveCount(0);

    await page.getByRole("button", { name: "Preview", exact: true }).click();
    await expect(widthSwitch(page)).toBeVisible();
  });
});
