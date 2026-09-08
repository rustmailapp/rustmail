import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { debounced } from "./reactive";

const DELAY_MS = 100;

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("debounced", () => {
  it("starts on the source's current value", () => {
    createRoot((dispose) => {
      const [source] = createSignal("a");

      expect(debounced(source, DELAY_MS)()).toBe("a");

      dispose();
    });
  });

  it("holds the old value until the delay elapses", async () => {
    await createRoot(async (dispose) => {
      const [source, setSource] = createSignal("a");
      const settled = debounced(source, DELAY_MS);
      await Promise.resolve();

      setSource("b");
      expect(settled()).toBe("a");

      await vi.advanceTimersByTimeAsync(DELAY_MS);
      expect(settled()).toBe("b");

      dispose();
    });
  });

  it("skips values the source only passes through", async () => {
    await createRoot(async () => {
      const [source, setSource] = createSignal(0);
      const settled = debounced(source, DELAY_MS);
      await Promise.resolve();

      for (const step of [1, 2, 3, 4, 5]) {
        setSource(step);
        await vi.advanceTimersByTimeAsync(DELAY_MS / 2);
      }
      expect(settled()).toBe(0);

      await vi.advanceTimersByTimeAsync(DELAY_MS);

      expect(settled()).toBe(5);
    });
  });

  it("drops a pending value when the owner goes away", async () => {
    const [source, setSource] = createSignal("a");
    const settled = await createRoot(async (dispose) => {
      const accessor = debounced(source, DELAY_MS);
      await Promise.resolve();
      setSource("b");
      dispose();
      return accessor;
    });

    await vi.advanceTimersByTimeAsync(DELAY_MS * 2);

    expect(settled()).toBe("a");
  });
});
