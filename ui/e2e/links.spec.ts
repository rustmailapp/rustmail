import { expect, test, type Page } from "@playwright/test";
import { mockInbox, summary } from "./inbox-fixture";
import type { Message } from "../src/lib/types";

const WIDE_VIEWPORT = { width: 1440, height: 900 };
const INBOX_SIZE = 5;
const LINKS_PREVIEW = 8;
const MANY_LINKS = 11;

const LINKED_HTML = `
  <p><a href="https://example.com/welcome">Welcome
     aboard</a></p>
  <a href="http://example.com/plain">Plain http</a>
  <a href="http://localhost:3000/verify?token=abc">Verify</a>
  <a href="mailto:help@example.com">Mail us</a>
  <a href="https://example.com/welcome">Again</a>
  <a href="#top">Top</a>
  <a href="javascript:alert(1)">Run</a>
`;
const LINKED_TEXT = "Welcome: https://example.com/welcome.";

function linksSection(page: Page) {
  return page
    .getByRole("complementary", { name: "Message details" })
    .locator("section", {
      has: page.getByRole("heading", { name: "Links", exact: true }),
    });
}

function heading(page: Page, subject: string) {
  return page.getByRole("heading", { name: subject, exact: true });
}

/** Serves the first message with the given bodies. */
async function serveFirstMessage(
  page: Page,
  html: string,
  text: string | null,
): Promise<void> {
  await page.route(/\/api\/v1\/messages\/msg-0000$/, (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const message: Message = {
      ...summary(0),
      html_body: html,
      text_body: text,
    };
    return route.fulfill({ json: message });
  });
}

test.describe("links section", () => {
  test.use({ viewport: WIDE_VIEWPORT });

  test("lists each link once with its flags and count", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await serveFirstMessage(page, LINKED_HTML, LINKED_TEXT);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const section = linksSection(page);
    const rows = section.getByRole("listitem");

    await expect(section.locator("h3 + span")).toHaveText("4");
    await expect(rows).toHaveCount(4);

    await expect(rows.nth(0)).toContainText("example.com/welcome");
    await expect(rows.nth(0)).toContainText("×2");
    await expect(rows.nth(0)).toContainText("Welcome aboard");
    await expect(rows.nth(1)).toContainText("Plain http");
    await expect(rows.nth(2)).toContainText("localhost:3000");
    await expect(rows.nth(3)).toContainText("help@example.com");

    const badge = (name: string) => section.getByText(name, { exact: true });
    await expect(badge("http")).toHaveCount(2);
    await expect(badge("local")).toHaveCount(1);
    await expect(rows.nth(0).getByText("http", { exact: true })).toHaveCount(0);
    await expect(rows.nth(2).getByText("http", { exact: true })).toBeVisible();
    await expect(rows.nth(2).getByText("local", { exact: true })).toBeVisible();

    const first = rows.nth(0).getByRole("link");
    await expect(first).toHaveAttribute("href", "https://example.com/welcome");
    await expect(first).toHaveAttribute("target", "_blank");
    await expect(first).toHaveAttribute("rel", "noopener noreferrer");
    await expect(section).not.toContainText("alert");
  });

  test("says so when the message has no links", async ({ page }) => {
    await mockInbox(page, INBOX_SIZE);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const section = linksSection(page);

    await expect(section).toContainText("No links");
    await expect(section.locator("h3 + span")).toHaveText("0");
    await expect(section.getByRole("listitem")).toHaveCount(0);
  });

  test("shows the first few until asked for all of them", async ({ page }) => {
    const html = Array.from(
      { length: MANY_LINKS },
      (_, i) => `<a href="https://example.com/${i}">Link ${i}</a>`,
    ).join(" ");
    await mockInbox(page, INBOX_SIZE);
    await serveFirstMessage(page, html, null);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const rows = linksSection(page).getByRole("listitem");
    const showAll = page.getByRole("button", {
      name: `Show all ${MANY_LINKS}`,
    });

    await expect(rows).toHaveCount(LINKS_PREVIEW);
    await showAll.click();
    await expect(rows).toHaveCount(MANY_LINKS);
    await expect(showAll).toHaveCount(0);

    await page.keyboard.press("j");
    await expect(heading(page, "Message 1")).toBeVisible();
    await page.keyboard.press("k");
    await expect(heading(page, "Message 0")).toBeVisible();
    await expect(rows).toHaveCount(LINKS_PREVIEW);
  });

  test("selects the link when the clipboard is out of reach", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      Object.defineProperty(Navigator.prototype, "clipboard", {
        value: undefined,
      });
    });
    await mockInbox(page, INBOX_SIZE);
    await serveFirstMessage(page, LINKED_HTML, null);
    await page.goto("/");
    await expect(heading(page, "Message 0")).toBeVisible();
    const row = linksSection(page).getByRole("listitem").nth(0);

    await row.getByRole("button", { name: "Copy the link" }).click();

    await expect(row).toContainText("Press Cmd/Ctrl+C to copy");
    expect(await page.evaluate(() => window.getSelection()?.toString())).toBe(
      "https://example.com/welcome",
    );
  });
});
