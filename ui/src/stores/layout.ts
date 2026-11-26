import { createEffect, createRoot, createSignal, on } from "solid-js";
import { selectedId } from "./messages";

/**
 * The viewport width from which the details rail gets a column of its own.
 *
 * Below it the three columns leave the message too little room to read, so
 * the rail becomes a drawer over the message body instead.
 */
const WIDE_LAYOUT_QUERY = "(min-width: 1300px)";

const RAIL_STORAGE_KEY = "rustmail-details-rail";
const RAIL_COLLAPSED = "collapsed";
const RAIL_EXPANDED = "expanded";

/** Whether the rail starts collapsed; anything but a stored collapse expands it. */
function getInitialRailCollapsed(): boolean {
  return localStorage.getItem(RAIL_STORAGE_KEY) === RAIL_COLLAPSED;
}

const wideQuery =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(WIDE_LAYOUT_QUERY)
    : null;
const [wideLayout, setWideLayout] = createSignal(wideQuery?.matches ?? false);
const [drawerOpen, setDrawerOpen] = createSignal(false);
const [railCollapsed, setRailCollapsed] = createSignal(
  getInitialRailCollapsed(),
);

wideQuery?.addEventListener("change", (event) => {
  setWideLayout(event.matches);
  if (event.matches) setDrawerOpen(false);
});

createRoot(() =>
  createEffect(
    on(selectedId, (id) => {
      if (id === null) setDrawerOpen(false);
    }),
  ),
);

/** Whether the details drawer is over the message body right now. */
function detailsDrawerOpen(): boolean {
  return !wideLayout() && drawerOpen();
}

/** Whether the details rail has its own column beside the message right now. */
function detailsRailOpen(): boolean {
  return wideLayout() && !railCollapsed();
}

function toggleRail(): void {
  const collapsed = !railCollapsed();
  setRailCollapsed(collapsed);
  localStorage.setItem(
    RAIL_STORAGE_KEY,
    collapsed ? RAIL_COLLAPSED : RAIL_EXPANDED,
  );
}

/**
 * Shows or hides the message details in the current layout.
 *
 * The wide layout collapses or expands the rail, and remembers it across
 * reloads; the narrow layout opens or closes the drawer, which is never kept.
 */
function toggleDetails(): void {
  if (wideLayout()) {
    toggleRail();
    return;
  }
  setDrawerOpen((open) => !open);
}

function closeDetails(): void {
  setDrawerOpen(false);
}

export {
  wideLayout,
  detailsDrawerOpen,
  detailsRailOpen,
  toggleDetails,
  closeDetails,
};
