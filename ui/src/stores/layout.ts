import { createEffect, createRoot, createSignal, on } from "solid-js";
import { selectedId } from "./messages";

/**
 * The viewport width from which the details rail gets a column of its own.
 *
 * Below it the three columns leave the message too little room to read, so
 * the rail becomes a drawer over the message body instead.
 */
const WIDE_LAYOUT_QUERY = "(min-width: 1300px)";

const wideQuery =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(WIDE_LAYOUT_QUERY)
    : null;
const [wideLayout, setWideLayout] = createSignal(wideQuery?.matches ?? false);
const [drawerOpen, setDrawerOpen] = createSignal(false);

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

/** Opens or closes the details drawer; the wide layout has none to toggle. */
function toggleDetails(): void {
  if (wideLayout()) return;
  setDrawerOpen((open) => !open);
}

function closeDetails(): void {
  setDrawerOpen(false);
}

export { wideLayout, detailsDrawerOpen, toggleDetails, closeDetails };
