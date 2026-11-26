import { createSignal } from "solid-js";

export type SettingsTab = "appearance" | "layout";

const [settingsOpen, setSettingsOpen] = createSignal(false);
const [settingsTab, setSettingsTab] = createSignal<SettingsTab>("appearance");

function toggleSettings() {
  setSettingsOpen(!settingsOpen());
}

export {
  settingsOpen,
  setSettingsOpen,
  settingsTab,
  setSettingsTab,
  toggleSettings,
};
