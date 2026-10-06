import { afterEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "rustmail-preview-width";

let storage: Map<string, string>;

async function loadStore(stored: string | null) {
  storage = new Map(stored === null ? [] : [[STORAGE_KEY, stored]]);
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.resetModules();
  return import("./previewWidth");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("preview width", () => {
  it("defaults to desktop when nothing is stored", async () => {
    const { previewWidth } = await loadStore(null);

    expect(previewWidth()).toBe("desktop");
  });

  it("falls back to desktop for an unknown stored value", async () => {
    const { previewWidth } = await loadStore("tablet");

    expect(previewWidth()).toBe("desktop");
  });

  it("restores a stored width", async () => {
    const { previewWidth } = await loadStore("mobile");

    expect(previewWidth()).toBe("mobile");
  });

  it("persists a new width", async () => {
    const { previewWidth, setPreviewWidth } = await loadStore(null);

    setPreviewWidth("mobile");

    expect(previewWidth()).toBe("mobile");
    expect(storage.get(STORAGE_KEY)).toBe("mobile");
  });
});
