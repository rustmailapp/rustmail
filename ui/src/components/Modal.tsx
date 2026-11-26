import { Show, onMount, onCleanup } from "solid-js";
import type { JSX } from "solid-js";
import { focusableIn } from "./focusable";
import { returnFocusOnCleanup } from "./focusReturn";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  /** Id of the element that names the dialog, usually its heading. */
  labelledBy: string;
  /** The element to focus on open; the first control when absent. */
  initialFocus?: () => HTMLElement | undefined;
  maxWidth?: string;
  /** Where the panel sits: centred, or near the top so its height can change without moving it. */
  align?: "center" | "top";
  children: JSX.Element;
}

/**
 * A modal dialog: Escape and a click outside close it.
 *
 * On open focus moves to `initialFocus`, or else the first control. Tab and
 * Shift+Tab cycle inside the panel, and on close focus goes back to whatever
 * had it before.
 */
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
      <div
        class={`fixed inset-0 z-50 flex justify-center p-4 ${props.align === "top" ? "items-start pt-[15vh]" : "items-center"}`}
      >
        <div
          class="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
          onClick={() => props.onClose()}
        />
        <Panel
          labelledBy={props.labelledBy}
          initialFocus={props.initialFocus}
          maxWidth={props.maxWidth}
        >
          {props.children}
        </Panel>
      </div>
    </Show>
  );
}

function Panel(
  props: Pick<
    ModalProps,
    "labelledBy" | "initialFocus" | "maxWidth" | "children"
  >,
) {
  let panel!: HTMLDivElement;
  returnFocusOnCleanup(() => panel);

  onMount(() => {
    if (panel.contains(document.activeElement)) return;
    const target = props.initialFocus?.() ?? focusableIn(panel)[0] ?? panel;
    target.focus();
  });

  onMount(() => document.addEventListener("keydown", trapTab, true));
  onCleanup(() => document.removeEventListener("keydown", trapTab, true));

  function trapTab(e: KeyboardEvent) {
    if (e.key !== "Tab") return;
    const focusable = focusableIn(panel);
    const index = focusable.findIndex(
      (element) => element === document.activeElement,
    );
    const last = focusable.length - 1;
    const wraps = e.shiftKey ? index <= 0 : index === -1 || index === last;
    if (!wraps) return;
    e.preventDefault();
    focusable[e.shiftKey ? last : 0]?.focus();
  }

  return (
    <div
      ref={panel}
      role="dialog"
      aria-modal="true"
      aria-labelledby={props.labelledBy}
      tabIndex={-1}
      class={`relative w-full ${props.maxWidth ?? "max-w-lg"} bg-white dark:bg-zinc-900 rounded-2xl border border-zinc-200 dark:border-zinc-800 shadow-2xl animate-scale-in overflow-hidden outline-none`}
    >
      {props.children}
    </div>
  );
}
