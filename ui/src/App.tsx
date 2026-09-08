import { onMount, onCleanup } from "solid-js";
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
import {
  fetchMessages,
  connectWebSocket,
  disconnectWebSocket,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  selectMessage,
  moveSelection,
  deleteWithUndo,
  flushPendingDelete,
  undoDelete,
  hasActiveFilters,
  clearFilters,
} from "./stores/messages";
import * as api from "./lib/api";
import { settingsOpen } from "./stores/settings";
import "./stores/theme";
import "./stores/rusted";

export default function App() {
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
        break;
      }
      case "k": {
        moveSelection("prev");
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
        const count = total();
        if (count === 0) break;
        confirm({
          title: "Clear all messages",
          message: `All ${count} messages will be permanently deleted.`,
          confirmLabel: "Clear all",
        }).then(async (ok) => {
          if (ok) {
            try {
              await api.deleteAllMessages();
            } catch {
              console.error("Failed to clear messages");
            }
          }
        });
        break;
      }
      case "s": {
        const id = selectedId();
        if (id) {
          const msg = filteredMessages().find((m) => m.id === id);
          if (msg) api.markStarred(id, !msg.is_starred).catch(() => {});
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
      case "Escape": {
        if (hasActiveFilters()) {
          clearFilters();
        } else {
          setSelectedId(null);
        }
        break;
      }
    }
  }

  onMount(async () => {
    await fetchMessages();
    if (!selectedId()) {
      const first = filteredMessages()[0];
      if (first) setSelectedId(first.id);
    }
    connectWebSocket();
    document.addEventListener("keydown", handleKeydown);
    window.addEventListener("pagehide", flushPendingDelete);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeydown);
    window.removeEventListener("pagehide", flushPendingDelete);
    disconnectWebSocket();
  });

  return (
    <div class="flex flex-col h-screen bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100">
      <div class="mesh-glow" />
      <Header />
      <div class="flex flex-1 overflow-hidden">
        <div class="w-96 flex-shrink-0 border-r border-zinc-200 dark:border-zinc-800 overflow-hidden flex flex-col">
          <FilterBar />
          <Inbox />
        </div>
        <div class="flex-1 overflow-hidden">
          <MessageDetail />
        </div>
      </div>
      <Settings />
      <ConfirmDialog />
      <UndoToast />
    </div>
  );
}
