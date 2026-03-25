import { Show } from "solid-js";
import { messages, total } from "../stores/messages";
import { toggleSettings } from "../stores/settings";
import * as api from "../lib/api";
import { confirm } from "./ConfirmDialog";

export default function Header() {
  return (
    <header class="flex items-center gap-3 border-b border-zinc-200 dark:border-zinc-800 px-4 py-3">
      <div class="flex items-center gap-2">
        <img src="/logo.webp" alt="RustMail" class="size-12 rounded-md" />
        <h1 class="font-brand text-xl font-bold tracking-tight bg-linear-to-r from-orange-500 to-zinc-300 dark:to-white bg-clip-text text-transparent">
          RustMail
        </h1>
      </div>

      <div class="flex-1" />

      <div class="flex items-center gap-3">
        <span class="text-xs text-zinc-500 dark:text-zinc-500">
          {total()} {total() === 1 ? "message" : "messages"}
        </span>

        <button
          onClick={toggleSettings}
          class="rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800 p-1.5 text-zinc-500 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200 transition cursor-pointer"
          title="Settings"
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
              d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 010 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 010-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28z"
            />
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
            />
          </svg>
        </button>

        <Show when={messages().length > 0}>
          <button
            onClick={async () => {
              const ok = await confirm({
                title: "Clear all messages",
                message: `All ${total()} messages will be permanently deleted.`,
                confirmLabel: "Clear all",
              });
              if (ok) await api.deleteAllMessages();
            }}
            class="btn-destructive rounded-md border px-2.5 py-1 text-xs font-medium transition cursor-pointer"
          >
            Clear all
          </button>
        </Show>
      </div>
    </header>
  );
}
