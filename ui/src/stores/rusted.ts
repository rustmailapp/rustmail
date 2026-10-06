import { createSignal } from "solid-js";

const STORAGE_KEY = "rustmail-rusted";

/** Logo clicks it takes to toggle rusted. */
const OXIDIZE_CLICKS = 5;

/** How close together those clicks must land, oldest to newest. */
const OXIDIZE_WINDOW_MS = 2000;

/** How long the "Oxidized" / "Polished" toast stays up. */
const RUSTED_TOAST_MS = 1500;

function getInitialRusted(): boolean {
  return localStorage.getItem(STORAGE_KEY) === "true";
}

const [rusted, setRustedSignal] = createSignal<boolean>(getInitialRusted());
const [rustedToast, setRustedToast] = createSignal<string | null>(null);
let toastDwell: ReturnType<typeof setTimeout> | undefined;
let logoClicks: number[] = [];

function applyRusted(on: boolean) {
  const root = document.documentElement;
  root.classList.add("theme-switching");
  root.classList.toggle("rusted", on);
  void root.offsetHeight;
  root.classList.remove("theme-switching");
}

applyRusted(rusted());

/** Turns rusted on or off and remembers the choice. */
function setRusted(on: boolean) {
  setRustedSignal(on);
  localStorage.setItem(STORAGE_KEY, String(on));
  applyRusted(on);
}

function showRustedToast(text: string) {
  clearTimeout(toastDwell);
  setRustedToast(text);
  toastDwell = setTimeout(() => setRustedToast(null), RUSTED_TOAST_MS);
}

/**
 * Counts a click on the header logo.
 *
 * The fifth click inside the window toggles rusted and starts the count over;
 * clicks that fell out of the window no longer count toward it.
 */
function registerLogoClick() {
  const now = Date.now();
  logoClicks = [
    ...logoClicks.filter((at) => now - at < OXIDIZE_WINDOW_MS),
    now,
  ];
  if (logoClicks.length < OXIDIZE_CLICKS) return;

  logoClicks = [];
  const on = !rusted();
  setRusted(on);
  showRustedToast(on ? "Oxidized" : "Polished");
}

export {
  rusted,
  rustedToast,
  setRusted,
  registerLogoClick,
  OXIDIZE_CLICKS,
  OXIDIZE_WINDOW_MS,
  RUSTED_TOAST_MS,
};
