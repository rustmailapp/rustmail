import { Show, onMount, onCleanup, createSignal } from "solid-js";
import Modal from "./Modal";

export interface ConfirmDialogOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
}

const [dialogState, setDialogState] = createSignal<{
  options: ConfirmDialogOptions;
  resolve: (confirmed: boolean) => void;
} | null>(null);

export function confirm(options: ConfirmDialogOptions): Promise<boolean> {
  return new Promise((resolve) => {
    setDialogState({ options, resolve });
  });
}

function close(confirmed: boolean) {
  const state = dialogState();
  if (state) {
    state.resolve(confirmed);
    setDialogState(null);
  }
}

export default function ConfirmDialog() {
  function onKeyDown(e: KeyboardEvent) {
    if (!dialogState()) return;
    if (e.key === "Enter") {
      e.stopPropagation();
      close(true);
    }
  }

  onMount(() => document.addEventListener("keydown", onKeyDown, true));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown, true));

  return (
    <Show when={dialogState()}>
      {(state) => (
        <Modal open={true} onClose={() => close(false)} maxWidth="max-w-sm">
          <div class="px-6 pt-5 pb-2">
            <h3 class="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
              {state().options.title}
            </h3>
            <p class="mt-1.5 text-xs text-zinc-500 dark:text-zinc-400">
              {state().options.message}
            </p>
          </div>
          <div class="flex justify-end gap-2 px-6 py-4">
            <button
              onClick={() => close(false)}
              class="rounded-lg border border-zinc-200 dark:border-zinc-700 px-3 py-1.5 text-xs font-medium text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
            >
              {state().options.cancelLabel ?? "Cancel"}
            </button>
            <button
              onClick={() => close(true)}
              class="btn-destructive rounded-lg border px-3 py-1.5 text-xs font-medium transition cursor-pointer"
            >
              {state().options.confirmLabel ?? "Confirm"}
            </button>
          </div>
        </Modal>
      )}
    </Show>
  );
}
