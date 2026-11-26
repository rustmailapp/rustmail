import { For } from "solid-js";

export interface SegmentOption<T extends string> {
  id: T;
  label: string;
}

/** Tab semantics for a segmented control whose segments switch a panel. */
export interface SegmentTabs<T extends string> {
  panelId: string;
  tabId: (id: T) => string;
}

/** Keys that move between tabs, as where they land in the list. */
const TAB_KEYS: Record<
  string,
  ((at: number, count: number) => number) | undefined
> = {
  ArrowRight: (at, count) => (at + 1) % count,
  ArrowLeft: (at, count) => (at - 1 + count) % count,
  Home: () => 0,
  End: (_, count) => count - 1,
};

/**
 * A row of segments with exactly one of them selected.
 *
 * The segments are toggle buttons unless `tabs` is given. Then they are a
 * tablist with a single tab stop, and the arrow keys, Home and End move
 * between tabs, selecting as focus moves.
 */
export default function SegmentedControl<T extends string>(props: {
  label: string;
  options: readonly SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  tabs?: SegmentTabs<T>;
  class?: string;
}) {
  function moveTab(event: KeyboardEvent): void {
    const tabs = props.tabs;
    const land = TAB_KEYS[event.key];
    if (!tabs || land === undefined) return;
    event.preventDefault();
    const at = props.options.findIndex((option) => option.id === props.value);
    const next = props.options[land(at, props.options.length)];
    if (!next) return;
    props.onChange(next.id);
    document.getElementById(tabs.tabId(next.id))?.focus();
  }

  return (
    <div
      role={props.tabs ? "tablist" : "group"}
      aria-label={props.label}
      onKeyDown={moveTab}
      class={`inline-flex rounded-lg bg-zinc-100 dark:bg-zinc-800/70 p-0.5 ${props.class ?? ""}`}
    >
      <For each={props.options}>
        {(option) => {
          const selected = () => props.value === option.id;
          return (
            <button
              role={props.tabs ? "tab" : undefined}
              id={props.tabs?.tabId(option.id)}
              aria-selected={props.tabs ? selected() : undefined}
              aria-pressed={props.tabs ? undefined : selected()}
              aria-controls={props.tabs?.panelId}
              tabIndex={props.tabs && !selected() ? -1 : undefined}
              onClick={() => props.onChange(option.id)}
              class="rounded-md px-3 py-1 text-xs font-medium transition cursor-pointer"
              classList={{
                "bg-white dark:bg-zinc-700 text-zinc-900 dark:text-zinc-50 shadow-sm":
                  selected(),
                "text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200":
                  !selected(),
              }}
            >
              {option.label}
            </button>
          );
        }}
      </For>
    </div>
  );
}
