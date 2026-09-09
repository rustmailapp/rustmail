import { Show } from "solid-js";
import { undoableId, undoDelete } from "../stores/messages";

/**
 * The way back from a deleted message.
 *
 * Deletion is reversible only while the store is still holding the DELETE
 * back, so this reads that state rather than keeping a queue of its own: it is
 * on screen exactly as long as there is something to undo. The toast stack in
 * `App` places it, so that the failure notices and this one cannot land on top
 * of each other.
 */
export default function UndoToast() {
  return (
    <Show when={undoableId()}>
      <div
        role="status"
        class="pointer-events-auto flex items-center gap-4 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-4 py-2.5 shadow-lg animate-fade-in"
      >
        <span class="text-xs text-zinc-600 dark:text-zinc-300">
          Message deleted
        </span>
        <button
          onClick={undoDelete}
          class="text-xs font-medium text-orange-500 hover:text-orange-400 transition cursor-pointer"
        >
          Undo <span class="text-zinc-400 dark:text-zinc-500">(u)</span>
        </button>
      </div>
    </Show>
  );
}
