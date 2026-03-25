import { onMount, onCleanup } from "solid-js";
import Header from "./components/Header";
import FilterBar from "./components/FilterBar";
import Inbox from "./components/Inbox";
import MessageDetail from "./components/MessageDetail";
import Settings from "./components/Settings";
import ConfirmDialog, { confirm } from "./components/ConfirmDialog";
import {
  fetchMessages,
  connectWebSocket,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  hasActiveFilters,
  clearFilters,
} from "./stores/messages";
import * as api from "./lib/api";
import { settingsOpen } from "./stores/settings";
import "./stores/theme";

export default function App() {
  function handleKeydown(e: KeyboardEvent) {
    if (settingsOpen()) return;
    const tag = (e.target as HTMLElement).tagName;
    if (tag === "INPUT" || tag === "TEXTAREA") return;

    const msgs = filteredMessages();
    if (msgs.length === 0 && (e.key === "j" || e.key === "k")) return;

    const currentIdx = msgs.findIndex((m) => m.id === selectedId());

    function selectAndRead(idx: number) {
      const msg = msgs[idx];
      if (msg) {
        setSelectedId(msg.id);
        if (!msg.is_read) api.markRead(msg.id, true);
      }
    }

    switch (e.key) {
      case "j": {
        if (currentIdx === -1) {
          selectAndRead(0);
        } else if (currentIdx < msgs.length - 1) {
          selectAndRead(currentIdx + 1);
        }
        break;
      }
      case "k": {
        if (currentIdx === -1) {
          selectAndRead(0);
        } else if (currentIdx > 0) {
          selectAndRead(currentIdx - 1);
        }
        break;
      }
      case "d": {
        const id = selectedId();
        if (id) {
          confirm({
            title: "Delete message",
            message: "This message will be permanently deleted.",
            confirmLabel: "Delete",
          }).then((ok) => {
            if (ok) {
              api.deleteMessage(id);
              setSelectedId(null);
            }
          });
        }
        break;
      }
      case "D": {
        const count = total();
        if (count === 0) break;
        confirm({
          title: "Clear all messages",
          message: `All ${count} messages will be permanently deleted.`,
          confirmLabel: "Clear all",
        }).then((ok) => {
          if (ok) api.deleteAllMessages();
        });
        break;
      }
      case "s": {
        const id = selectedId();
        if (id) {
          const msg = filteredMessages().find((m) => m.id === id);
          if (msg) api.markStarred(id, !msg.is_starred);
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

  onMount(() => {
    fetchMessages();
    connectWebSocket();
    document.addEventListener("keydown", handleKeydown);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeydown);
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
    </div>
  );
}
