import { afterEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "rustmail-density";

let storage: Map<string, string>;

async function loadStore(stored: string | null) {
  storage = new Map(stored === null ? [] : [[STORAGE_KEY, stored]]);
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.resetModules();
  return import("./density");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("density", () => {
  it("defaults to comfortable when nothing is stored", async () => {
    const { density } = await loadStore(null);

    expect(density()).toBe("comfortable");
  });

  it("falls back to comfortable for an unknown stored value", async () => {
    const { density } = await loadStore("cozy");

    expect(density()).toBe("comfortable");
  });

  it("restores a stored density", async () => {
    const { density } = await loadStore("compact");

    expect(density()).toBe("compact");
  });

  it("persists a new density", async () => {
    const { density, setDensity } = await loadStore(null);

    setDensity("compact");

    expect(density()).toBe("compact");
    expect(storage.get(STORAGE_KEY)).toBe("compact");
  });

  it("toggles between the two densities and persists each step", async () => {
    const { density, toggleDensity } = await loadStore(null);

    toggleDensity();
    expect(density()).toBe("compact");
    expect(storage.get(STORAGE_KEY)).toBe("compact");

    toggleDensity();
    expect(density()).toBe("comfortable");
    expect(storage.get(STORAGE_KEY)).toBe("comfortable");
  });
});
