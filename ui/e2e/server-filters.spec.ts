import { expect, test } from "@playwright/test";
import { mockInbox } from "./inbox-fixture";

const LARGE_INBOX = 100_000;
/** One message in this many is starred, so the filter is sparse. */
const STARRED_EVERY = 2_000;

test("a starred filter over a large inbox takes one list read", async ({
  page,
}) => {
  const backend = await mockInbox(page, LARGE_INBOX, (index) => ({
    is_starred: index % STARRED_EVERY === 0,
  }));
  await page.goto("/");
  const inbox = page.getByRole("listbox", { name: "Messages" });
  await expect(page.locator('[role="option"]').first()).toBeVisible();
  const readsBefore = backend.calls.listed.length;

  await page.getByRole("button", { name: "Starred" }).click();
  await expect(
    page.getByText(`${LARGE_INBOX / STARRED_EVERY} matches`),
  ).toBeVisible();
  await inbox.focus();
  await page.keyboard.press("End");
  await expect(inbox).toHaveAttribute("aria-busy", "false");

  const reads = backend.calls.listed.slice(readsBefore);
  expect(reads).toHaveLength(1);
  expect(new URLSearchParams(reads[0]).get("starred")).toBe("true");
});
