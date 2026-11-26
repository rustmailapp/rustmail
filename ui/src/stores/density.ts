import { createSignal } from "solid-js";

/** Every inbox density, in picker order. */
const DENSITIES = [
  { id: "comfortable", label: "Comfortable" },
  { id: "compact", label: "Compact" },
] as const;

type Density = (typeof DENSITIES)[number]["id"];

const STORAGE_KEY = "rustmail-density";
const DEFAULT_DENSITY: Density = "comfortable";

function isDensity(value: string | null): value is Density {
  return DENSITIES.some((density) => density.id === value);
}

function getInitialDensity(): Density {
  const stored = localStorage.getItem(STORAGE_KEY);
  return isDensity(stored) ? stored : DEFAULT_DENSITY;
}

const [density, setDensitySignal] = createSignal<Density>(getInitialDensity());

/** Sets the inbox density and remembers the choice. */
function setDensity(next: Density) {
  setDensitySignal(next);
  localStorage.setItem(STORAGE_KEY, next);
}

/** Switches between Comfortable and Compact and remembers the choice. */
function toggleDensity() {
  setDensity(density() === "compact" ? "comfortable" : "compact");
}

export { DENSITIES, density, setDensity, toggleDensity };
export type { Density };
