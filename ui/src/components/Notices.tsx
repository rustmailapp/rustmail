import { For } from "solid-js";
import { dismissNotice, notices } from "../stores/notices";

/**
 * What the UI has to say about a write that did not land.
 *
 * `alert` rather than `status`: these appear only when something the user
 * asked for failed, which is worth interrupting a screen reader for, and it
 * keeps the undo toast the one `status` on the page.
 */
export default function Notices() {
  return (
    <For each={notices()}>
      {(notice) => (
        <div
          role="alert"
          class="pointer-events-auto flex items-center gap-3 rounded-lg border border-red-300 dark:border-red-900 bg-white dark:bg-zinc-900 px-4 py-2.5 shadow-lg animate-fade-in"
        >
          <span class="text-xs text-zinc-700 dark:text-zinc-200">
            {notice.text}
          </span>
          <button
            onClick={() => dismissNotice(notice.id)}
            aria-label="Dismiss notice"
            class="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 transition cursor-pointer"
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
