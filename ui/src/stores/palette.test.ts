import { afterEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "rustmail-palette";

let storage: Map<string, string>;
let root: {
  classList: Set<string>;
  dataset: Record<string, string>;
  added: string[];
};

function stubBrowser(stored: string | null): void {
  storage = new Map(stored === null ? [] : [[STORAGE_KEY, stored]]);
  const classList = new Set<string>();
  const added: string[] = [];
  root = { classList, dataset: {}, added };
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.stubGlobal("document", {
    documentElement: {
      dataset: root.dataset,
      offsetHeight: 0,
      classList: {
        add: (name: string) => {
          added.push(name);
          classList.add(name);
        },
        remove: (name: string) => classList.delete(name),
      },
    },
  });
}

async function loadStore(stored: string | null) {
  stubBrowser(stored);
  vi.resetModules();
  return import("./palette");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("palette", () => {
  it("defaults to rustmail when nothing is stored", async () => {
    const { palette } = await loadStore(null);

    expect(palette()).toBe("rustmail");
    expect(root.dataset.palette).toBe("rustmail");
  });

  it("falls back to rustmail for an unknown stored value", async () => {
    const { palette } = await loadStore("neon");

    expect(palette()).toBe("rustmail");
    expect(root.dataset.palette).toBe("rustmail");
  });

  it("restores a stored palette", async () => {
    const { palette } = await loadStore("tide");

    expect(palette()).toBe("tide");
    expect(root.dataset.palette).toBe("tide");
  });

  it.each([
    ["copper", "tide"],
    ["classic", "graphite"],
    ["ember", "graphite"],
    ["dawn", "rustmail"],
  ])("migrates the retired %s palette to %s", async (retired, replacement) => {
    const { palette } = await loadStore(retired);

    expect(palette()).toBe(replacement);
    expect(root.dataset.palette).toBe(replacement);
    expect(storage.get(STORAGE_KEY)).toBe(replacement);
  });

  it("leaves an unknown stored value in storage", async () => {
    await loadStore("neon");

    expect(storage.get(STORAGE_KEY)).toBe("neon");
  });

  it("persists and applies a new palette", async () => {
    const { palette, setPalette } = await loadStore(null);

    setPalette("graphite");

    expect(palette()).toBe("graphite");
    expect(storage.get(STORAGE_KEY)).toBe("graphite");
    expect(root.dataset.palette).toBe("graphite");
  });

  it("suppresses transitions only for the switch itself", async () => {
    const { setPalette } = await loadStore(null);

    root.added.length = 0;
    setPalette("tide");

    expect(root.added).toContain("theme-switching");
    expect(root.classList.has("theme-switching")).toBe(false);
  });
});
