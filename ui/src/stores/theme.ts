import { createSignal } from "solid-js";

type Theme = "dark" | "light" | "system";

const STORAGE_KEY = "rustmail-theme";
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

function getInitialTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "light" || stored === "dark" || stored === "system") {
    return stored;
  }
  return "system";
}

const [theme, setThemeSignal] = createSignal<Theme>(getInitialTheme());

function resolve(t: Theme): "dark" | "light" {
  if (t === "system") return darkQuery.matches ? "dark" : "light";
  return t;
}

function applyTheme(t: Theme) {
  document.documentElement.classList.toggle("dark", resolve(t) === "dark");
}

applyTheme(theme());

darkQuery.addEventListener("change", () => {
  if (theme() === "system") applyTheme("system");
});

function setTheme(t: Theme) {
  setThemeSignal(t);
  localStorage.setItem(STORAGE_KEY, t);
  applyTheme(t);
}

export { theme, setTheme };
