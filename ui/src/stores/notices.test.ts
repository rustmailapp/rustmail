import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MAX_NOTICES,
  NOTICE_TTL_MS,
  dismissNotice,
  notices,
  notify,
} from "./notices";

function texts(): string[] {
  return notices().map((notice) => notice.text);
}

beforeEach(() => {
  vi.useFakeTimers();
  for (const notice of notices()) dismissNotice(notice.id);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("notify", () => {
  it("puts a notice on screen", () => {
    notify("Could not delete the message.");

    expect(texts()).toEqual(["Could not delete the message."]);
  });

  it("takes it down once its dwell runs out", async () => {
    notify("Could not delete the message.");

    await vi.advanceTimersByTimeAsync(NOTICE_TTL_MS - 1);
    expect(texts()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(texts()).toEqual([]);
  });

  it("keeps distinct notices side by side, newest last", () => {
    notify("first");
    notify("second");

    expect(texts()).toEqual(["first", "second"]);
  });

  it("restarts the dwell of a repeat rather than stacking it", async () => {
    notify("Could not mark the message as read.");
    await vi.advanceTimersByTimeAsync(NOTICE_TTL_MS - 1);

    notify("Could not mark the message as read.");
    await vi.advanceTimersByTimeAsync(NOTICE_TTL_MS - 1);

    expect(texts()).toEqual(["Could not mark the message as read."]);
    await vi.advanceTimersByTimeAsync(1);
    expect(texts()).toEqual([]);
  });

  it("holds no more than the cap, dropping the oldest", () => {
    for (let i = 0; i <= MAX_NOTICES; i++) notify(`notice ${i}`);

    expect(texts()).toHaveLength(MAX_NOTICES);
    expect(texts()).not.toContain("notice 0");
    expect(texts().at(-1)).toBe(`notice ${MAX_NOTICES}`);
  });

  it("leaves no dwell running for a notice it dropped", () => {
    for (let i = 0; i <= MAX_NOTICES; i++) notify(`notice ${i}`);

    expect(vi.getTimerCount()).toBe(MAX_NOTICES);
  });
});

describe("dismissNotice", () => {
  it("takes one notice down and leaves the rest", () => {
    notify("first");
    notify("second");
    const first = notices()[0];
    expect(first).toBeDefined();
    if (!first) return;

    dismissNotice(first.id);

    expect(texts()).toEqual(["second"]);
  });

  it("stops the dwell of the notice it dismissed", () => {
    notify("only");
    const only = notices()[0];
    expect(only).toBeDefined();
    if (!only) return;

    dismissNotice(only.id);

    expect(vi.getTimerCount()).toBe(0);
  });
});
