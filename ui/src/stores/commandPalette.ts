import { createSignal } from "solid-js";

const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);
const [shortcutsOpen, setShortcutsOpen] = createSignal(false);

function toggleCommandPalette(): void {
  setCommandPaletteOpen((open) => !open);
}

export {
  commandPaletteOpen,
  setCommandPaletteOpen,
  toggleCommandPalette,
  shortcutsOpen,
  setShortcutsOpen,
};
