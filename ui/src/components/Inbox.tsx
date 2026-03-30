import { For, Show } from "solid-js";
import {
  filteredMessages,
  messages,
  selectedId,
  setSelectedId,
  loading,
  hasActiveFilters,
  clearFilters,
  search,
} from "../stores/messages";
import { formatDate, formatSize } from "../lib/format";
import * as api from "../lib/api";

export default function Inbox() {
  return (
    <div class="flex flex-col overflow-y-auto h-full">
      <Show when={!loading() && filteredMessages().length === 0}>
        <div class="flex flex-col items-center justify-center h-full text-zinc-500 dark:text-zinc-500">
          <Show
            when={messages().length === 0 && !search()}
            fallback={
              <>
                <svg
                  class="size-10 mb-3 opacity-30"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  stroke-width="1.5"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M12 3c2.755 0 5.455.232 8.083.678.533.09.917.556.917 1.096v1.044a2.25 2.25 0 01-.659 1.591l-5.432 5.432a2.25 2.25 0 00-.659 1.591v2.927a2.25 2.25 0 01-1.244 2.013L9.75 21v-6.568a2.25 2.25 0 00-.659-1.591L3.659 7.409A2.25 2.25 0 013 5.818V4.774c0-.54.384-1.006.917-1.096A48.32 48.32 0 0112 3z"
                  />
                </svg>
                <p class="text-sm">No matching messages</p>
                <Show when={hasActiveFilters()}>
                  <button
                    onClick={clearFilters}
                    class="text-xs mt-2 text-orange-500 hover:text-orange-400 transition cursor-pointer"
                  >
                    Clear filters
                  </button>
                </Show>
              </>
            }
          >
            <svg
              class="size-12 mb-3 opacity-30"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              stroke-width="1.5"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0L3.32 8.91a2.25 2.25 0 01-1.07-1.916V6.75"
              />
            </svg>
            <p class="text-sm">No messages yet</p>
            <p class="text-xs mt-1 text-zinc-400 dark:text-zinc-600">
              Send an email to the SMTP port to get started
            </p>
          </Show>
        </div>
      </Show>

      <For each={filteredMessages()}>
        {(msg) => {
          const isSelected = () => selectedId() === msg.id;
          const recipients = () => msg.recipients;

          return (
            <div
              role="button"
              tabIndex={0}
              onClick={async () => {
                setSelectedId(msg.id);
                if (!msg.is_read) {
                  try {
                    await api.markRead(msg.id, true);
                  } catch {
                    // WS event handles UI sync
                  }
                }
              }}
              class={`w-full text-left px-4 py-3 border-b border-zinc-100 dark:border-zinc-800/50 transition cursor-pointer ${
                isSelected()
                  ? "bg-zinc-100 dark:bg-zinc-800/80"
                  : "hover:bg-zinc-50 dark:hover:bg-zinc-900"
              }`}
            >
              <div class="flex items-start gap-3">
                <div class="flex-shrink-0 mt-0.5 flex flex-col items-center gap-1">
                  <div
                    class={`size-2 rounded-full mt-1.5 ${msg.is_read ? "bg-transparent" : "bg-orange-500"}`}
                  />
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      api.markStarred(msg.id, !msg.is_starred).catch(() => {});
                    }}
                    class="cursor-pointer"
                    title={msg.is_starred ? "Unstar" : "Star"}
                  >
                    <svg
                      class={`size-3.5 transition ${msg.is_starred ? "text-amber-400 fill-amber-400" : "text-zinc-300 dark:text-zinc-600 hover:text-amber-400"}`}
                      viewBox="0 0 24 24"
                      stroke="currentColor"
                      stroke-width="2"
                      fill={msg.is_starred ? "currentColor" : "none"}
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"
                      />
                    </svg>
                  </button>
                </div>
                <div class="flex-1 min-w-0">
                  <div class="flex items-center justify-between gap-2">
                    <span
                      class={`text-sm truncate ${msg.is_read ? "text-zinc-400 dark:text-zinc-400" : "text-zinc-900 dark:text-zinc-100 font-medium"}`}
                    >
                      {msg.sender || "(no sender)"}
                    </span>
                    <span class="text-xs text-zinc-500 dark:text-zinc-600 flex-shrink-0">
                      {formatDate(msg.created_at)} · {formatSize(msg.size)}
                    </span>
                  </div>
                  <div class="flex items-center gap-1.5 mt-0.5">
                    <span
                      class={`text-sm truncate ${msg.is_read ? "text-zinc-500 dark:text-zinc-500" : "text-zinc-700 dark:text-zinc-200"}`}
                    >
                      {msg.subject || "(no subject)"}
                    </span>
                    <Show when={msg.has_attachments}>
                      <svg
                        class="size-3.5 flex-shrink-0 text-zinc-500 dark:text-zinc-500"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                        stroke-width="2"
                      >
                        <path
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          d="M18.375 12.739l-7.693 7.693a4.5 4.5 0 01-6.364-6.364l10.94-10.94A3 3 0 1119.5 7.372L8.552 18.32m.009-.01l-.01.01m5.699-9.941l-7.81 7.81a1.5 1.5 0 002.112 2.13"
                        />
                      </svg>
                    </Show>
                  </div>
                  <div class="flex items-center gap-1.5 mt-0.5">
                    <p class="text-xs text-zinc-500 dark:text-zinc-600 truncate">
                      To: {recipients().join(", ")}
                    </p>
                    <Show when={msg.tags.length > 0}>
                      <div class="flex gap-1 flex-shrink-0">
                        <For each={msg.tags.slice(0, 3)}>
                          {(tag) => (
                            <span class="inline-block px-1.5 py-0 rounded text-[10px] font-medium bg-orange-100 text-orange-700 dark:bg-orange-900/40 dark:text-orange-300">
                              {tag}
                            </span>
                          )}
                        </For>
                        <Show when={msg.tags.length > 3}>
                          <span class="text-[10px] text-zinc-400">
                            +{msg.tags.length - 3}
                          </span>
                        </Show>
                      </div>
                    </Show>
                  </div>
                </div>
              </div>
            </div>
          );
        }}
      </For>
    </div>
  );
}
