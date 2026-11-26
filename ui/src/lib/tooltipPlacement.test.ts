import { describe, expect, it } from "vitest";
import {
  TOOLTIP_EDGE_MARGIN_PX,
  TOOLTIP_GAP_PX,
  placeTooltip,
} from "./tooltipPlacement";

const VIEWPORT = { width: 1000, height: 800 };
const TOOLTIP = { width: 120, height: 24 };

function trigger(left: number, top: number, width = 28, height = 28) {
  return { left, top, width, bottom: top + height };
}

describe("placeTooltip", () => {
  it("centres the tooltip below the trigger when there is room", () => {
    const placement = placeTooltip(trigger(500, 100), TOOLTIP, VIEWPORT);

    expect(placement).toEqual({
      side: "below",
      top: 128 + TOOLTIP_GAP_PX,
      left: 514 - 60,
    });
  });

  it("flips above a trigger near the bottom edge", () => {
    const placement = placeTooltip(trigger(500, 770), TOOLTIP, VIEWPORT);

    expect(placement.side).toBe("above");
    expect(placement.top).toBe(770 - TOOLTIP_GAP_PX - TOOLTIP.height);
  });

  it("stays below when neither side has room", () => {
    const placement = placeTooltip(trigger(500, 10, 28, 780), TOOLTIP, {
      width: 1000,
      height: 800,
    });

    expect(placement.side).toBe("below");
  });

  it("slides left of a trigger at the right edge", () => {
    const placement = placeTooltip(trigger(980, 100, 16), TOOLTIP, VIEWPORT);

    expect(placement.left).toBe(
      VIEWPORT.width - TOOLTIP_EDGE_MARGIN_PX - TOOLTIP.width,
    );
  });

  it("slides right of a trigger at the left edge", () => {
    const placement = placeTooltip(trigger(0, 100, 16), TOOLTIP, VIEWPORT);

    expect(placement.left).toBe(TOOLTIP_EDGE_MARGIN_PX);
  });
});
