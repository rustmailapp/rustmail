import { expect, test } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

test("a confirmed deletion stays removed after another page loads", async ({
  page,
}) => {
  await mockInbox(page, 600);
  let releaseDelete = () => {};
  const pendingDelete = new Promise<void>((resolve) => {
    releaseDelete = resolve;
  });
  let deleting = false;
  await page.route(/\/api\/v1\/messages\/msg-0000$/, async (route) => {
    if (route.request().method() !== "DELETE") return route.fallback();
    deleting = true;
    await pendingDelete;
    return route.fallback();
  });
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.clock.install();
  await page.goto("/");
  await expect(
    page.locator('[role="option"][aria-selected="true"]'),
  ).toHaveAttribute("data-id", "msg-0000");
  const inbox = page.getByRole("listbox", { name: "Messages" });
  await inbox.focus();
  await page.keyboard.press("d");
  await page.clock.runFor(5000);
  await expect.poll(() => deleting).toBe(true);

  const nextPage = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return (
      url.pathname === "/api/v1/messages" &&
      url.searchParams.get("offset") === "100"
    );
  });
  await page.keyboard.press("End");
  await nextPage;
  await expect(inbox).toHaveAttribute("aria-busy", "false");

  releaseDelete();
  await expect(page.locator("header")).toContainText("599 messages");
  await page.keyboard.press("Home");
  await expect(
    page.locator('[role="option"][aria-selected="true"]'),
  ).toHaveAttribute("data-id", "msg-0001");
  await expect(page.locator('[role="option"][data-id="msg-0000"]')).toHaveCount(
    0,
  );
  expect(errors).toEqual([]);
});
