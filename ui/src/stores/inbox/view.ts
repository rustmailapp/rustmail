import { createSignal, createMemo } from "solid-js";
import type { FilterState, MessageSummary } from "../../lib/types";
import * as api from "../../lib/api";
import { MAX_TAG_FILTERS } from "./constants";
import {
  hiddenIds,
  indexOfRow,
  letGoWhileHidden,
  listedWhole,
  messages,
  nextCursor,
  search,
  selectedId,
  setNextCursor,
  storedTotal,
} from "./rows";

export const defaultFilters: FilterState = {
  starred: false,
  unread: false,
  attachments: false,
  tags: [],
};
export const [filters, setFilters] = createSignal<FilterState>({
  ...defaultFilters,
});

function narrows(f: FilterState): boolean {
  return f.starred || f.unread || f.attachments || f.tags.length > 0;
}

export function hasActiveFilters(): boolean {
  return narrows(filters());
}

/** Whether `m` belongs in a list narrowed by `f`; any one tag is enough. */
export function matchesFilters(m: MessageSummary, f: FilterState): boolean {
  if (f.starred && !m.is_starred) return false;
  if (f.unread && m.is_read) return false;
  if (f.attachments && !m.has_attachments) return false;
  if (f.tags.length > 0 && !f.tags.some((t) => m.tags.includes(t)))
    return false;
  return true;
}

/** Whether another tag filter would take the list read past the server's cap. */
export function tagFiltersFull(): boolean {
  return filters().tags.length >= MAX_TAG_FILTERS;
}

/** The search and filters a read was issued under. */
export type View = { q: string; filters: FilterState };

/**
 * The view {@link nextCursor} was read under.
 *
 * A cursor only continues the list it came from: a search or a filter changes
 * before the read for it lands, and paging the old list under the new view
 * would append rows the new view never asked for.
 */
export let cursorView: View = { q: "", filters: defaultFilters };

export function setPageCursor(cursor: string | null, view: View): void {
  cursorView = view;
  setNextCursor(cursor);
}

export function currentView(): View {
  return { q: search(), filters: filters() };
}

export function isCurrentView(view: View): boolean {
  return view.q === search() && view.filters === filters();
}

export function viewQuery(view: View): Pick<api.ListQuery, "q" | "filters"> {
  return {
    q: view.q || undefined,
    filters: narrows(view.filters) ? view.filters : undefined,
  };
}

export const visibleMessages = createMemo(() => {
  const hidden = hiddenIds();
  if (hidden.length === 0) return messages();
  return messages().filter((m) => !hidden.includes(m.id));
});

export const total = createMemo(() => {
  const hidden = hiddenIds();
  if (hidden.length === 0) return storedTotal();
  messages();
  const hiddenCount = hidden.filter(
    (id) => indexOfRow(id) >= 0 || letGoWhileHidden.has(id),
  ).length;
  return Math.max(0, storedTotal() - hiddenCount);
});

/**
 * Whether the inbox holds no mail at all, as opposed to none matching.
 *
 * False until a read of the whole inbox lands, so the empty state neither
 * flashes up over an inbox still loading nor over leftover search results.
 */
export function inboxEmpty(): boolean {
  return (
    listedWhole() &&
    search() === "" &&
    !hasActiveFilters() &&
    visibleMessages().length === 0
  );
}

/**
 * The inbox list: loaded messages narrowed by the active filters.
 *
 * The server already narrowed what it returned; this keeps a row out once a
 * live flag change stops it matching. The unread filter alone makes an exception for the selected message.
 * Selecting marks a message read, so that one filter is invalidated by the
 * act of selecting: every keypress would drop the row it had just landed on,
 * leaving the selection outside the list and sending navigation back to the
 * top. Keeping it lets the list shrink behind the cursor instead. No other
 * filter is affected by selecting, so none of them make the exception.
 */
export const filteredMessages = createMemo(() => {
  const f = filters();
  if (!f.starred && !f.unread && !f.attachments && f.tags.length === 0) {
    return visibleMessages();
  }
  const selected = selectedId();
  return visibleMessages().filter((m) =>
    matchesFilters(m.id === selected ? { ...m, is_read: false } : m, f),
  );
});

function tagsByUse(rows: readonly MessageSummary[]): string[] {
  const counts = new Map<string, number>();
  for (const m of rows) {
    for (const t of m.tags) {
      counts.set(t, (counts.get(t) || 0) + 1);
    }
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([tag]) => tag);
}

/**
 * The tags on offer to filter by, most used first.
 *
 * A tag filter makes the server return only rows carrying one of the chosen
 * tags, so while one is set the tags seen before it stay on offer: adding a
 * second tag must not need the first one cleared.
 */
export const allTags = createMemo<string[]>((before) => {
  const loaded = tagsByUse(visibleMessages());
  if (filters().tags.length === 0) return loaded;
  const kept = new Set(before);
  return [...before, ...loaded.filter((tag) => !kept.has(tag))];
}, []);

export const hasMore = createMemo(() => nextCursor() !== null);

/**
 * How many rows the list is presenting as its set, for `aria-setsize`.
 *
 * The server total, plus the selected row when the unread filter keeps it
 * only because it is selected: it is on screen, so it is part of the set.
 */
export const listSize = createMemo(() => {
  const f = filters();
  const id = selectedId();
  if (!f.unread || id === null) return total();
  const selected = visibleMessages().find((m) => m.id === id);
  const keptBySelection =
    selected !== undefined &&
    !matchesFilters(selected, f) &&
    matchesFilters({ ...selected, is_read: false }, f);
  return total() + (keptBySelection ? 1 : 0);
});
