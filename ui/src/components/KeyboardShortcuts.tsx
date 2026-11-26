import { For, createUniqueId, onMount } from "solid-js";
import Modal from "./Modal";
import Kbd from "./Kbd";
import { returnFocusOnCleanup } from "./focusReturn";
import { shortcutList } from "../lib/shortcuts";
import { setShortcutsOpen, shortcutsOpen } from "../stores/commandPalette";

/** Every keyboard shortcut, listed in a dialog the command palette opens. */
export default function KeyboardShortcuts() {
  return (
    <Modal
      open={shortcutsOpen()}
      onClose={() => setShortcutsOpen(false)}
      maxWidth="max-w-sm"
    >
      <ShortcutsBody />
    </Modal>
  );
}

function ShortcutsBody() {
  const titleId = createUniqueId();
  let dialog: HTMLDivElement | undefined;
  let closeButton!: HTMLButtonElement;
  returnFocusOnCleanup(() => dialog);
  onMount(() => closeButton.focus());

  return (
    <div
      ref={dialog}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        if (event.key === "Tab") event.preventDefault();
      }}
    >
      <div class="flex items-center justify-between px-6 py-4 border-b border-zinc-200 dark:border-zinc-800">
        <h2 id={titleId} class="text-base font-semibold">
          Keyboard shortcuts
        </h2>
        <button
          ref={closeButton}
          onClick={() => setShortcutsOpen(false)}
          aria-label="Close"
          class="rounded-md p-1.5 text-zinc-500 dark:text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
        >
          <svg
            class="size-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
      <dl class="px-6 py-4 space-y-2.5 text-xs">
        <For each={shortcutList()}>
          {(shortcut) => (
            <div class="flex items-center justify-between gap-4">
              <dt class="text-zinc-600 dark:text-zinc-300">{shortcut.label}</dt>
              <dd class="flex items-center gap-1">
                <For each={shortcut.keys}>{(key) => <Kbd>{key}</Kbd>}</For>
              </dd>
            </div>
          )}
        </For>
      </dl>
    </div>
  );
}
