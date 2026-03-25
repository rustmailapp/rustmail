import { createSignal } from "solid-js";

type Theme = "dark" | "light";

function getInitialTheme(): Theme {
  const stored = localStorage.getItem("rustmail-theme");
  if (stored === "light" || stored === "dark") return stored;
  return "dark";
}

const [theme, setThemeSignal] = createSignal<Theme>(getInitialTheme());

function applyTheme(t: Theme) {
  document.documentElement.classList.toggle("dark", t === "dark");
}

applyTheme(theme());

function setTheme(t: Theme) {
  setThemeSignal(t);
  localStorage.setItem("rustmail-theme", t);
  applyTheme(t);
}

function toggleTheme() {
  setTheme(theme() === "dark" ? "light" : "dark");
}

export { theme, setTheme, toggleTheme };
