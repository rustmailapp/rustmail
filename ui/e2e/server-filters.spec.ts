import { expect, test } from "@playwright/test";
import { MAX_TAG_FILTERS, mockInbox } from "./inbox-fixture";

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

test("the tag filter stops at the most tags the server accepts", async ({
  page,
}) => {
  const tagCount = MAX_TAG_FILTERS + 1;
  const backend = await mockInbox(page, tagCount, (index) => ({
    tags: [`tag-${String(index).padStart(2, "0")}`],
  }));
  await page.goto("/");
  await expect(page.locator('[role="option"]').first()).toBeVisible();

  await page.getByRole("button", { name: "Tags" }).click();
  const tagButtons = page.getByRole("button", { name: /^tag-\d+$/ });
  await expect(tagButtons).toHaveCount(tagCount);
  for (let i = 0; i < MAX_TAG_FILTERS; i += 1) {
    await tagButtons.nth(i).click();
  }

  await expect(tagButtons.nth(MAX_TAG_FILTERS)).toBeDisabled();
  await expect(page.getByText(`${MAX_TAG_FILTERS} matches`)).toBeVisible();
  for (const read of backend.calls.listed) {
    expect(new URLSearchParams(read).getAll("tag").length).toBeLessThanOrEqual(
      MAX_TAG_FILTERS,
    );
  }
});
