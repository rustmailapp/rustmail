import { For } from "solid-js";
import { dismissNotice, notices } from "../stores/notices";

/**
 * What the UI has to say about a write: one that did not land, or a release
 * whose outcome the user is waiting on.
 *
 * `alert` rather than `status`: these appear only for something the user
 * asked for and is waiting to hear about, which is worth interrupting a
 * screen reader for, and it keeps the undo toast the one `status` on the page.
 */
export default function Notices() {
  return (
    <For each={notices()}>
      {(notice) => (
        <div
          role="alert"
          class="pointer-events-auto flex items-center gap-3 rounded-lg border bg-white dark:bg-zinc-900 px-4 py-2.5 shadow-lg animate-fade-in"
          classList={{
            "border-red-300 dark:border-red-900": notice.tone === "error",
            "border-emerald-300 dark:border-emerald-800":
              notice.tone === "success",
          }}
        >
          <span class="text-xs text-zinc-700 dark:text-zinc-200">
            {notice.text}
          </span>
          <button
            onClick={() => dismissNotice(notice.id)}
            aria-label="Dismiss notice"
            class="text-zinc-500 dark:text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 transition cursor-pointer"
          >
            <svg
              class="size-3.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              stroke-width="2.5"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>
      )}
    </For>
  );
}
