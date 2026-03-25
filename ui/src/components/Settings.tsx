import { Show, For } from "solid-js";
import { theme, setTheme } from "../stores/theme";
import {
  settingsOpen,
  setSettingsOpen,
  settingsTab,
  setSettingsTab,
} from "../stores/settings";
import type { SettingsTab } from "../stores/settings";
import Modal from "./Modal";

const TABS: { value: SettingsTab; label: string; icon: string }[] = [
  {
    value: "appearance",
    label: "Appearance",
    icon: "M4.098 19.902a3.75 3.75 0 005.304 0l6.401-6.402M6.75 21A3.75 3.75 0 013 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 003.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.879 2.88M6.75 17.25h.008v.008H6.75v-.008z",
  },
];

export default function Settings() {
  return (
    <Modal open={settingsOpen()} onClose={() => setSettingsOpen(false)}>
      <div class="flex items-center justify-between px-6 py-4 border-b border-zinc-200 dark:border-zinc-800">
        <h2 class="text-base font-semibold">Settings</h2>
        <button
          onClick={() => setSettingsOpen(false)}
          class="rounded-lg p-1.5 text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
        >
          <svg
            class="size-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>

      <div class="flex border-b border-zinc-200 dark:border-zinc-800 px-6 gap-1">
        <For each={TABS}>
          {(tab) => (
            <button
              onClick={() => setSettingsTab(tab.value)}
              class={`flex items-center gap-1.5 px-3 py-2.5 text-xs font-medium border-b-2 -mb-px transition cursor-pointer ${
                settingsTab() === tab.value
                  ? "border-orange-500 text-zinc-900 dark:text-zinc-100"
                  : "border-transparent text-zinc-400 dark:text-zinc-500 hover:text-zinc-600 dark:hover:text-zinc-300"
              }`}
            >
              <svg
                class="size-3.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d={tab.icon}
                />
              </svg>
              {tab.label}
            </button>
          )}
        </For>
      </div>

      <div class="px-6 py-5 max-h-[60vh] overflow-y-auto">
        <Show when={settingsTab() === "appearance"}>
          <AppearanceTab />
        </Show>
      </div>
    </Modal>
  );
}

function AppearanceTab() {
  return (
    <div>
      <label class="text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-2 block">
        Theme
      </label>
      <div class="grid grid-cols-2 gap-2">
        <button
          onClick={() => setTheme("dark")}
          class={`rounded-lg border px-3 py-2.5 text-xs font-medium transition cursor-pointer flex items-center justify-center gap-2 ${
            theme() === "dark"
              ? "border-orange-500 bg-orange-500/10 text-orange-500"
              : "border-zinc-200 dark:border-zinc-700 text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600"
          }`}
        >
          <svg
            class="size-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M21.752 15.002A9.718 9.718 0 0118 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 003 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 009.002-5.998z"
            />
          </svg>
          Dark
        </button>
        <button
          onClick={() => setTheme("light")}
          class={`rounded-lg border px-3 py-2.5 text-xs font-medium transition cursor-pointer flex items-center justify-center gap-2 ${
            theme() === "light"
              ? "border-orange-500 bg-orange-500/10 text-orange-500"
              : "border-zinc-200 dark:border-zinc-700 text-zinc-500 dark:text-zinc-400 hover:border-zinc-300 dark:hover:border-zinc-600"
          }`}
        >
          <svg
            class="size-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M12 3v2.25m6.364.386l-1.591 1.591M21 12h-2.25m-.386 6.364l-1.591-1.591M12 18.75V21m-4.773-4.227l-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0z"
            />
          </svg>
          Light
        </button>
      </div>
    </div>
  );
}
