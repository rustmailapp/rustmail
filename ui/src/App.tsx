import { onMount, onCleanup, Show } from "solid-js";
import Header from "./components/Header";
import FilterBar from "./components/FilterBar";
import Inbox from "./components/Inbox";
import MessageDetail from "./components/MessageDetail";
import Settings from "./components/Settings";
import ConfirmDialog, {
  confirm,
  confirmOpen,
} from "./components/ConfirmDialog";
import UndoToast from "./components/UndoToast";
import Notices from "./components/Notices";
import StatusBar from "./components/StatusBar";
import {
  connectWebSocket,
  disconnectWebSocket,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  selectMessage,
  starMessage,
  clearInbox,
  clearInboxPrompt,
  moveSelection,
  deleteWithUndo,
  flushPendingDelete,
  undoDelete,
  hasActiveFilters,
  clearFilters,
} from "./stores/messages";
import { settingsOpen } from "./stores/settings";
import {
  closeDetails,
  detailsDrawerOpen,
  toggleDetails,
} from "./stores/layout";
import "./stores/theme";
import "./stores/palette";
import { rustedToast } from "./stores/rusted";

export default function App() {
  /**
   * Hands focus to the message list after a j/k move.
   *
   * A button clicked earlier would otherwise keep focus and start showing its
   * keyboard ring, and the arrow keys only move the selection from the list.
   * The HTML preview keeps focus so its own scrolling still works.
   */
  function focusList() {
    if (document.activeElement?.tagName === "IFRAME") return;
    document
      .querySelector<HTMLElement>('[role="listbox"][aria-label="Messages"]')
      ?.focus({ preventScroll: true });
  }

  function handleKeydown(e: KeyboardEvent) {
    if (settingsOpen() || confirmOpen()) return;
    const tag = (e.target as HTMLElement).tagName;
    if (tag === "INPUT" || tag === "TEXTAREA") return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;

    const msgs = filteredMessages();
    const currentIdx = msgs.findIndex((m) => m.id === selectedId());

    switch (e.key) {
      case "j": {
        moveSelection("next");
        focusList();
        break;
      }
      case "k": {
        moveSelection("prev");
        focusList();
        break;
      }
      case "d": {
        const id = selectedId();
        if (id) {
          const next =
            currentIdx === -1
              ? null
              : (msgs[currentIdx + 1] ?? msgs[currentIdx - 1] ?? null);
          if (next) {
            selectMessage(next);
          } else {
            setSelectedId(null);
          }
          deleteWithUndo(id);
        }
        break;
      }
      case "u": {
        undoDelete();
        break;
      }
      case "D": {
        if (total() === 0) break;
        confirm(clearInboxPrompt()).then(async (ok) => {
          if (ok) await clearInbox();
        });
        break;
      }
      case "s": {
        const id = selectedId();
        if (id) {
          const msg = filteredMessages().find((m) => m.id === id);
          if (msg) starMessage(id, !msg.is_starred);
        }
        break;
      }
      case "/": {
        e.preventDefault();
        document
          .querySelector<HTMLInputElement>(
            'input[placeholder="Search emails..."]',
          )
          ?.focus();
        break;
      }
      case "i": {
        if (selectedId()) toggleDetails();
        break;
      }
      case "Escape": {
        if (detailsDrawerOpen() && selectedId()) {
          closeDetails();
        } else if (hasActiveFilters()) {
          clearFilters();
        } else {
          setSelectedId(null);
        }
        break;
      }
    }
  }

  function selectFirstMessage() {
    if (selectedId()) return;
    const first = filteredMessages()[0];
    if (first) setSelectedId(first.id);
  }

  onMount(() => {
    connectWebSocket(selectFirstMessage);
    document.addEventListener("keydown", handleKeydown);
    window.addEventListener("pagehide", flushPendingDelete);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeydown);
    window.removeEventListener("pagehide", flushPendingDelete);
    disconnectWebSocket();
  });

  return (
    <div class="app-shell flex flex-col h-screen gap-2.5 p-2.5 text-zinc-900 dark:text-zinc-100">
      <div class="flex flex-1 min-h-0 gap-2.5">
        <aside
          aria-label="Inbox"
          class="app-panel w-90 flex-shrink-0 overflow-hidden flex flex-col"
        >
          <Header />
          <FilterBar />
          <Inbox />
        </aside>
        <MessageDetail />
      </div>
      <StatusBar />
      <Settings />
      <ConfirmDialog />
      <div class="pointer-events-none fixed bottom-14 left-1/2 -translate-x-1/2 z-50 flex flex-col items-center gap-2">
        <Notices />
        <UndoToast />
        <div role="status" class="contents">
          <Show when={rustedToast()}>
            {(text) => (
              <div class="rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-4 py-2.5 shadow-lg animate-fade-in text-xs text-zinc-600 dark:text-zinc-300">
                {text()}
              </div>
            )}
          </Show>
        </div>
      </div>
    </div>
  );
}
