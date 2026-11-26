/** Gap between the trigger and the tooltip, in pixels. */
export const TOOLTIP_GAP_PX = 6;
/** Closest the tooltip may come to a viewport edge, in pixels. */
export const TOOLTIP_EDGE_MARGIN_PX = 8;

export interface TriggerRect {
  top: number;
  bottom: number;
  left: number;
  width: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface TooltipPlacement {
  top: number;
  left: number;
  side: "below" | "above";
}

/**
 * Where a tooltip goes for a trigger: centred below it, flipped above when
 * the bottom edge has no room, and slid sideways so it never crosses the
 * left or right edge of the viewport.
 */
export function placeTooltip(
  trigger: TriggerRect,
  tooltip: Size,
  viewport: Size,
): TooltipPlacement {
  const below = trigger.bottom + TOOLTIP_GAP_PX;
  const fitsBelow =
    below + tooltip.height <= viewport.height - TOOLTIP_EDGE_MARGIN_PX;
  const above = trigger.top - TOOLTIP_GAP_PX - tooltip.height;
  const side = fitsBelow || above < TOOLTIP_EDGE_MARGIN_PX ? "below" : "above";

  const centred = trigger.left + trigger.width / 2 - tooltip.width / 2;
  const maxLeft = viewport.width - TOOLTIP_EDGE_MARGIN_PX - tooltip.width;
  const left = Math.max(TOOLTIP_EDGE_MARGIN_PX, Math.min(centred, maxLeft));

  return { top: side === "below" ? below : above, left, side };
}
