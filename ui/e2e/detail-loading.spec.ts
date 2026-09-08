import { expect, test, type Page, type Route } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";
import { REQUEST_TIMEOUT_MS } from "../src/lib/api";

const SETTLED_MS = 150;
const BEFORE_SETTLED_MS = 20;
const SELECTED_OPTION = '[role="option"][aria-selected="true"]';
const FIRST_ROW = '[role="option"][data-id="msg-0000"]';
const RAW_BODY = "Subject: Next message\r\n\r\nBody";
const MESSAGE_READ = {
  name: "message",
  suffix: "",
  tab: null,
  label: "this message",
} as const;
const RESOURCES = [
  MESSAGE_READ,
  {
    name: "attachments",
    suffix: "/attachments",
    tab: null,
    label: "attachments",
  },
  { name: "headers", suffix: "/headers", tab: "Headers", label: "headers" },
  {
    name: "auth",
    suffix: "/auth",
    tab: "Auth",
    label: "authentication results",
  },
  { name: "raw", suffix: "/raw", tab: "Raw", label: "the raw source" },
] as const;

type PaneRead = (typeof RESOURCES)[number];

/** Matches the GET the pane issues for one kind of read, of any message. */
function readPattern(suffix: string): RegExp {
  return new RegExp(`/api/v1/messages/[^/]+${suffix}(?:\\?.*)?$`);
}

/**
 * Answers a read the way a healthy backend would.
 *
 * The inbox fixture serves only the endpoints the list needs, so the reads
 * that sit behind the pane's tabs are filled in here.
 */
function answer(read: PaneRead, route: Route): Promise<void> {
  switch (read.name) {
    case "headers":
      return route.fulfill({ json: [] });
    case "auth":
      return route.fulfill({ json: { dkim: [], spf: [], dmarc: [], arc: [] } });
    case "raw":
      return route.fulfill({ body: RAW_BODY });
    default:
      return route.fallback();
  }
}

/**
 * Holds the first matching read open, and answers every later one normally.
 *
 * The returned function lets the held route go. It is abandoned rather than
 * answered: by the time a test releases it the browser has already cancelled
 * the request, which is the whole point of holding it, and a cancelled request
 * has nothing left to reply to.
 */
async function stallFirstRead(page: Page, read: PaneRead): Promise<() => void> {
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let stalling = true;

  await page.route(readPattern(read.suffix), async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    if (!stalling) return answer(read, route);
    stalling = false;
    await held;
    await route.abort("failed").catch(() => {});
  });

  return release;
}

async function openFirstMessage(page: Page): Promise<void> {
  const now = new Date("2026-09-08T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now);
  await page.goto("/");
  await expect(page.locator(SELECTED_OPTION)).toHaveAttribute(
    "aria-posinset",
    "1",
  );
  await page.clock.runFor(SETTLED_MS);
}

for (const resource of RESOURCES) {
  test(`ignores obsolete ${resource.name} errors after keyboard deletion`, async ({
    page,
  }) => {
    await mockInbox(page, 5);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    let releaseRequest = () => {};
    const released = new Promise<void>((resolve) => {
      releaseRequest = resolve;
    });
    let pending = false;
    const targetPath = `/api/v1/messages/msg-0000${resource.suffix}`;

    await page.route(readPattern(resource.suffix), async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      if (new URL(route.request().url()).pathname === targetPath) {
        pending = true;
        await released;
        return route.fulfill({ status: 404, json: {} });
      }
      return answer(resource, route);
    });

    await openFirstMessage(page);
    if (resource.tab) {
      await page
        .getByRole("button", { name: resource.tab, exact: true })
        .click();
    }
    await expect.poll(() => pending).toBe(true);

    await page.getByRole("listbox", { name: "Messages" }).focus();
    await page.keyboard.press("d");
    await expect(page.locator(SELECTED_OPTION)).toHaveAttribute(
      "data-id",
      "msg-0001",
    );
    await expect(page.locator(FIRST_ROW)).toHaveCount(0);
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

for (const resource of RESOURCES) {
  test(`offers a retry once the ${resource.name} read passes its deadline`, async ({
    page,
  }) => {
    await mockInbox(page, 5);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const release = await stallFirstRead(page, resource);
    const notice = page.getByText(`Could not load ${resource.label}.`);

    await openFirstMessage(page);
    if (resource.tab) {
      await page
        .getByRole("button", { name: resource.tab, exact: true })
        .click();
    }
    await expect(notice).toBeHidden();

    await page.clock.runFor(REQUEST_TIMEOUT_MS);
    await expect(notice).toBeVisible();

    release();
    await page
      .getByRole("button", { name: `Retry loading ${resource.label}` })
      .click();
    await expect(notice).toBeHidden();
    expect(errors).toEqual([]);
  });
}

test("renders a message whose raw source is empty", async ({ page }) => {
  await mockInbox(page, 2);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route(readPattern("/raw"), (route) => route.fulfill({ body: "" }));

  await openFirstMessage(page);
  await page.getByRole("button", { name: "Raw", exact: true }).click();

  await expect(page.locator("pre")).toHaveCount(1);
  await expect(page.getByText("Loading...")).toBeHidden();
  expect(errors).toEqual([]);
});

test("cancels a read the selection has already moved past", async ({
  page,
}) => {
  await mockInbox(page, 5);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const release = await stallFirstRead(page, MESSAGE_READ);
  const cancelled = page.waitForEvent(
    "requestfailed",
    (request) =>
      new URL(request.url()).pathname === "/api/v1/messages/msg-0000" &&
      request.method() === "GET",
  );

  await openFirstMessage(page);
  await page.getByRole("listbox", { name: "Messages" }).focus();
  await page.keyboard.press("ArrowDown");
  await page.clock.runFor(SETTLED_MS);

  await cancelled;
  await expect(
    page.getByRole("heading", { name: "Message 1", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
  release();
});
