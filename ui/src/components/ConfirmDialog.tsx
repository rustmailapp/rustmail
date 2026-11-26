import {
  For,
  Show,
  onMount,
  onCleanup,
  createSignal,
  createUniqueId,
} from "solid-js";
import Modal from "./Modal";

export interface ConfirmDialogOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** A caution about this particular confirmation, shown above the list. */
  warning?: string;
  /** Items the confirmation acts on, listed under the message. */
  list?: { label: string; items: readonly string[] };
  /**
   * Whether Enter confirms. Off for an action that cannot be taken back:
   * focus then starts on Cancel instead of the confirm button, so only a
   * click or a deliberate Tab to the confirm button goes ahead.
   */
  enterConfirms?: boolean;
}

const [dialogState, setDialogState] = createSignal<{
  options: ConfirmDialogOptions;
  resolve: (confirmed: boolean) => void;
} | null>(null);

/** Whether a confirmation is on screen and waiting on an answer. */
export function confirmOpen(): boolean {
  return dialogState() !== null;
}

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
  const titleId = createUniqueId();
  let dialog: HTMLDivElement | undefined;
  let cancelButton: HTMLButtonElement | undefined;
  let confirmButton: HTMLButtonElement | undefined;
  function onKeyDown(e: KeyboardEvent) {
    const state = dialogState();
    if (!state || e.key !== "Enter") return;
    e.stopPropagation();
    const focused = document.activeElement;
    const onDialogButton =
      focused instanceof HTMLButtonElement &&
      (dialog?.contains(focused) ?? false);
    if (e.repeat || !onDialogButton) e.preventDefault();
    if (!e.repeat && !onDialogButton && state.options.enterConfirms !== false) {
      close(true);
    }
  }

  onMount(() => document.addEventListener("keydown", onKeyDown, true));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown, true));

  return (
    <Show when={dialogState()}>
      {(state) => (
        <Modal
          open={true}
          onClose={() => close(false)}
          labelledBy={titleId}
          initialFocus={() =>
            state().options.enterConfirms === false
              ? cancelButton
              : confirmButton
          }
          maxWidth="max-w-sm"
        >
          <div ref={dialog}>
            <div class="px-6 pt-5 pb-2">
              <h3
                id={titleId}
                class="text-sm font-semibold text-zinc-900 dark:text-zinc-100"
              >
                {state().options.title}
              </h3>
              <p class="mt-1.5 text-xs text-zinc-500 dark:text-zinc-400">
                {state().options.message}
              </p>
              <Show when={state().options.warning}>
                {(warning) => (
                  <p class="mt-3 rounded-md border border-amber-200 dark:border-amber-900/50 bg-amber-50 dark:bg-amber-900/20 px-2.5 py-1.5 text-xs font-medium text-amber-800 dark:text-amber-300">
                    {warning()}
                  </p>
                )}
              </Show>
              <Show when={state().options.list}>
                {(list) => (
                  <div class="mt-3">
                    <p class="text-xs font-medium text-zinc-700 dark:text-zinc-200">
                      {list().label}
                    </p>
                    <ul
                      aria-label={list().label}
                      class="mt-1 max-h-40 overflow-y-auto rounded-md border border-zinc-200 dark:border-zinc-700 px-2.5 py-1.5 font-mono text-xs text-zinc-800 dark:text-zinc-200"
                    >
                      <For each={list().items}>
                        {(item) => <li class="break-all py-0.5">{item}</li>}
                      </For>
                    </ul>
                  </div>
                )}
              </Show>
            </div>
            <div class="flex justify-end gap-2 px-6 py-4">
              <button
                ref={cancelButton}
                onClick={() => close(false)}
                class="rounded-md border border-zinc-200 dark:border-zinc-700 px-3 py-1.5 text-xs font-medium text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
              >
                {state().options.cancelLabel ?? "Cancel"}
              </button>
              <button
                ref={confirmButton}
                onClick={() => close(true)}
                class="btn-destructive rounded-md border px-3 py-1.5 text-xs font-medium transition cursor-pointer"
              >
                {state().options.confirmLabel ?? "Confirm"}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </Show>
  );
}
