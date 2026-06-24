import { createSignal } from "solid-js";

const STORAGE_KEY = "rustmail-rusted";

function getInitialRusted(): boolean {
  return localStorage.getItem(STORAGE_KEY) === "true";
}

const [rusted, setRustedSignal] = createSignal<boolean>(getInitialRusted());

function applyRusted(on: boolean) {
  const root = document.documentElement;
  root.classList.add("theme-switching");
  root.classList.toggle("rusted", on);
  void root.offsetHeight;
  root.classList.remove("theme-switching");
}

applyRusted(rusted());

function setRusted(on: boolean) {
  setRustedSignal(on);
  localStorage.setItem(STORAGE_KEY, String(on));
  applyRusted(on);
}

export { rusted, setRusted };
