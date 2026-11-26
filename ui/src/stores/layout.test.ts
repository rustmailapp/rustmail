import { afterEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "rustmail-details-rail";

let storage: Map<string, string>;
let crossBoundary: (wide: boolean) => void;

/** Loads fresh layout and selection stores at the given width and storage. */
async function loadStores(options: { wide: boolean; stored?: string }) {
  storage = new Map(
    options.stored === undefined ? [] : [[STORAGE_KEY, options.stored]],
  );
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.stubGlobal("window", {
    matchMedia: () => ({
      matches: options.wide,
      addEventListener: (
        _type: string,
        listener: (event: { matches: boolean }) => void,
      ) => {
        crossBoundary = (wide) => listener({ matches: wide });
      },
    }),
  });
  vi.resetModules();
  const messages = await import("./messages");
  const layout = await import("./layout");
  return { ...messages, ...layout };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("details drawer", () => {
  it("closes when the selection goes away and stays closed after", async () => {
    const { setSelectedId, toggleDetails, detailsDrawerOpen } =
      await loadStores({ wide: false });
    setSelectedId("msg-1");
    toggleDetails();
    expect(detailsDrawerOpen()).toBe(true);

    setSelectedId(null);
    expect(detailsDrawerOpen()).toBe(false);

    setSelectedId("msg-2");
    expect(detailsDrawerOpen()).toBe(false);
  });

  it("stays open while the selection moves between messages", async () => {
    const { setSelectedId, toggleDetails, detailsDrawerOpen } =
      await loadStores({ wide: false });
    setSelectedId("msg-1");
    toggleDetails();
    expect(detailsDrawerOpen()).toBe(true);

    setSelectedId("msg-2");
    expect(detailsDrawerOpen()).toBe(true);
  });

  it("is never persisted", async () => {
    const { setSelectedId, toggleDetails } = await loadStores({ wide: false });
    setSelectedId("msg-1");

    toggleDetails();

    expect(storage.size).toBe(0);
  });
});

describe("details rail", () => {
  it("starts expanded when nothing is stored", async () => {
    const { detailsRailOpen } = await loadStores({ wide: true });

    expect(detailsRailOpen()).toBe(true);
  });

  it("starts expanded for an unknown stored value", async () => {
    const { detailsRailOpen } = await loadStores({
      wide: true,
      stored: "hidden",
    });

    expect(detailsRailOpen()).toBe(true);
  });

  it("restores a stored collapse", async () => {
    const { detailsRailOpen } = await loadStores({
      wide: true,
      stored: "collapsed",
    });

    expect(detailsRailOpen()).toBe(false);
  });

  it("persists each toggle", async () => {
    const { toggleDetails, detailsRailOpen } = await loadStores({ wide: true });

    toggleDetails();
    expect(detailsRailOpen()).toBe(false);
    expect(storage.get(STORAGE_KEY)).toBe("collapsed");

    toggleDetails();
    expect(detailsRailOpen()).toBe(true);
    expect(storage.get(STORAGE_KEY)).toBe("expanded");
  });

  it("stays collapsed when the selection goes away", async () => {
    const { setSelectedId, toggleDetails, detailsRailOpen } = await loadStores({
      wide: true,
    });
    setSelectedId("msg-1");
    toggleDetails();

    setSelectedId(null);

    expect(detailsRailOpen()).toBe(false);
  });
});

describe("crossing the wide layout boundary", () => {
  it("does not open the drawer from a collapsed rail", async () => {
    const { toggleDetails, detailsDrawerOpen, detailsRailOpen } =
      await loadStores({ wide: true });
    toggleDetails();

    crossBoundary(false);

    expect(detailsDrawerOpen()).toBe(false);
    expect(detailsRailOpen()).toBe(false);
  });

  it("does not collapse the rail from an open drawer", async () => {
    const { setSelectedId, toggleDetails, detailsDrawerOpen, detailsRailOpen } =
      await loadStores({ wide: false });
    setSelectedId("msg-1");
    toggleDetails();

    crossBoundary(true);

    expect(detailsRailOpen()).toBe(true);
    expect(detailsDrawerOpen()).toBe(false);
    expect(storage.size).toBe(0);
  });

  it("keeps the collapse for the next wide stretch", async () => {
    const { toggleDetails, detailsRailOpen } = await loadStores({ wide: true });
    toggleDetails();

    crossBoundary(false);
    crossBoundary(true);

    expect(detailsRailOpen()).toBe(false);
  });

  it("leaves the drawer closed when coming back narrow", async () => {
    const { setSelectedId, toggleDetails, detailsDrawerOpen } =
      await loadStores({ wide: false });
    setSelectedId("msg-1");
    toggleDetails();

    crossBoundary(true);
    crossBoundary(false);

    expect(detailsDrawerOpen()).toBe(false);
  });
});
