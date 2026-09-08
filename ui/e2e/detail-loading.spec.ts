import { expect, test } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const SETTLED_MS = 150;
const BEFORE_SETTLED_MS = 20;
const SELECTED_OPTION = '[role="option"][aria-selected="true"]';
const RESOURCES = [
  { name: "message", suffix: "", tab: null },
  { name: "attachments", suffix: "/attachments", tab: null },
  { name: "headers", suffix: "/headers", tab: "Headers" },
  { name: "auth", suffix: "/auth", tab: "Auth" },
  { name: "raw", suffix: "/raw", tab: "Raw" },
] as const;

for (const resource of RESOURCES) {
  test(`ignores obsolete ${resource.name} errors after keyboard deletion`, async ({
    page,
  }) => {
    const backend = await mockInbox(page, 5);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    let releaseRequest = () => {};
    const released = new Promise<void>((resolve) => {
      releaseRequest = resolve;
    });
    let pending = false;
    const targetPath = `/api/v1/messages/msg-0000${resource.suffix}`;
    const resourceUrl = new RegExp(
      `/api/v1/messages/[^/]+${resource.suffix}(?:\\?.*)?$`,
    );

    await page.route(resourceUrl, async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      if (new URL(route.request().url()).pathname === targetPath) {
        pending = true;
        await released;
        return route.fulfill({ status: 404, json: {} });
      }
      switch (resource.name) {
        case "headers":
          return route.fulfill({ json: [] });
        case "auth":
          return route.fulfill({
            json: { dkim: [], spf: [], dmarc: [], arc: [] },
          });
        case "raw":
          return route.fulfill({ body: "Subject: Next message\r\n\r\nBody" });
        default:
          return route.fallback();
      }
    });

    const now = new Date("2026-09-08T12:00:00Z");
    await page.clock.install({ time: now });
    await page.clock.pauseAt(now);
    await page.goto("/");
    await expect(page.locator(SELECTED_OPTION)).toHaveAttribute(
      "aria-posinset",
      "1",
    );
    await page.clock.runFor(SETTLED_MS);
    if (resource.tab) {
      await page
        .getByRole("button", { name: resource.tab, exact: true })
        .click();
    }
    await expect.poll(() => pending).toBe(true);

    await page.getByRole("listbox", { name: "Messages" }).focus();
    await page.keyboard.press("d");
    await expect(page.locator(SELECTED_OPTION)).toHaveAttribute(
      "aria-posinset",
      "2",
    );
    await expect
      .poll(() => backend.calls.deleted)
      .toEqual(["/messages/msg-0000"]);
    const response = page.waitForResponse(
      (res) =>
        new URL(res.url()).pathname === targetPath &&
        res.request().method() === "GET",
    );
    releaseRequest();
    await (await response).finished();
    await page.clock.runFor(BEFORE_SETTLED_MS);
    await page.evaluate(
      () => new Promise<void>((resolve) => queueMicrotask(resolve)),
    );
    expect(errors).toEqual([]);

    await page.clock.runFor(SETTLED_MS);
    await expect(
      page.getByRole("heading", { name: "Message 1", exact: true }),
    ).toBeVisible();
    await page.keyboard.press("ArrowDown");
    await page.clock.runFor(SETTLED_MS);
    await expect(
      page.getByRole("heading", { name: "Message 2", exact: true }),
    ).toBeVisible();
    expect(errors).toEqual([]);
  });
}
