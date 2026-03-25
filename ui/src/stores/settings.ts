import { createSignal } from "solid-js";

export type SettingsTab = "appearance";

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
