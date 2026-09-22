import { batch, createSignal, createMemo } from "solid-js";
import type { ConfirmDialogOptions } from "../components/ConfirmDialog";
import type {
  FilterState,
  ListResponse,
  MessageSummary,
  WsEvent,
} from "../lib/types";
import * as api from "../lib/api";
import * as schema from "../lib/schema";
import { notify } from "./notices";

const PAGE_SIZE = 100;

/**
 * The most rows live mail keeps loaded at the top of the list.
 *
 * Each arrival pushes the oldest loaded row out past this, and scrolling down
 * reads it back from the server, so memory and every per-frame pass over the
 * list stay bounded however long the traffic runs. Only the reader scrolling
 * down loads past it.
 */
const MAX_LIVE_ROWS = 5 * PAGE_SIZE;

/** The most `tag` filters `GET /messages` accepts; one more is a `400`. */
const MAX_TAG_FILTERS = 20;

/** The `code` `GET /messages` rejects a `before` it does not know with. */
const UNKNOWN_CURSOR_CODE = "unknown_cursor";

/**
 * How many times a list read is retried when a deletion confirms mid-flight.
 *
 * A page read while a DELETE was in flight may have been served either side of
 * the commit, and only a fresh read settles it. Retention purges a whole batch
 * one event at a time, so the races can arrive in bursts and the retry needs a
 * floor: the last attempt takes the page it got and leaves the count to
 * {@link refreshTotal}, rather than reading again for as long as messages keep
 * disappearing.
 */
const LIST_READ_ATTEMPTS = 3;

/** How much of a subject a notice quotes before trimming it. */
const NOTICE_SUBJECT_MAX = 50;

/**
 * How long a deleted message can be brought back.
 *
 * It is the DELETE that waits out the window, not the row: the message leaves
 * the list on the keystroke, so undo costs nothing but dropping a timer.
 */
const UNDO_WINDOW_MS = 5000;

/**
 * The most often live mail refetches an active search.
 *
 * Every arrival makes the results stale, and at hundreds per second a refetch
 * each would never let one land while flooding the server with full-text
 * queries.
 */
const SEARCH_REFRESH_WINDOW_MS = 1000;

/**
 * The most live events held for the next frame.
 *
 * Only a tab that gets no frames, one in the background, comes near this; a
 * backlog that long is cheaper to replace with one list read than to replay.
 */
const MAX_QUEUED_EVENTS = 10_000;

/**
 * How long a queued event waits for an animation frame before it is applied
 * anyway.
 *
 * A hidden tab gets no frames at all, and its list must still be current when
 * it comes back.
 */
const FRAME_FALLBACK_MS = 250;

const [messages, setMessages] = createSignal<MessageSummary[]>([]);

/**
 * Each loaded row's position; its index in {@link messages} is the position
 * minus {@link headPosition}.
 *
 * Arrivals take positions above the head, so a prepend leaves every other
 * entry where it was, and finding a row by id stays a lookup however long the
 * list grows.
 */
const positions = new Map<string, number>();
let headPosition = 0;

function indexOfRow(id: string): number {
  const position = positions.get(id);
  return position === undefined ? -1 : position - headPosition;
}

/** The loaded message with `id`, tracking the list like a read of it would. */
function findMessage(id: string): MessageSummary | undefined {
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
const [liveHeld, setLiveHeldSignal] = createSignal(false);
/** Arrivals waiting for the reader to come back to the top, oldest first. */
const heldRows = new Map<string, MessageSummary>();
/** How many arrivals are waiting, including any {@link heldRows} let go. */
const [heldArrivals, setHeldArrivals] = createSignal(0);
/** Whether more arrived than {@link heldRows} keeps, so only a read can show them. */
let heldOverflowed = false;
/** Whether a search went stale while held, to be read again on the way back. */
const [heldRefresh, setHeldRefresh] = createSignal(false);

function dropHeldArrivals(): void {
  heldRows.clear();
  heldOverflowed = false;
  setHeldArrivals(0);
  setHeldRefresh(false);
}

function holdArrivals(arrivals: readonly MessageSummary[]): void {
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

/**
 * Holds live arrivals back while `held`, and shows what waited once it is not.
 *
 * The list tells the store where the reader is; the store decides nothing
 * from scroll positions itself.
 */
function setLiveHeld(held: boolean): void {
  if (held === liveHeld()) return;
  setLiveHeldSignal(held);
  if (!held) revealArrivals();
}

/**
 * Puts the waiting arrivals on top, or reads the first page again when what
 * waited cannot be shown from memory: a stale search, or more than was kept.
 */
function revealArrivals(): void {
  const refresh = heldRefresh() || heldOverflowed;
  const waiting = [...heldRows.values()].reverse();
  if (!refresh && waiting.length === 0) return;
  batch(() => {
    dropHeldArrivals();
    if (!refresh) {
      prependRows(waiting.filter((m) => !positions.has(m.id)));
      trimRows();
    }
  });
  if (refresh) {
    fetchMessages().catch(() => notify("Could not load the new messages."));
  }
}

function replaceRows(rows: MessageSummary[]): void {
  const selected = selectedId();
  const open = selected === null ? undefined : findMessage(selected);
  if (open !== undefined) setOpenRow(open);
  letGoWhileHidden.clear();
  positions.clear();
  headPosition = 0;
  rows.forEach((m, i) => positions.set(m.id, i));
  setMessages(rows);
}

/** Puts `rows`, newest first, above the loaded ones. */
function prependRows(rows: MessageSummary[]): void {
  if (rows.length === 0) return;
  headPosition -= rows.length;
  rows.forEach((m, i) => positions.set(m.id, headPosition + i));
  setMessages((prev) => [...rows, ...prev]);
}

function appendRows(rows: MessageSummary[]): void {
  const tail = headPosition + messages().length;
  rows.forEach((m, i) => positions.set(m.id, tail + i));
  setMessages((prev) => [...prev, ...rows]);
}

function removeRow(id: string): void {
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
const letGoWhileHidden = new Set<string>();

/**
 * Lets the rows past {@link MAX_LIVE_ROWS} go, and pages on from the last kept.
 *
 * The row that can still be undone is kept, with every row above it, so undo
 * puts it back where it was; the undo window bounds how long the list can run
 * past the cap for it. The cursor stays in the view it was read under, since
 * the rows let go belong to that view.
 */
function trimRows(): void {
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

function replaceRow(patched: MessageSummary): void {
  const index = indexOfRow(patched.id);
  if (index < 0) return;
  setMessages((prev) => prev.with(index, patched));
}
const [storedTotal, setStoredTotal] = createSignal(0);
const [selectedId, setSelectedId] = createSignal<string | null>(null);
/**
 * The open message's row as it was when the list let it go, kept current by
 * live events, so the detail pane never reads its state from a list that no
 * longer holds it.
 */
const [openRow, setOpenRow] = createSignal<MessageSummary | null>(null);

/**
 * The latest known state of `loaded`, the message the detail pane opened.
 *
 * Its loaded row when there is one, else the open row the list let go, else
 * `loaded` itself as it was read.
 */
function liveSummary(loaded: MessageSummary): MessageSummary {
  const row = findMessage(loaded.id);
  if (row !== undefined) return row;
  const open = openRow();
  return open?.id === loaded.id ? open : loaded;
}
const [loading, setLoading] = createSignal(false);
const [loadingMore, setLoadingMore] = createSignal(false);
const [search, setSearch] = createSignal("");
/**
 * Where the next older page starts, or `null` once the oldest one is loaded.
 *
 * It is always the id of the last loaded row: the server hands it back with
 * each page, and a deletion of that row moves it back to the row before.
 */
const [nextCursor, setNextCursor] = createSignal<string | null>(null);

/**
 * Messages kept out of the list while their deletion is still pending.
 *
 * The server still holds them, and a refetch puts them back in `messages`, so
 * hiding is the only thing keeping them off screen until the DELETE lands.
 */
const [hiddenIds, setHiddenIds] = createSignal<readonly string[]>([]);
/** The one hidden message that can still be brought back. */
const [undoableId, setUndoableId] = createSignal<string | null>(null);
let undoTimer: ReturnType<typeof setTimeout> | null = null;

const defaultFilters: FilterState = {
  starred: false,
  unread: false,
  attachments: false,
  tags: [],
};
const [filters, setFilters] = createSignal<FilterState>({ ...defaultFilters });

function narrows(f: FilterState): boolean {
  return f.starred || f.unread || f.attachments || f.tags.length > 0;
}

function hasActiveFilters(): boolean {
  return narrows(filters());
}

/** Whether `m` belongs in a list narrowed by `f`; any one tag is enough. */
function matchesFilters(m: MessageSummary, f: FilterState): boolean {
  if (f.starred && !m.is_starred) return false;
  if (f.unread && m.is_read) return false;
  if (f.attachments && !m.has_attachments) return false;
  if (f.tags.length > 0 && !f.tags.some((t) => m.tags.includes(t)))
    return false;
  return true;
}

/**
 * Switches to `next` filters and reads the list under them.
 *
 * The server does the narrowing, so a sparse filter over a large inbox costs
 * one read rather than a walk through every page.
 */
function applyFilters(next: FilterState): void {
  setFilters(next);
  fetchMessages().catch(() => notify("Could not apply the filters."));
}

function clearFilters(): void {
  if (!hasActiveFilters()) return;
  applyFilters({ ...defaultFilters });
}

function clearTagFilters(): void {
  if (filters().tags.length === 0) return;
  applyFilters({ ...filters(), tags: [] });
}

function toggleFilter(key: "starred" | "unread" | "attachments"): void {
  const f = filters();
  applyFilters({ ...f, [key]: !f[key] });
}

/** Whether another tag filter would take the list read past the server's cap. */
function tagFiltersFull(): boolean {
  return filters().tags.length >= MAX_TAG_FILTERS;
}

function toggleTagFilter(tag: string): void {
  const f = filters();
  if (!f.tags.includes(tag) && tagFiltersFull()) return;
  applyFilters({
    ...f,
    tags: f.tags.includes(tag)
      ? f.tags.filter((t) => t !== tag)
      : [...f.tags, tag],
  });
}

/** The search and filters a read was issued under. */
type View = { q: string; filters: FilterState };

/**
 * The view {@link nextCursor} was read under.
 *
 * A cursor only continues the list it came from: a search or a filter changes
 * before the read for it lands, and paging the old list under the new view
 * would append rows the new view never asked for.
 */
let cursorView: View = { q: "", filters: defaultFilters };

function setPageCursor(cursor: string | null, view: View): void {
  cursorView = view;
  setNextCursor(cursor);
}

function currentView(): View {
  return { q: search(), filters: filters() };
}

function isCurrentView(view: View): boolean {
  return view.q === search() && view.filters === filters();
}

function viewQuery(view: View): Pick<api.ListQuery, "q" | "filters"> {
  return {
    q: view.q || undefined,
    filters: narrows(view.filters) ? view.filters : undefined,
  };
}

const visibleMessages = createMemo(() => {
  const hidden = hiddenIds();
  if (hidden.length === 0) return messages();
  return messages().filter((m) => !hidden.includes(m.id));
});

const total = createMemo(() => {
  const hidden = hiddenIds();
  if (hidden.length === 0) return storedTotal();
  messages();
  const hiddenCount = hidden.filter(
    (id) => indexOfRow(id) >= 0 || letGoWhileHidden.has(id),
  ).length;
  return Math.max(0, storedTotal() - hiddenCount);
});

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
const filteredMessages = createMemo(() => {
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
const allTags = createMemo<string[]>((before) => {
  const loaded = tagsByUse(visibleMessages());
  if (filters().tags.length === 0) return loaded;
  const kept = new Set(before);
  return [...before, ...loaded.filter((tag) => !kept.has(tag))];
}, []);

const hasMore = createMemo(() => nextCursor() !== null);

/**
 * How many rows the list is presenting as its set, for `aria-setsize`.
 *
 * The server total, plus the selected row when the unread filter keeps it
 * only because it is selected: it is on screen, so it is part of the set.
 */
const listSize = createMemo(() => {
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

/** Where {@link moveSelection} should land, relative to the current selection. */
type SelectionTarget = "next" | "prev" | "first" | "last";

/**
 * Selects a message and marks it read.
 *
 * The read flag is written optimistically: the WebSocket `message:read` event
 * is what syncs the list, so a failed write only costs the unread dot.
 */
function selectMessage(msg: MessageSummary): void {
  setSelectedId(msg.id);
  if (!msg.is_read) {
    api
      .markRead(msg.id, true)
      .catch(() => notify("Could not mark the message as read."));
  }
}

/**
 * How a message reads inside a notice.
 *
 * Only writes the user aimed at one message name it. Marking read fires once
 * per row while the selection walks, so naming the subject there would defeat
 * the deduplication and bury the screen where one sentence does the job; a
 * star or a delete is a single deliberate act, and "which one" is the part
 * worth saying.
 */
function quoted(id: string): string {
  const subject = findMessage(id)?.subject?.trim();
  if (!subject) return "the message";

  const characters = [...subject];
  if (characters.length <= NOTICE_SUBJECT_MAX) return `“${subject}”`;
  return `“${characters.slice(0, NOTICE_SUBJECT_MAX).join("")}…”`;
}

/** What a whole-inbox delete asks before it runs. */
function clearInboxPrompt(): ConfirmDialogOptions {
  return {
    title: "Clear all messages",
    message: `All ${total()} messages will be permanently deleted.`,
    confirmLabel: "Clear all",
  };
}

/** Deletes every message, saying so when the write does not land. */
async function clearInbox(): Promise<void> {
  try {
    await api.deleteAllMessages();
  } catch {
    notify("Could not clear the inbox. The messages are still here.");
  }
}

/** Stars or unstars a message, saying so when the write does not land. */
function starMessage(id: string, starred: boolean): void {
  const label = quoted(id);
  api
    .markStarred(id, starred)
    .catch(() =>
      notify(
        starred ? `Could not star ${label}.` : `Could not unstar ${label}.`,
      ),
    );
}

function clearUndoTimer(): void {
  if (undoTimer !== null) {
    clearTimeout(undoTimer);
    undoTimer = null;
  }
}

function unhide(id: string): void {
  letGoWhileHidden.delete(id);
  setHiddenIds((ids) => ids.filter((hidden) => hidden !== id));
}

/** Identifies the latest server count applied to the list. */
let snapshot = 0;
/** Invalidates reads started before a confirmed deletion or an inbox clear. */
let deletionRevision = 0;
/** Distinguishes deletions made before the entire inbox was cleared. */
let clearRevision = 0;
let latestFetch = 0;
let currentListRead: AbortController | null = null;
let searchStale = false;
let searchRefreshTimer: ReturnType<typeof setTimeout> | null = null;
/** Live events applied while the current list read is in flight. */
let eventsDuringRead: ReplayableEvent[] | null = null;
let latestCountRead = 0;
let countNeedsRefresh = false;

/** A local deletion and the count against which it was issued. */
type IssuedDelete = {
  snapshot: number;
  clearRevision: number;
  confirmed: boolean;
};

/** Local deletions awaiting their HTTP response or WebSocket echo. */
const issuedDeletes = new Map<string, IssuedDelete>();

/** Removes a confirmed deletion without assuming the current total includes it. */
function forgetMessage(id: string): void {
  if (nextCursor() === id) {
    setNextCursor(messages()[indexOfRow(id) - 1]?.id ?? null);
  }
  removeRow(id);
  if (openRow()?.id === id) setOpenRow(null);
  if (heldRows.delete(id)) setHeldArrivals((count) => count - 1);
  unhide(id);
  if (selectedId() === id) setSelectedId(null);
}

/**
 * Reconciles an ambiguous count without replacing the pages already loaded.
 *
 * A page fetched during a DELETE may have read before or after the commit.
 * Only a new read after confirmation can settle that ambiguity. A newer
 * snapshot, deletion, clear or search supersedes this count request.
 */
async function refreshTotal(): Promise<void> {
  const request = ++latestCountRead;
  const currentSnapshot = snapshot;
  const currentRevision = deletionRevision;
  const view = currentView();
  try {
    const response = await api.listMessages({ limit: 1, ...viewQuery(view) });
    if (
      request !== latestCountRead ||
      currentSnapshot !== snapshot ||
      currentRevision !== deletionRevision ||
      !isCurrentView(view)
    )
      return;
    snapshot += 1;
    countNeedsRefresh = false;
    setStoredTotal(response.total);
  } catch {
    if (
      request === latestCountRead &&
      currentSnapshot === snapshot &&
      currentRevision === deletionRevision &&
      isCurrentView(view)
    ) {
      notify(
        "The message was deleted, but the inbox count could not be refreshed.",
      );
    }
  }
}

/** Applies either confirmation once, removing the row regardless of the snapshot. */
function reconcileDeletion(id: string): void {
  const issued = issuedDeletes.get(id);
  if (issued?.confirmed === true) {
    issuedDeletes.delete(id);
    return;
  }

  const cleared =
    issued !== undefined && issued.clearRevision !== clearRevision;
  const countIsCurrent =
    !countNeedsRefresh &&
    (issued === undefined || issued.snapshot === snapshot);
  if (issued) issued.confirmed = true;
  if (!cleared) {
    deletionRevision += 1;
    if (!countIsCurrent) countNeedsRefresh = true;
  }
  batch(() => {
    forgetMessage(id);
    if (!cleared && countIsCurrent) {
      setStoredTotal((current) => Math.max(0, current - 1));
    }
  });
  if (!cleared && !countIsCurrent) void refreshTotal();
}

/**
 * Drops the undo offer for a message the server has already deleted.
 *
 * Another client, or the retention sweep, can delete a message inside its undo
 * window. Bringing it back is no longer possible, and the DELETE the timer
 * would fire has nothing left to delete.
 */
function cancelUndo(id: string): void {
  if (undoableId() !== id) return;
  clearUndoTimer();
  setUndoableId(null);
}

/**
 * Takes `id` out of the list, and deletes it once its undo window closes.
 *
 * Only the newest deletion is undoable: a second one commits the one before
 * it, so undo always means the message that just disappeared.
 */
function deleteWithUndo(id: string): void {
  commitDelete();
  setHiddenIds((ids) => (ids.includes(id) ? ids : [...ids, id]));
  setUndoableId(id);
  undoTimer = setTimeout(() => commitDelete(), UNDO_WINDOW_MS);
}

/**
 * Issues the DELETE for the message waiting on one, closing its undo window.
 *
 * The message stays hidden until the write settles, so the row does not come
 * back for a round trip on the way out. A settled write then removes it
 * outright instead of trusting the event to arrive and do it: a message the
 * server has confirmed gone must not reappear because its event is late or its
 * socket is down. A failed write unhides instead, since the server still holds
 * the message — unless the event got there first, which means the deletion did
 * happen and only the response was lost.
 */
function commitDelete(): void {
  const id = undoableId();
  clearUndoTimer();
  setUndoableId(null);
  if (id === null) return;

  const label = quoted(id);
  issuedDeletes.set(id, { snapshot, clearRevision, confirmed: false });
  api.deleteMessage(id).then(
    () => reconcileDeletion(id),
    () => {
      const issued = issuedDeletes.get(id);
      issuedDeletes.delete(id);
      if (issued?.confirmed || issued?.clearRevision !== clearRevision) return;
      unhide(id);
      notify(`Could not delete ${label}. It is back in the inbox.`);
    },
  );
}

/**
 * Commits the still-undoable deletion before the page goes away.
 *
 * A deletion already issued outlives the document on its own, because every
 * DELETE carries `keepalive`. The one waiting out its undo window has not been
 * sent yet, and this is its last chance to go.
 */
function flushPendingDelete(): void {
  commitDelete();
}

/** Brings the last deleted message back, while its window is still open. */
function undoDelete(): void {
  const id = undoableId();
  clearUndoTimer();
  setUndoableId(null);
  if (id === null) return;

  unhide(id);
  setSelectedId(id);
}

/**
 * Forgets every pending deletion, for when the whole store is gone anyway.
 *
 * The issued deletions outlive this: their responses are still on the way, and
 * the entry is what tells a late one that the count it would adjust is no
 * longer the count it was issued against.
 */
function dropPendingDeletes(): void {
  clearUndoTimer();
  setUndoableId(null);
  setHiddenIds([]);
  snapshot += 1;
  deletionRevision += 1;
  clearRevision += 1;
  countNeedsRefresh = false;
}

function targetIndex(
  to: SelectionTarget,
  current: number,
  count: number,
): number {
  switch (to) {
    case "first":
      return 0;
    case "last":
      return count - 1;
    case "next":
      return current < 0 ? 0 : Math.min(current + 1, count - 1);
    case "prev":
      return current < 0 ? 0 : Math.max(current - 1, 0);
  }
}

/**
 * Moves the selection within {@link filteredMessages}.
 *
 * Navigation is index-based rather than DOM-based so it stays correct while the
 * inbox list is virtualized and most rows are unmounted. `last` lands on the
 * last *loaded* message, not the last on the server.
 */
function moveSelection(to: SelectionTarget): void {
  const msgs = filteredMessages();
  if (msgs.length === 0) return;

  const current = msgs.findIndex((m) => m.id === selectedId());
  const index = targetIndex(to, current, msgs.length);
  if (index === current) return;

  const msg = msgs[index];
  if (msg) selectMessage(msg);
}

/**
 * Reads the first page again, replacing the list.
 *
 * Live events that arrive while the read is in flight are replayed over the
 * page it returns, since the server may have answered from before them.
 * Starting a read aborts the one before it, which then resolves quietly: its
 * answer would be dropped anyway, and only the current read's failure is worth
 * reporting.
 */
async function fetchMessages(): Promise<void> {
  await readFirstPage();
}

/** Does the work of {@link fetchMessages}, resolving whether its page landed. */
async function readFirstPage(): Promise<boolean> {
  const request = ++latestFetch;
  const view = currentView();
  currentListRead?.abort();
  const controller = new AbortController();
  currentListRead = controller;
  setLoading(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (request !== latestFetch || !isCurrentView(view)) return false;
      const revision = deletionRevision;
      eventsDuringRead = [];
      const res = await api.listMessages(
        { limit: PAGE_SIZE, ...viewQuery(view) },
        controller.signal,
      );
      if (request !== latestFetch || !isCurrentView(view)) return false;
      const raced = revision !== deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      batch(() => {
        snapshot += 1;
        countNeedsRefresh = raced;
        replaceRows(res.messages);
        dropHeldArrivals();
        setPageCursor(res.next_cursor, view);
        setStoredTotal(res.total);
        searchStale = false;
        applyEvents(eventsDuringRead ?? []);
      });
      if (raced) void refreshTotal();
      return true;
    }
    return false;
  } catch (error) {
    if (request === latestFetch) throw error;
    return false;
  } finally {
    if (request === latestFetch) {
      currentListRead = null;
      eventsDuringRead = null;
      setLoading(false);
      if (searchStale) scheduleSearchRefresh();
    }
  }
}

/**
 * Marks the search results stale and refetches them once the window closes.
 *
 * A search cannot place a live message itself, since only the server knows
 * whether it matches, so arrivals are folded into one trailing read per
 * {@link SEARCH_REFRESH_WINDOW_MS}. A read still in flight when the window
 * closes is left alone; the stale flag outlives it and schedules the next.
 */
function scheduleSearchRefresh(): void {
  searchStale = true;
  if (searchRefreshTimer !== null) return;
  searchRefreshTimer = setTimeout(() => {
    searchRefreshTimer = null;
    if (!searchStale || !search() || loading()) return;
    if (liveHeld()) {
      setHeldRefresh(true);
      return;
    }
    fetchMessages().catch(() =>
      notify("Could not refresh the search results."),
    );
  }, SEARCH_REFRESH_WINDOW_MS);
}

/**
 * Reads the page older than the last loaded row, and appends it.
 *
 * A cursor the server no longer knows belongs to a message deleted before its
 * event got here, so the row goes and the read starts again from the one
 * before it. A page read from a cursor that has since moved, because its row
 * was deleted or let go, would not join the rows above it, so it is read again
 * from where the list now ends. Any other failure is reported rather than
 * thrown, since the list's scroll position is what calls this.
 */
async function loadMore(): Promise<void> {
  if (loading() || loadingMore() || !hasMore()) return;
  if (!isCurrentView(cursorView)) return;
  const view = currentView();
  const startedOn = latestFetch;
  setLoadingMore(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (!isCurrentView(view) || startedOn !== latestFetch) return;
      const before = nextCursor();
      if (before === null) return;
      const revision = deletionRevision;
      let res: ListResponse;
      try {
        res = await api.listMessages({
          limit: PAGE_SIZE,
          ...viewQuery(view),
          before,
        });
      } catch (error) {
        if (!isUnknownCursor(error)) throw error;
        if (!isCurrentView(view) || startedOn !== latestFetch) return;
        forgetMessage(before);
        continue;
      }
      if (!isCurrentView(view) || startedOn !== latestFetch) return;
      if (nextCursor() !== before) continue;
      const raced = revision !== deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      batch(() => {
        snapshot += 1;
        countNeedsRefresh = raced;
        appendRows(res.messages.filter((m) => !positions.has(m.id)));
        setPageCursor(res.next_cursor, view);
        setStoredTotal(res.total);
      });
      if (raced) void refreshTotal();
      return;
    }
  } catch {
    if (isCurrentView(view) && startedOn === latestFetch) {
      notify("Could not load older messages.");
    }
  } finally {
    setLoadingMore(false);
  }
}

function isUnknownCursor(error: unknown): boolean {
  return error instanceof api.ApiError && error.code === UNKNOWN_CURSOR_CODE;
}

const RECONNECT_BASE_DELAY = 2000;
const MAX_RECONNECT_DELAY = 30000;
let reconnectDelay = RECONNECT_BASE_DELAY;
let currentWs: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * How long the first socket may take to open before the list is read over
 * HTTP instead.
 *
 * A proxy that drops the upgrade leaves the socket pending or failing forever,
 * and the inbox must still load without live updates.
 */
const SOCKET_OPEN_DEADLINE_MS = 3000;
let httpFallbackTimer: ReturnType<typeof setTimeout> | null = null;
let inboxLoaded = false;
let onFirstSync: (() => void) | null = null;

/**
 * The event a frame carries, or `undefined` if it carries nothing usable.
 *
 * A frame reaches the store as text nobody has checked, and the list is built
 * straight out of it — `message:new` is appended as a message. The frame never
 * reaches the log either: it holds a sender, its recipients and a subject.
 */
function readEvent(frame: unknown): WsEvent | undefined {
  if (typeof frame !== "string") return undefined;

  let body: unknown;
  try {
    body = JSON.parse(frame);
  } catch {
    return undefined;
  }

  const parsed = schema.wsEvent.safeParse(body);
  return parsed.success ? parsed.data : undefined;
}

/**
 * Puts live messages, oldest first as they arrived, at the top of the list and
 * counts them in.
 *
 * A search cannot place an arrival itself, since only the server knows whether
 * it matches, so it schedules a refetch instead; filters are decided here. The
 * socket is subscribed before a list read lands, over another connection, so a
 * message can reach the store both ways. The copy that comes second must add
 * neither a row nor a count.
 */
function admitArrivals(arrivals: readonly MessageSummary[]): void {
  if (arrivals.length === 0) return;
  if (search()) {
    scheduleSearchRefresh();
    return;
  }
  const f = filters();
  const seen = new Set<string>();
  const fresh: MessageSummary[] = [];
  for (let i = arrivals.length - 1; i >= 0; i -= 1) {
    const arrival = arrivals[i];
    if (seen.has(arrival.id) || isKnown(arrival.id)) continue;
    seen.add(arrival.id);
    if (matchesFilters(arrival, f)) fresh.push(arrival);
  }
  if (fresh.length === 0) return;
  batch(() => {
    if (liveHeld()) {
      holdArrivals(fresh.toReversed());
    } else {
      prependRows(fresh);
      trimRows();
    }
    setStoredTotal((t) => t + fresh.length);
  });
  if (countNeedsRefresh) void refreshTotal();
}

function isKnown(id: string): boolean {
  return positions.has(id) || heldRows.has(id);
}

/**
 * Changes a loaded or held message, and counts it in or out of a filtered
 * total. A held arrival that stops matching is let go, since the list it waits
 * to join would not show it.
 *
 * Deciding from the row's state rather than from the event keeps this
 * idempotent, so a replay over a page that already reflects it counts nothing.
 */
function patchMessage(id: string, patch: Partial<MessageSummary>): void {
  const open = openRow();
  if (open?.id === id) setOpenRow({ ...open, ...patch });
  const waiting = heldRows.get(id);
  if (waiting !== undefined) {
    const patched = { ...waiting, ...patch };
    if (matchesFilters(patched, filters())) {
      heldRows.set(id, patched);
      return;
    }
    heldRows.delete(id);
    batch(() => {
      setHeldArrivals((count) => count - 1);
      setStoredTotal((t) => Math.max(0, t - 1));
    });
    return;
  }
  const current = findMessage(id);
  if (current === undefined) return;
  const patched = { ...current, ...patch };
  const f = filters();
  const was = matchesFilters(current, f);
  const is = matchesFilters(patched, f);
  batch(() => {
    replaceRow(patched);
    if (was !== is) setStoredTotal((t) => Math.max(0, t + (is ? 1 : -1)));
  });
}

/** The events a list read may have been served without. */
type ReplayableEvent = Extract<
  WsEvent,
  { type: "message:new" | "message:read" | "message:starred" | "message:tags" }
>;

function isReplayable(event: WsEvent): event is ReplayableEvent {
  return (
    event.type === "message:new" ||
    event.type === "message:read" ||
    event.type === "message:starred" ||
    event.type === "message:tags"
  );
}

/**
 * Applies `events` in order, each run of arrivals as one prepend.
 *
 * Applying a replayable event twice is harmless, which is what lets
 * {@link fetchMessages} replay the ones that arrived while its read was in
 * flight.
 */
function applyEvents(events: readonly WsEvent[]): void {
  let arrivals: MessageSummary[] = [];
  for (const event of events) {
    if (event.type === "message:new") {
      arrivals.push(event.data);
      continue;
    }
    admitArrivals(arrivals);
    arrivals = [];
    applyEvent(event);
  }
  admitArrivals(arrivals);
}

function applyEvent(event: Exclude<WsEvent, { type: "message:new" }>): void {
  switch (event.type) {
    case "message:read":
      patchMessage(event.data.id, { is_read: event.data.is_read });
      break;
    case "message:starred":
      patchMessage(event.data.id, { is_starred: event.data.is_starred });
      break;
    case "message:tags":
      patchMessage(event.data.id, { tags: event.data.tags });
      break;
    case "message:delete":
      cancelUndo(event.data.id);
      reconcileDeletion(event.data.id);
      break;
    case "messages:clear":
      batch(() => {
        dropPendingDeletes();
        dropHeldArrivals();
        replaceRows([]);
        setNextCursor(null);
        setStoredTotal(0);
        setSelectedId(null);
      });
      break;
  }
}

let queuedEvents: WsEvent[] = [];
let queueOverflowed = false;
let frameRequest: number | null = null;
let frameFallback: ReturnType<typeof setTimeout> | null = null;

/**
 * Holds a live event for the next frame.
 *
 * At hundreds of events a second, applying each as it lands would recompute
 * the list, its filters and its tags that many times between two paints.
 */
function enqueueEvent(event: WsEvent): void {
  if (queuedEvents.length >= MAX_QUEUED_EVENTS) {
    queueOverflowed = true;
    queuedEvents = [];
  }
  if (!queueOverflowed) queuedEvents.push(event);
  if (frameFallback !== null) return;
  frameRequest = globalThis.requestAnimationFrame?.(flushEvents) ?? null;
  frameFallback = setTimeout(flushEvents, FRAME_FALLBACK_MS);
}

function cancelFlush(): void {
  if (frameRequest !== null) {
    globalThis.cancelAnimationFrame?.(frameRequest);
    frameRequest = null;
  }
  if (frameFallback !== null) {
    clearTimeout(frameFallback);
    frameFallback = null;
  }
}

/** Applies the events queued since the last frame, in one batch. */
function flushEvents(): void {
  cancelFlush();
  const events = queuedEvents;
  queuedEvents = [];
  if (queueOverflowed) {
    queueOverflowed = false;
    fetchMessages().catch(() => notify("Could not catch up with live mail."));
    return;
  }
  if (eventsDuringRead !== null) {
    eventsDuringRead.push(...events.filter(isReplayable));
  }
  batch(() => applyEvents(events));
}

function dropQueuedEvents(): void {
  cancelFlush();
  queuedEvents = [];
  queueOverflowed = false;
}

/**
 * Opens the live connection, and reads the list each time it opens.
 *
 * The read comes after the open, the first time included: anything stored
 * before the socket subscribed is only in the list the server returns, and
 * anything after it is on the socket. A socket that fails before it first
 * opens, or misses {@link SOCKET_OPEN_DEADLINE_MS}, gets the list read over
 * HTTP once instead. `onSynced` runs once, after the first read that lands.
 */
function connectWebSocket(onSynced?: () => void): WebSocket {
  stopHttpFallback();
  inboxLoaded = false;
  onFirstSync = onSynced ?? null;
  httpFallbackTimer = setTimeout(loadOverHttp, SOCKET_OPEN_DEADLINE_MS);
  return openSocket();
}

function stopHttpFallback(): void {
  if (httpFallbackTimer !== null) {
    clearTimeout(httpFallbackTimer);
    httpFallbackTimer = null;
  }
}

function loadOverHttp(): void {
  if (httpFallbackTimer === null) return;
  stopHttpFallback();
  syncList();
}

function syncList(): void {
  const failure = inboxLoaded
    ? "Reconnected, but could not reload the inbox."
    : "Could not load the inbox.";
  readFirstPage().then(
    (landed) => {
      if (!landed) return;
      inboxLoaded = true;
      const synced = onFirstSync;
      onFirstSync = null;
      synced?.();
    },
    () => notify(failure),
  );
}

function openSocket(): WebSocket {
  closeSocket();

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/api/v1/ws`);
  currentWs = ws;
  let opened = false;

  ws.onopen = () => {
    opened = true;
    stopHttpFallback();
    reconnectDelay = RECONNECT_BASE_DELAY;
    syncList();
  };

  ws.onmessage = (e) => {
    const event = readEvent(e.data);
    if (event === undefined) {
      console.error("Discarded a WebSocket frame the UI could not read");
      return;
    }

    enqueueEvent(event);
  };

  ws.onclose = () => {
    currentWs = null;
    if (!opened) loadOverHttp();
    const jitter = reconnectDelay * (0.5 + Math.random() * 0.5);
    reconnectTimer = setTimeout(openSocket, jitter);
    reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY);
  };

  return ws;
}

function closeSocket(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (currentWs) {
    currentWs.onclose = null;
    currentWs.close();
    currentWs = null;
  }
}

function disconnectWebSocket(): void {
  stopHttpFallback();
  closeSocket();
  dropQueuedEvents();
  latestFetch += 1;
  currentListRead?.abort();
  currentListRead = null;
  eventsDuringRead = null;
  setLoading(false);
  if (searchRefreshTimer !== null) {
    clearTimeout(searchRefreshTimer);
    searchRefreshTimer = null;
  }
  searchStale = false;
}

export {
  UNDO_WINDOW_MS,
  NOTICE_SUBJECT_MAX,
  FRAME_FALLBACK_MS,
  MAX_LIVE_ROWS,
  MAX_QUEUED_EVENTS,
  MAX_TAG_FILTERS,
  LIST_READ_ATTEMPTS,
  PAGE_SIZE,
  SEARCH_REFRESH_WINDOW_MS,
  SOCKET_OPEN_DEADLINE_MS,
  flushPendingDelete,
  liveSummary,
  visibleMessages,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  selectMessage,
  starMessage,
  clearInbox,
  clearInboxPrompt,
  moveSelection,
  deleteWithUndo,
  undoDelete,
  undoableId,
  type SelectionTarget,
  loading,
  loadingMore,
  hasMore,
  heldArrivals,
  heldRefresh,
  setLiveHeld,
  listSize,
  loadMore,
  search,
  setSearch,
  filters,
  hasActiveFilters,
  clearFilters,
  clearTagFilters,
  toggleFilter,
  toggleTagFilter,
  tagFiltersFull,
  allTags,
  fetchMessages,
  connectWebSocket,
  disconnectWebSocket,
};
