import { Show, For, createUniqueId, type JSX } from "solid-js";
import { theme, setTheme, type Theme } from "../stores/theme";
import { PALETTES, palette, setPalette } from "../stores/palette";
import { rusted, setRusted } from "../stores/rusted";
import { DENSITIES, density, setDensity } from "../stores/density";
import {
  settingsOpen,
  setSettingsOpen,
  settingsTab,
  setSettingsTab,
} from "../stores/settings";
import type { SettingsTab } from "../stores/settings";
import Modal from "./Modal";
import {
  BarsIcon,
  CloseIcon,
  MonitorIcon,
  MoonIcon,
  PaintBrushIcon,
  SunIcon,
} from "./icons";

type Icon = (props: { class: string }) => JSX.Element;

const TABS: { value: SettingsTab; label: string; Icon: Icon }[] = [
  {
    value: "appearance",
    label: "Appearance",
    Icon: PaintBrushIcon,
  },
  {
    value: "layout",
    label: "Layout",
    Icon: BarsIcon,
  },
];

const THEMES: { value: Theme; label: string; Icon: Icon }[] = [
  { value: "dark", label: "Dark", Icon: MoonIcon },
  { value: "light", label: "Light", Icon: SunIcon },
  { value: "system", label: "System", Icon: MonitorIcon },
];

const SETTINGS_PANEL_ID = "settings-panel";

function tabId(tab: SettingsTab): string {
  return `settings-tab-${tab}`;
}

/** Arrow keys, Home and End move between tabs, selecting as focus moves. */
function handleTabKeyDown(e: KeyboardEvent) {
  const current = TABS.findIndex((tab) => tab.value === settingsTab());
  const last = TABS.length - 1;
  const targets: Record<string, number | undefined> = {
    ArrowRight: current === last ? 0 : current + 1,
    ArrowLeft: current === 0 ? last : current - 1,
    Home: 0,
    End: last,
  };
  const next = TABS[targets[e.key] ?? -1];
  if (!next) return;
  e.preventDefault();
  setSettingsTab(next.value);
  document.getElementById(tabId(next.value))?.focus();
}

export default function Settings() {
  const titleId = createUniqueId();
  return (
    <Modal
      open={settingsOpen()}
      onClose={() => setSettingsOpen(false)}
      labelledBy={titleId}
    >
      <div class="flex items-center justify-between px-6 py-4 border-b border-zinc-200 dark:border-zinc-800">
        <h2 id={titleId} class="text-base font-semibold">
          Settings
        </h2>
        <button
          onClick={() => setSettingsOpen(false)}
          aria-label="Close"
          class="rounded-md p-1.5 text-zinc-500 dark:text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
        >
          <CloseIcon class="size-4" />
        </button>
      </div>

      <div
        role="tablist"
        aria-label="Settings sections"
        onKeyDown={handleTabKeyDown}
        class="flex border-b border-zinc-200 dark:border-zinc-800 px-6 gap-1"
      >
        <For each={TABS}>
          {(tab) => (
            <button
              role="tab"
              id={tabId(tab.value)}
              onClick={() => setSettingsTab(tab.value)}
              aria-selected={settingsTab() === tab.value}
              aria-controls={SETTINGS_PANEL_ID}
              tabIndex={settingsTab() === tab.value ? 0 : -1}
              class={`flex items-center gap-1.5 px-3 py-2.5 text-xs font-medium border-b-2 -mb-px transition cursor-pointer ${
                settingsTab() === tab.value
                  ? "border-orange-500 text-zinc-900 dark:text-zinc-100"
                  : "border-transparent text-zinc-500 dark:text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-300"
              }`}
            >
              <tab.Icon class="size-3.5" />
              {tab.label}
            </button>
          )}
        </For>
      </div>

      <div
        role="tabpanel"
        id={SETTINGS_PANEL_ID}
        aria-labelledby={tabId(settingsTab())}
        class="px-6 py-5 max-h-[60vh] overflow-y-auto"
      >
        <Show when={settingsTab() === "appearance"}>
          <AppearanceTab />
        </Show>
        <Show when={settingsTab() === "layout"}>
          <LayoutTab />
        </Show>
      </div>
    </Modal>
  );
}

function AppearanceTab() {
  return (
    <div>
      <span
        id="theme-label"
        class="text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-2 block"
      >
        Theme
      </span>
      <div
        role="group"
        aria-labelledby="theme-label"
        class="grid grid-cols-3 gap-2"
      >
        <For each={THEMES}>
          {(option) => (
            <button
              onClick={() => setTheme(option.value)}
              aria-pressed={theme() === option.value}
              class={`rounded-lg border px-3 py-2.5 text-xs font-medium transition cursor-pointer flex items-center justify-center gap-2 ${
                theme() === option.value
                  ? "border-orange-500 bg-orange-500/10 text-orange-700 dark:text-orange-500"
                  : "border-zinc-200 dark:border-zinc-700 text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600"
              }`}
            >
              <option.Icon class="size-4" />
              {option.label}
            </button>
          )}
        </For>
      </div>

      <span
        id="palette-label"
        class="text-sm font-medium text-zinc-700 dark:text-zinc-300 mt-6 mb-2 block"
      >
        Palette
      </span>
      <div
        role="group"
        aria-labelledby="palette-label"
        class="grid grid-cols-3 gap-2"
      >
        <For each={PALETTES}>
          {(option) => (
            <button
              onClick={() => setPalette(option.id)}
              aria-pressed={palette() === option.id}
              class={`rounded-lg border p-2 text-xs font-medium transition cursor-pointer flex flex-col gap-2 ${
                palette() === option.id
                  ? "border-orange-500 bg-orange-500/10 text-orange-700 dark:text-orange-500"
                  : "border-zinc-200 dark:border-zinc-700 text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600"
              }`}
            >
              <span
                data-palette={option.id}
                aria-hidden="true"
                class="flex h-5 w-full overflow-hidden rounded-md border border-zinc-200 dark:border-zinc-700"
              >
                <span class="flex-1 bg-zinc-100 dark:bg-zinc-950" />
                <span class="flex-1 bg-white dark:bg-zinc-900" />
                <span class="flex-1 bg-orange-500" />
                <span class="flex-1 bg-zinc-800 dark:bg-zinc-200" />
              </span>
              {option.label}
            </button>
          )}
        </For>
      </div>

      <Show when={rusted()}>
        <label class="text-sm font-medium text-zinc-700 dark:text-zinc-300 mt-6 mb-2 block">
          Easter egg
        </label>
        <div class="rounded-lg border border-orange-500 bg-orange-500/10 px-3 py-2 text-xs font-medium text-orange-700 dark:text-orange-500 flex items-center justify-between gap-2">
          <span>Rusted · On</span>
          <button
            type="button"
            onClick={() => setRusted(false)}
            class="rounded-lg border border-zinc-200 dark:border-zinc-700 px-3 py-1.5 text-xs font-medium text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600 transition cursor-pointer"
          >
            Turn off
          </button>
        </div>
      </Show>
    </div>
  );
}

function LayoutTab() {
  return (
    <div>
      <span
        id="density-label"
        class="text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-2 block"
      >
        Density
      </span>
      <div
        role="group"
        aria-labelledby="density-label"
        class="grid grid-cols-2 gap-2"
      >
        <For each={DENSITIES}>
          {(option) => (
            <button
              onClick={() => setDensity(option.id)}
              aria-pressed={density() === option.id}
              class={`rounded-lg border px-3 py-2.5 text-xs font-medium transition cursor-pointer ${
                density() === option.id
                  ? "border-orange-500 bg-orange-500/10 text-orange-700 dark:text-orange-500"
                  : "border-zinc-200 dark:border-zinc-700 text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600"
              }`}
            >
              {option.label}
            </button>
          )}
        </For>
      </div>
    </div>
  );
}
