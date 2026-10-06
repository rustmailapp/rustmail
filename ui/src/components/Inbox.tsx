import {
  For,
  Show,
  createEffect,
  on,
  onMount,
  untrack,
  type Accessor,
} from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import {
  filteredMessages,
  visibleMessages,
  listSize,
  selectedId,
  selectMessage,
  starMessage,
  moveSelection,
  type SelectionTarget,
  loading,
  loadingMore,
  loadMore,
  hasActiveFilters,
  clearFilters,
  search,
  heldArrivals,
  heldRefresh,
  setLiveHeld,
} from "../stores/messages";
import { formatDate } from "../lib/format";
import { PaperclipIcon, StarIcon } from "./icons";
import type { MessageSummary } from "../lib/types";

/**
 * Starting guess for a row's height, in pixels.
 *
 * Rows are measured once rendered, so this only has to be close enough to size
 * the scrollbar before anything is on screen.
 */
const ROW_ESTIMATE_PX = 56;
const OVERSCAN_ROWS = 8;
/** Distance from the end of the loaded list that starts the next page. */
const LOAD_MORE_ROW_THRESHOLD = 10;
/**
 * How far down the list may be scrolled and still count as at the top.
 *
 * At the top, live mail enters the list as it arrives; past this it waits
 * behind the "new" pill so the rows being read stay where they are.
 */
const LIVE_TOP_TOLERANCE_PX = 4;

/**
 * Keys that move the selection when the list has focus.
 *
 * Arrow keys are bound here rather than on the document so they keep scrolling
 * the message body when focus is in the detail pane.
 */
const SELECTION_KEYS: Record<string, SelectionTarget | undefined> = {
  ArrowDown: "next",
  ArrowUp: "prev",
  Home: "first",
  End: "last",
};

function optionId(messageId: string): string {
  return `msg-option-${messageId}`;
}

export default function Inbox() {
  let scroller: HTMLDivElement | undefined;
  let listbox: HTMLDivElement | undefined;

  const virtualizer = createVirtualizer({
    get count() {
      return filteredMessages().length;
    },
    getScrollElement: () => scroller ?? null,
    getItemKey: (index) => filteredMessages()[index]?.id ?? index,
    estimateSize: () => ROW_ESTIMATE_PX,
    overscan: OVERSCAN_ROWS,
  });

  createEffect(() => {
    const items = virtualizer.getVirtualItems();
    const last = items[items.length - 1];
    if (!last) return;
    if (last.index >= filteredMessages().length - LOAD_MORE_ROW_THRESHOLD) {
      loadMore();
    }
  });

  createEffect(
    on(selectedId, (id) => {
      if (!id) return;
      const index = untrack(() =>
        filteredMessages().findIndex((m) => m.id === id),
      );
      if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
    }),
  );

  /**
   * Attaches a row to the virtualizer's measurement loop.
   *
   * The index is written here rather than as a JSX attribute so it exists
   * before `measureElement` reads it: Solid applies dynamic attributes in an
   * effect that runs after `ref`, which left every row stuck at the estimated
   * height and made `scrollToIndex` land short.
   *
   * Callers must defer this past the render pass. Measuring resizes an item,
   * which invalidates the virtualizer's measurement memo; doing that from a
   * `ref` re-enters the graph mid-render and `getVirtualItems` then yields
   * holes for indexes it has not remeasured yet.
   */
  function measureRow(el: HTMLElement, index: number): void {
    el.dataset.index = String(index);
    virtualizer.measureElement(el);
  }

  /**
   * The `aria-activedescendant` target, or nothing when it is not rendered.
   *
   * An IDREF pointing at an unmounted row is invalid, and mouse-wheel
   * scrolling unmounts the selected row without changing the selection, so
   * this has to track the rendered window rather than the selection alone.
   */
  const activeDescendant = () => {
    const id = selectedId();
    if (!id) return undefined;
    const index = filteredMessages().findIndex((m) => m.id === id);
    if (index < 0) return undefined;
    const rendered = virtualizer
      .getVirtualItems()
      .some((item) => item.index === index);
    return rendered ? optionId(id) : undefined;
  };

  function followScroll(): void {
    if (scroller) setLiveHeld(scroller.scrollTop > LIVE_TOP_TOLERANCE_PX);
  }

  function showNewest(): void {
    scroller?.scrollTo({ top: 0 });
    setLiveHeld(false);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const target = SELECTION_KEYS[e.key];
    if (!target) return;
    e.preventDefault();
    moveSelection(target);
  }

  return (
    <div
      ref={scroller}
      onScroll={followScroll}
      class="flex-1 min-h-0 flex flex-col overflow-y-auto pb-2"
    >
      <Show when={heldArrivals() > 0 || heldRefresh()}>
        <div class="sticky top-0 z-10 h-0 flex justify-center">
          <button
            onClick={showNewest}
            class="mt-2 rounded-full bg-orange-500 px-3 py-1 text-xs font-medium text-white shadow-md hover:bg-orange-400 transition cursor-pointer"
          >
            {heldArrivals() > 0 ? `${heldArrivals()} new` : "New results"}
          </button>
        </div>
      </Show>
      <Show when={!loading() && filteredMessages().length === 0}>
        <div class="flex flex-col items-center justify-center h-full text-zinc-500 dark:text-zinc-500">
          <Show
            when={visibleMessages().length === 0 && !search()}
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

      <div
        ref={listbox}
        role="listbox"
        aria-label="Messages"
        tabIndex={0}
        aria-activedescendant={activeDescendant()}
        aria-busy={loadingMore()}
        onKeyDown={handleKeyDown}
        onClick={() => listbox?.focus()}
        class="group relative w-full shrink-0 outline-none"
        style={{ height: `${virtualizer.getTotalSize()}px` }}
      >
        <For each={virtualizer.getVirtualItems()}>
          {(item) => {
            const msg = () => filteredMessages()[item.index];
            let row: HTMLDivElement | undefined;
            onMount(() => {
              if (row) measureRow(row, item.index);
            });
            return (
              <div
                role="presentation"
                ref={row}
                class="absolute top-0 left-0 w-full px-2 pb-0.5"
                style={{ transform: `translateY(${item.start}px)` }}
              >
                <Show when={msg()}>
                  {(m) => <MessageRow msg={m} index={item.index} />}
                </Show>
              </div>
            );
          }}
        </For>
      </div>

      <Show when={loadingMore()}>
        <div class="py-3 text-center text-xs text-zinc-400 dark:text-zinc-600">
          Loading…
        </div>
      </Show>
    </div>
  );
}

/** The envelope line under a row's subject: who sent it, and to whom. */
function routeLine(msg: MessageSummary): string {
  const sender = msg.sender || "(no sender)";
  const [first, ...rest] = msg.recipients;
  if (first === undefined) return sender;
  const more = rest.length > 0 ? ` +${rest.length}` : "";
  return `${sender} \u2192 ${first}${more}`;
}

function MessageRow(props: { msg: Accessor<MessageSummary>; index: number }) {
  const msg = props.msg;
  const isSelected = () => selectedId() === msg().id;
  const emphasised = () => isSelected() || !msg().is_read;

  return (
    <div
      role="option"
      id={optionId(msg().id)}
      aria-selected={isSelected()}
      aria-posinset={props.index + 1}
      aria-setsize={listSize()}
      data-id={msg().id}
      onClick={() => selectMessage(msg())}
      class={`w-full text-left rounded-lg px-2.5 py-2 transition cursor-pointer ${
        isSelected()
          ? "inbox-row-selected"
          : "hover:bg-zinc-100/80 dark:hover:bg-zinc-800/50"
      }`}
    >
      <div class="flex items-start gap-2.5">
        <div class="flex-shrink-0 flex flex-col items-center gap-1 pt-0.5">
          <div
            class={`size-2 rounded-full mt-1 ${msg().is_read ? "bg-transparent" : "bg-orange-500"}`}
          />
          <button
            tabIndex={-1}
            onClick={(e) => {
              e.stopPropagation();
              starMessage(msg().id, !msg().is_starred);
            }}
            class="cursor-pointer"
            title={msg().is_starred ? "Unstar" : "Star"}
          >
            <StarIcon
              class={`size-3.5 transition ${msg().is_starred ? "text-amber-400" : "text-zinc-300 dark:text-zinc-600 hover:text-amber-400"}`}
              filled={msg().is_starred}
            />
          </button>
        </div>
        <div class="flex-1 min-w-0">
          <div class="flex items-baseline justify-between gap-2">
            <span
              class={`text-sm leading-5 truncate ${emphasised() ? "font-semibold text-zinc-900 dark:text-zinc-50" : "text-zinc-600 dark:text-zinc-400"}`}
            >
              {msg().subject || "(no subject)"}
            </span>
            <span
              class={`text-[11px] leading-4 tabular-nums flex-shrink-0 ${isSelected() ? "text-zinc-600 dark:text-zinc-300" : "text-zinc-400 dark:text-zinc-500"}`}
            >
              {formatDate(msg().created_at)}
            </span>
          </div>
          <div class="flex items-center gap-1.5 mt-0.5">
            <span
              class={`text-xs leading-4 truncate min-w-0 ${isSelected() ? "text-zinc-600 dark:text-zinc-300" : "text-zinc-500 dark:text-zinc-500"}`}
            >
              {routeLine(msg())}
            </span>
            <Show when={msg().has_attachments}>
              <PaperclipIcon class="size-3 flex-shrink-0 text-zinc-400 dark:text-zinc-500" />
            </Show>
            <Show when={msg().tags.length > 0}>
              <div class="ml-auto flex gap-1 flex-shrink-0">
                <For each={msg().tags.slice(0, 3)}>
                  {(tag) => (
                    <span class="inline-block px-1.5 rounded-md text-[10px] font-medium bg-orange-100 text-orange-700 dark:bg-orange-900/40 dark:text-orange-300">
                      {tag}
                    </span>
                  )}
                </For>
                <Show when={msg().tags.length > 3}>
                  <span class="text-[10px] text-zinc-400">
                    +{msg().tags.length - 3}
                  </span>
                </Show>
              </div>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
}
