import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
} from "solid-js";
import Modal from "./Modal";
import Kbd from "./Kbd";
import { confirm } from "./ConfirmDialog";
import { confirmRelease } from "./MessageDetail";
import {
  availableCommands,
  type CommandActions,
  type CommandState,
} from "../lib/commands";
import { fuzzyFilter } from "../lib/fuzzy";
import * as api from "../lib/api";
import type { MessageSummary } from "../lib/types";
import {
  allTags,
  clearFilters,
  clearInbox,
  clearInboxPrompt,
  deleteSelected,
  filteredMessages,
  filters,
  selectMessage,
  selectedId,
  setMessageRead,
  starMessage,
  tagFiltersFull,
  toggleFilter,
  toggleTagFilter,
  total,
  undoDelete,
  undoableId,
} from "../stores/messages";
import {
  detailsDrawerOpen,
  detailsRailOpen,
  toggleDetails,
  wideLayout,
} from "../stores/layout";
import { theme, setTheme } from "../stores/theme";
import { PALETTES, palette, setPalette } from "../stores/palette";
import { rusted, setRusted } from "../stores/rusted";
import { density, toggleDensity } from "../stores/density";
import {
  PREVIEW_WIDTHS,
  previewWidth,
  setPreviewWidth,
} from "../stores/previewWidth";
import { releaseTarget } from "../stores/server";
import { releasing } from "../stores/release";
import { setSettingsOpen } from "../stores/settings";
import {
  commandPaletteOpen,
  setCommandPaletteOpen,
  setShortcutsOpen,
} from "../stores/commandPalette";

/** The most messages a query lists, so a broad one stays readable. */
const MAX_MESSAGE_RESULTS = 20;

function resultCount(count: number): string {
  return count === 1 ? "1 result" : `${count} results`;
}

/** One row of the palette: a command, or a message to jump to. */
interface PaletteItem {
  key: string;
  label: string;
  detail?: string;
  shortcut?: string;
  run: () => void;
}

function downloadEml(id: string): void {
  const link = document.createElement("a");
  link.href = api.exportUrl(id, "eml");
  link.download = `${id}.eml`;
  link.click();
}

const actions: CommandActions = {
  setStarred: starMessage,
  setRead: setMessageRead,
  deleteSelected,
  undoDelete,
  downloadEml,
  release: (message) => {
    const target = releaseTarget();
    if (target) void confirmRelease(message, target);
  },
  toggleDetails,
  toggleFilter,
  toggleTagFilter,
  clearFilters,
  clearInbox: () => {
    void confirm(clearInboxPrompt()).then(async (ok) => {
      if (ok) await clearInbox();
    });
  },
  setTheme,
  setPalette,
  setRusted,
  toggleDensity,
  setPreviewWidth,
  openSettings: () => setSettingsOpen(true),
  showShortcuts: () => setShortcutsOpen(true),
};

function commandState(): CommandState {
  const id = selectedId();
  return {
    selected: filteredMessages().find((m) => m.id === id) ?? null,
    releaseConfigured: releaseTarget() !== null,
    releasing: id !== null && releasing(id),
    detailsShown: wideLayout() ? detailsRailOpen() : detailsDrawerOpen(),
    canUndo: undoableId() !== null,
    total: total(),
    filters: filters(),
    tags: allTags(),
    tagFiltersFull: tagFiltersFull(),
    theme: theme(),
    palette: palette(),
    palettes: PALETTES,
    rusted: rusted(),
    density: density(),
    previewWidth: previewWidth(),
    previewWidths: PREVIEW_WIDTHS,
  };
}

function messageItem(message: MessageSummary): PaletteItem {
  return {
    key: `message.${message.id}`,
    label: message.subject || "(no subject)",
    detail: message.sender,
    run: () => selectMessage(message),
  };
}

/**
 * The ⌘K / Ctrl+K command palette.
 *
 * Every command it lists calls the same store function as the button or key
 * that does the same thing, so the two never drift apart.
 */
export default function CommandPalette() {
  const titleId = createUniqueId();
  return (
    <Modal
      open={commandPaletteOpen()}
      onClose={() => setCommandPaletteOpen(false)}
      labelledBy={titleId}
      maxWidth="max-w-xl"
      align="top"
    >
      <PaletteBody titleId={titleId} />
    </Modal>
  );
}

/**
 * The palette's search box and results, mounted only while it is open.
 *
 * The search box is the dialog's only control, so focus starts and stays
 * there, and the arrow keys move the active row through
 * `aria-activedescendant`.
 */
function PaletteBody(props: { titleId: string }) {
  const listId = createUniqueId();
  const [query, setQuery] = createSignal("");
  const [active, setActive] = createSignal(0);
  let input!: HTMLInputElement;
  let list!: HTMLDivElement;

  const items = createMemo<PaletteItem[]>(() => {
    const q = query();
    const commands = fuzzyFilter(
      availableCommands(commandState(), actions),
      q,
      (command) => `${command.label} ${command.keywords ?? ""}`,
    ).map((command) => ({ ...command, key: command.id }));
    if (q.trim() === "") return commands;
    const messages = fuzzyFilter(
      filteredMessages(),
      q,
      (message) => `${message.subject ?? ""} ${message.sender}`,
    )
      .slice(0, MAX_MESSAGE_RESULTS)
      .map(messageItem);
    return [...commands, ...messages];
  });

  const optionId = (index: number) => `${listId}-option-${index}`;
  const current = () => Math.min(active(), items().length - 1);

  createEffect(() => {
    query();
    setActive(0);
  });

  createEffect(() => {
    const index = current();
    list
      .querySelector(`#${CSS.escape(optionId(index))}`)
      ?.scrollIntoView({ block: "nearest" });
  });

  function runItem(item: PaletteItem | undefined): void {
    if (!item) return;
    setCommandPaletteOpen(false);
    item.run();
  }

  function move(step: number): void {
    const count = items().length;
    if (count === 0) return;
    setActive((current() + step + count) % count);
  }

  function onKeyDown(event: KeyboardEvent): void {
    switch (event.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        setActive(0);
        break;
      case "End":
        setActive(Math.max(0, items().length - 1));
        break;
      case "Enter":
        if (!event.repeat && !event.isComposing) runItem(items()[current()]);
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  return (
    <div
      onMouseDown={(event) => {
        if (event.target !== input) event.preventDefault();
      }}
    >
      <h2 id={props.titleId} class="sr-only">
        Command palette
      </h2>
      <div class="flex items-center gap-2.5 px-4 border-b border-zinc-200 dark:border-zinc-800">
        <svg
          class="size-4 shrink-0 text-zinc-500 dark:text-zinc-400"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          stroke-width="2"
          aria-hidden="true"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 5.196a7.5 7.5 0 0010.607 10.607z"
          />
        </svg>
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label="Command"
          aria-expanded={items().length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={
            items().length > 0 ? optionId(current()) : undefined
          }
          autocomplete="off"
          spellcheck={false}
          placeholder="Type a command or search messages..."
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={onKeyDown}
          class="flex-1 min-w-0 bg-transparent py-3.5 text-sm text-zinc-900 dark:text-zinc-100 placeholder:text-zinc-500 dark:placeholder:text-zinc-400 outline-none"
        />
      </div>
      <div
        ref={list}
        id={listId}
        role="listbox"
        aria-label="Commands"
        class="max-h-80 overflow-y-auto p-1.5 empty:hidden"
      >
        <For each={items()}>
          {(item, index) => (
            <div
              id={optionId(index())}
              role="option"
              aria-selected={index() === current()}
              onMouseMove={() => setActive(index())}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => runItem(item)}
              class="flex items-center gap-3 rounded-lg px-3 py-2 text-sm cursor-pointer text-zinc-700 dark:text-zinc-200 aria-selected:bg-zinc-100 aria-selected:text-zinc-900 dark:aria-selected:bg-zinc-800 dark:aria-selected:text-zinc-50"
            >
              <span class="min-w-0 flex-1 truncate">
                {item.label}
                <Show when={item.detail}>
                  {(detail) => (
                    <span class="ml-2 text-xs text-zinc-500 dark:text-zinc-400">
                      {detail()}
                    </span>
                  )}
                </Show>
              </span>
              <Show when={item.shortcut}>
                {(shortcut) => <Kbd>{shortcut()}</Kbd>}
              </Show>
            </div>
          )}
        </For>
      </div>
      <Show when={items().length === 0}>
        <p class="px-3 pb-6 pt-4 text-center text-sm text-zinc-500 dark:text-zinc-400">
          No matching commands or messages
        </p>
      </Show>
      <p role="status" class="sr-only">
        {resultCount(items().length)}
      </p>
    </div>
  );
}
