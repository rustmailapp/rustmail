import { describe, expect, it } from "vitest";
import { setSelectedId } from "./messages";
import { detailsDrawerOpen, toggleDetails } from "./layout";

describe("details drawer", () => {
  it("closes when the selection goes away and stays closed after", () => {
    setSelectedId("msg-1");
    toggleDetails();
    expect(detailsDrawerOpen()).toBe(true);

    setSelectedId(null);
    expect(detailsDrawerOpen()).toBe(false);

    setSelectedId("msg-2");
    expect(detailsDrawerOpen()).toBe(false);
  });

  it("stays open while the selection moves between messages", () => {
    setSelectedId("msg-1");
    toggleDetails();
    expect(detailsDrawerOpen()).toBe(true);

    setSelectedId("msg-2");
    expect(detailsDrawerOpen()).toBe(true);

    setSelectedId(null);
  });
});
