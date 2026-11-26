import {
  Show,
  createSignal,
  createUniqueId,
  onCleanup,
  type JSX,
} from "solid-js";
import { Portal } from "solid-js/web";
import { placeTooltip, type TooltipPlacement } from "../lib/tooltipPlacement";

const HOVER_DELAY_MS = 300;

/** What the trigger element must carry for the tooltip to drive and describe it. */
export interface TooltipTrigger {
  ref: (element: HTMLElement) => void;
  describedBy: string;
}

/**
 * A short description shown next to an icon button, with an optional
 * keyboard shortcut.
 *
 * It opens on hover after a short delay and at once on keyboard focus, and
 * closes on pointer leave, blur, press or Escape. The trigger keeps its own
 * accessible name; the tooltip only describes it through `aria-describedby`.
 * It is portalled and fixed-positioned so a panel's overflow never clips it,
 * and `children` is a render function so the trigger stays a direct child of
 * its own layout (button groups style first and last children).
 */
export default function Tooltip(props: {
  label: string;
  shortcut?: string;
  children: (trigger: TooltipTrigger) => JSX.Element;
}) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(false);
  const [placement, setPlacement] = createSignal<TooltipPlacement>({
    top: 0,
    left: 0,
    side: "below",
  });
  let trigger: HTMLElement | undefined;
  let tooltip!: HTMLDivElement;
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  function cancelHover(): void {
    clearTimeout(hoverTimer);
    hoverTimer = undefined;
  }

  function show(): void {
    cancelHover();
    if (!trigger || open()) return;
    setOpen(true);
    const root = document.documentElement;
    setPlacement(
      placeTooltip(
        trigger.getBoundingClientRect(),
        tooltip.getBoundingClientRect(),
        { width: root.clientWidth, height: root.clientHeight },
      ),
    );
    document.addEventListener("keydown", hideOnEscape, true);
  }

  function hide(): void {
    cancelHover();
    if (!open()) return;
    setOpen(false);
    document.removeEventListener("keydown", hideOnEscape, true);
  }

  function hideOnEscape(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    hide();
  }

  function showSoon(): void {
    cancelHover();
    hoverTimer = setTimeout(show, HOVER_DELAY_MS);
  }

  function showOnKeyboardFocus(): void {
    if (trigger?.matches(":focus-visible")) show();
  }

  function attach(element: HTMLElement): void {
    trigger = element;
    element.addEventListener("pointerenter", showSoon);
    element.addEventListener("pointerleave", hide);
    element.addEventListener("click", hide);
    element.addEventListener("focus", showOnKeyboardFocus);
    element.addEventListener("blur", hide);
  }

  onCleanup(() => {
    hide();
    if (!trigger) return;
    trigger.removeEventListener("pointerenter", showSoon);
    trigger.removeEventListener("pointerleave", hide);
    trigger.removeEventListener("click", hide);
    trigger.removeEventListener("focus", showOnKeyboardFocus);
    trigger.removeEventListener("blur", hide);
  });

  return (
    <>
      {props.children({ ref: attach, describedBy: id })}
      <Portal>
        <div
          ref={tooltip}
          id={id}
          role="tooltip"
          hidden={!open()}
          style={{
            top: `${placement().top}px`,
            left: `${placement().left}px`,
          }}
          class="fixed z-50 flex items-center gap-1.5 rounded-md bg-zinc-900 px-2 py-1 text-xs font-medium whitespace-nowrap text-zinc-50 shadow-lg pointer-events-none dark:bg-zinc-100 dark:text-zinc-900 motion-safe:animate-fade-in"
        >
          {props.label}
          <Show when={props.shortcut}>
            {(shortcut) => (
              <kbd class="rounded border border-zinc-600 px-1 font-sans text-[10px] leading-4 text-zinc-300 dark:border-zinc-300 dark:text-zinc-600">
                {shortcut()}
              </kbd>
            )}
          </Show>
        </div>
      </Portal>
    </>
  );
}
