import { createSignal } from "solid-js";

/** Every palette the app ships, in picker order. */
const PALETTES = [
  { id: "rustmail", label: "RustMail" },
  { id: "graphite", label: "Graphite" },
  { id: "tide", label: "Tide" },
] as const;

type Palette = (typeof PALETTES)[number]["id"];

/**
 * Retired palette ids and the palette that replaces each one. The boot script
 * in `public/theme-boot.js` applies the same map before the bundle loads.
 */
const RETIRED_PALETTES: Readonly<Record<string, Palette>> = {
  copper: "tide",
  classic: "graphite",
  ember: "graphite",
  dawn: "rustmail",
};

const STORAGE_KEY = "rustmail-palette";
const DEFAULT_PALETTE: Palette = "rustmail";

function isPalette(value: string | null): value is Palette {
  return PALETTES.some((palette) => palette.id === value);
}

function getInitialPalette(): Palette {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (isPalette(stored)) return stored;
  if (stored !== null && Object.hasOwn(RETIRED_PALETTES, stored)) {
    const replacement = RETIRED_PALETTES[stored];
    localStorage.setItem(STORAGE_KEY, replacement);
    return replacement;
  }
  return DEFAULT_PALETTE;
}

const [palette, setPaletteSignal] = createSignal<Palette>(getInitialPalette());

function applyPalette(p: Palette) {
  const root = document.documentElement;
  root.classList.add("theme-switching");
  root.dataset.palette = p;
  void root.offsetHeight;
  root.classList.remove("theme-switching");
}

applyPalette(palette());

function setPalette(p: Palette) {
  setPaletteSignal(p);
  localStorage.setItem(STORAGE_KEY, p);
  applyPalette(p);
}

export { PALETTES, DEFAULT_PALETTE, palette, setPalette };
export type { Palette };
