import { Show, onMount, onCleanup } from "solid-js";
import type { JSX } from "solid-js";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  maxWidth?: string;
  children: JSX.Element;
}

export default function Modal(props: ModalProps) {
  function onKeyDown(e: KeyboardEvent) {
    if (!props.open) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
    }
  }

  onMount(() => document.addEventListener("keydown", onKeyDown, true));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown, true));

  return (
    <Show when={props.open}>
      <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
        <div
          class="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
          onClick={() => props.onClose()}
        />
        <div
          class={`relative w-full ${props.maxWidth ?? "max-w-lg"} bg-white dark:bg-zinc-900 rounded-2xl border border-zinc-200 dark:border-zinc-800 shadow-2xl animate-scale-in overflow-hidden`}
        >
          {props.children}
        </div>
      </div>
    </Show>
  );
}
