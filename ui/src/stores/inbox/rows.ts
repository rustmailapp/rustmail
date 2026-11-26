import { createSignal } from "solid-js";
import type { MessageSummary } from "../../lib/types";
import { MAX_LIVE_ROWS } from "./constants";

export const [messages, setMessages] = createSignal<MessageSummary[]>([]);

/**
 * Each loaded row's position; its index in {@link messages} is the position
 * minus {@link headPosition}.
 *
 * Arrivals take positions above the head, so a prepend leaves every other
 * entry where it was, and finding a row by id stays a lookup however long the
 * list grows.
 */
export const positions = new Map<string, number>();
let headPosition = 0;

export function indexOfRow(id: string): number {
  const position = positions.get(id);
  return position === undefined ? -1 : position - headPosition;
}

/** The loaded message with `id`, tracking the list like a read of it would. */
export function findMessage(id: string): MessageSummary | undefined {
  const rows = messages();
  const index = indexOfRow(id);
  return index < 0 ? undefined : rows[index];
}

/**
 * Whether live arrivals wait above the list instead of entering it.
 *
 * Set while the reader is scrolled away from the top: a prepend there would
 * slide every row they are reading down by one, hundreds of times a second.
 */
export const [liveHeld, setLiveHeldSignal] = createSignal(false);
/** Arrivals waiting for the reader to come back to the top, oldest first. */
export const heldRows = new Map<string, MessageSummary>();
/** How many arrivals are waiting, including any {@link heldRows} let go. */
export const [heldArrivals, setHeldArrivals] = createSignal(0);
/** Whether more arrived than {@link heldRows} keeps, so only a read can show them. */
export let heldOverflowed = false;
/** Whether a search went stale while held, to be read again on the way back. */
export const [heldRefresh, setHeldRefresh] = createSignal(false);

export function dropHeldArrivals(): void {
  heldRows.clear();
  heldOverflowed = false;
  setHeldArrivals(0);
  setHeldRefresh(false);
}

export function holdArrivals(arrivals: readonly MessageSummary[]): void {
  for (const arrival of arrivals) {
    if (heldOverflowed) continue;
    if (heldRows.size === MAX_LIVE_ROWS) {
      heldOverflowed = true;
      heldRows.clear();
      continue;
    }
    heldRows.set(arrival.id, arrival);
  }
  setHeldArrivals((count) => count + arrivals.length);
}

export function replaceRows(rows: MessageSummary[]): void {
  letGoWhileHidden.clear();
  setRows(rows);
}

/** Makes `rows` the loaded list, keeping the open message's row if it leaves. */
export function setRows(rows: MessageSummary[]): void {
  const selected = selectedId();
  const open = selected === null ? undefined : findMessage(selected);
  if (open !== undefined) setOpenRow(open);
  positions.clear();
  headPosition = 0;
  rows.forEach((m, i) => positions.set(m.id, i));
  setMessages(rows);
}

/** Puts `rows`, newest first, above the loaded ones. */
export function prependRows(rows: MessageSummary[]): void {
  if (rows.length === 0) return;
  headPosition -= rows.length;
  rows.forEach((m, i) => positions.set(m.id, headPosition + i));
  setMessages((prev) => [...rows, ...prev]);
}

export function appendRows(rows: MessageSummary[]): void {
  const tail = headPosition + messages().length;
  rows.forEach((m, i) => positions.set(m.id, tail + i));
  setMessages((prev) => [...prev, ...rows]);
}

export function removeRow(id: string): void {
  const index = indexOfRow(id);
  if (index < 0) return;
  const next = messages().toSpliced(index, 1);
  positions.delete(id);
  for (let i = index; i < next.length; i += 1) {
    positions.set(next[i].id, headPosition + i);
  }
  setMessages(next);
}

/**
 * Rows awaiting deletion that {@link trimRows} let go.
 *
 * The server still holds them, so its total still counts them, and the list
 * must keep counting them out until the deletion settles or is undone.
 */
export const letGoWhileHidden = new Set<string>();

/**
 * Lets the rows past {@link MAX_LIVE_ROWS} go, and pages on from the last kept.
 *
 * The row that can still be undone is kept, with every row above it, so undo
 * puts it back where it was; the undo window bounds how long the list can run
 * past the cap for it. The cursor stays in the view it was read under, since
 * the rows let go belong to that view.
 */
export function trimRows(): void {
  const rows = messages();
  const undoable = undoableId();
  const keep = Math.max(
    MAX_LIVE_ROWS,
    undoable === null ? 0 : indexOfRow(undoable) + 1,
  );
  if (rows.length <= keep) return;
  const hidden = hiddenIds();
  const selected = selectedId();
  for (const m of rows.slice(keep)) {
    positions.delete(m.id);
    if (hidden.includes(m.id)) letGoWhileHidden.add(m.id);
    if (m.id === selected) setOpenRow(m);
  }
  const kept = rows.slice(0, keep);
  setMessages(kept);
  setNextCursor(kept[kept.length - 1].id);
}

export function replaceRow(patched: MessageSummary): void {
  const index = indexOfRow(patched.id);
  if (index < 0) return;
  setMessages((prev) => prev.with(index, patched));
}
export const [storedTotal, setStoredTotal] = createSignal(0);
export const [selectedId, setSelectedId] = createSignal<string | null>(null);
/**
 * The open message's row as it was when the list let it go, kept current by
 * live events, so the detail pane never reads its state from a list that no
 * longer holds it.
 */
export const [openRow, setOpenRow] = createSignal<MessageSummary | null>(null);

/**
 * The latest known state of `loaded`, the message the detail pane opened.
 *
 * Its loaded row when there is one, else the open row the list let go, else
 * `loaded` itself as it was read.
 */
export function liveSummary(loaded: MessageSummary): MessageSummary {
  const row = findMessage(loaded.id);
  if (row !== undefined) return row;
  const open = openRow();
  return open?.id === loaded.id ? open : loaded;
}
export const [loading, setLoading] = createSignal(false);
/**
 * Whether the last list read to land was of the whole inbox, unsearched and
 * unfiltered, so an empty list then means an empty inbox.
 */
export const [listedWhole, setListedWhole] = createSignal(false);
export const [loadingMore, setLoadingMore] = createSignal(false);
export const [search, setSearch] = createSignal("");
/**
 * Where the next older page starts, or `null` once the oldest one is loaded.
 *
 * It is always the id of the last loaded row: the server hands it back with
 * each page, and a deletion of that row moves it back to the row before.
 */
export const [nextCursor, setNextCursor] = createSignal<string | null>(null);

/**
 * Messages kept out of the list while their deletion is still pending.
 *
 * The server still holds them, and a refetch puts them back in `messages`, so
 * hiding is the only thing keeping them off screen until the DELETE lands.
 */
export const [hiddenIds, setHiddenIds] = createSignal<readonly string[]>([]);
/** The one hidden message that can still be brought back. */
export const [undoableId, setUndoableId] = createSignal<string | null>(null);
