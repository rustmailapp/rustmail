import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "rustmail-rusted";

let storage: Map<string, string>;
let rootClasses: Set<string>;

async function loadStore() {
  storage = new Map();
  rootClasses = new Set();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.stubGlobal("document", {
    documentElement: {
      offsetHeight: 0,
      classList: {
        add: (name: string) => rootClasses.add(name),
        remove: (name: string) => rootClasses.delete(name),
        toggle: (name: string, on: boolean) =>
          on ? rootClasses.add(name) : rootClasses.delete(name),
      },
    },
  });
  vi.resetModules();
  return import("./rusted");
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("registerLogoClick", () => {
  it("toggles rusted on the fifth click inside the window", async () => {
    const { OXIDIZE_CLICKS, registerLogoClick, rusted, rustedToast } =
      await loadStore();

    for (let i = 0; i < OXIDIZE_CLICKS; i++) {
      registerLogoClick();
      vi.advanceTimersByTime(100);
    }

    expect(rusted()).toBe(true);
    expect(rootClasses.has("rusted")).toBe(true);
    expect(storage.get(STORAGE_KEY)).toBe("true");
    expect(rustedToast()).toBe("Oxidized");
  });

  it("does nothing on four clicks", async () => {
    const { OXIDIZE_CLICKS, registerLogoClick, rusted } = await loadStore();

    for (let i = 0; i < OXIDIZE_CLICKS - 1; i++) registerLogoClick();

    expect(rusted()).toBe(false);
  });

  it("does not count clicks spread wider than the window", async () => {
    const { OXIDIZE_CLICKS, OXIDIZE_WINDOW_MS, registerLogoClick, rusted } =
      await loadStore();
    const gap = OXIDIZE_WINDOW_MS / (OXIDIZE_CLICKS - 1);

    for (let i = 0; i < OXIDIZE_CLICKS; i++) {
      registerLogoClick();
      vi.advanceTimersByTime(gap);
    }

    expect(rusted()).toBe(false);
  });

  it("toggles back off and says so", async () => {
    const { OXIDIZE_CLICKS, registerLogoClick, rusted, rustedToast } =
      await loadStore();

    for (let i = 0; i < OXIDIZE_CLICKS * 2; i++) registerLogoClick();

    expect(rusted()).toBe(false);
    expect(storage.get(STORAGE_KEY)).toBe("false");
    expect(rustedToast()).toBe("Polished");
  });

  it("takes the toast down after its dwell", async () => {
    const { OXIDIZE_CLICKS, RUSTED_TOAST_MS, registerLogoClick, rustedToast } =
      await loadStore();

    for (let i = 0; i < OXIDIZE_CLICKS; i++) registerLogoClick();
    vi.advanceTimersByTime(RUSTED_TOAST_MS - 1);
    expect(rustedToast()).toBe("Oxidized");

    vi.advanceTimersByTime(1);
    expect(rustedToast()).toBeNull();
  });
});

describe("setRusted", () => {
  it("turns rusted off and remembers it", async () => {
    const { OXIDIZE_CLICKS, registerLogoClick, rusted, setRusted } =
      await loadStore();
    for (let i = 0; i < OXIDIZE_CLICKS; i++) registerLogoClick();

    setRusted(false);

    expect(rusted()).toBe(false);
    expect(rootClasses.has("rusted")).toBe(false);
    expect(storage.get(STORAGE_KEY)).toBe("false");
  });
});
