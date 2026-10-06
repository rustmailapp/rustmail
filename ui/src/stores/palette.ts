import { createSignal } from "solid-js";

/** Every palette the app ships, in picker order. */
const PALETTES = [
  { id: "rustmail", label: "RustMail" },
  { id: "ember", label: "Ember" },
  { id: "copper", label: "Copper & Teal" },
  { id: "dawn", label: "Dawn" },
  { id: "classic", label: "Classic" },
] as const;

type Palette = (typeof PALETTES)[number]["id"];

const STORAGE_KEY = "rustmail-palette";
const DEFAULT_PALETTE: Palette = "rustmail";

function isPalette(value: string | null): value is Palette {
  return PALETTES.some((palette) => palette.id === value);
}

function getInitialPalette(): Palette {
  const stored = localStorage.getItem(STORAGE_KEY);
  return isPalette(stored) ? stored : DEFAULT_PALETTE;
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
