import { batch, createSignal, createMemo } from "solid-js";
import type { ConfirmDialogOptions } from "../components/ConfirmDialog";
import type { MessageSummary, FilterState, WsEvent } from "../lib/types";
import * as api from "../lib/api";
import * as schema from "../lib/schema";
import { notify } from "./notices";

const PAGE_SIZE = 100;

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

const [messages, setMessages] = createSignal<MessageSummary[]>([]);
const [storedTotal, setStoredTotal] = createSignal(0);
const [selectedId, setSelectedId] = createSignal<string | null>(null);
const [loading, setLoading] = createSignal(false);
const [loadingMore, setLoadingMore] = createSignal(false);
const [search, setSearch] = createSignal("");

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

function hasActiveFilters(): boolean {
  const f = filters();
  return f.starred || f.unread || f.attachments || f.tags.length > 0;
}

function clearFilters() {
  setFilters({ ...defaultFilters });
}

function clearTagFilters() {
  setFilters((f) => ({ ...f, tags: [] }));
}

function toggleFilter(key: "starred" | "unread" | "attachments") {
  setFilters((f) => ({ ...f, [key]: !f[key] }));
}

function toggleTagFilter(tag: string) {
  setFilters((f) => ({
    ...f,
    tags: f.tags.includes(tag)
      ? f.tags.filter((t) => t !== tag)
      : [...f.tags, tag],
  }));
}

const visibleMessages = createMemo(() => {
  const hidden = hiddenIds();
  if (hidden.length === 0) return messages();
  return messages().filter((m) => !hidden.includes(m.id));
});

const total = createMemo(() => {
  const hidden = new Set(hiddenIds());
  const hiddenCount = messages().filter((message) =>
    hidden.has(message.id),
  ).length;
  return Math.max(0, storedTotal() - hiddenCount);
});

/**
 * The inbox list: loaded messages narrowed by the active filters.
 *
 * The unread filter alone makes an exception for the selected message.
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
  return visibleMessages().filter((m) => {
    if (f.starred && !m.is_starred) return false;
    if (f.unread && m.is_read && m.id !== selected) return false;
    if (f.attachments && !m.has_attachments) return false;
    if (f.tags.length > 0 && !f.tags.some((t) => m.tags.includes(t)))
      return false;
    return true;
  });
});

const allTags = createMemo(() => {
  const counts = new Map<string, number>();
  for (const m of visibleMessages()) {
    for (const t of m.tags) {
      counts.set(t, (counts.get(t) || 0) + 1);
    }
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([tag]) => tag);
});

const hasMore = createMemo(() => messages().length < storedTotal());

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
  const subject = messages()
    .find((m) => m.id === id)
    ?.subject?.trim();
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
  setHiddenIds((ids) => ids.filter((hidden) => hidden !== id));
}

/** Identifies the latest server count applied to the list. */
let snapshot = 0;
/** Invalidates reads started before a confirmed deletion or an inbox clear. */
let deletionRevision = 0;
/** Distinguishes deletions made before the entire inbox was cleared. */
let clearRevision = 0;
let latestFetch = 0;
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
  setMessages((prev) => prev.filter((m) => m.id !== id));
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
  const query = search();
  try {
    const response = await api.listMessages(1, 0, query || undefined);
    if (
      request !== latestCountRead ||
      currentSnapshot !== snapshot ||
      currentRevision !== deletionRevision ||
      query !== search()
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
      query === search()
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

async function fetchMessages() {
  const request = ++latestFetch;
  const query = search();
  setLoading(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (request !== latestFetch || query !== search()) return;
      const revision = deletionRevision;
      const res = await api.listMessages(PAGE_SIZE, 0, query || undefined);
      if (request !== latestFetch || query !== search()) return;
      const raced = revision !== deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      batch(() => {
        snapshot += 1;
        countNeedsRefresh = raced;
        setMessages(res.messages);
        setStoredTotal(res.total);
      });
      if (raced) void refreshTotal();
      return;
    }
  } finally {
    if (request === latestFetch) setLoading(false);
  }
}

async function loadMore() {
  if (loading() || loadingMore() || !hasMore()) return;
  const query = search();
  const startedOn = latestFetch;
  setLoadingMore(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (query !== search() || startedOn !== latestFetch) return;
      const revision = deletionRevision;
      const res = await api.listMessages(
        PAGE_SIZE,
        messages().length,
        query || undefined,
      );
      if (query !== search() || startedOn !== latestFetch) return;
      const raced = revision !== deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      batch(() => {
        snapshot += 1;
        countNeedsRefresh = raced;
        setMessages((prev) => {
          const seen = new Set(prev.map((m) => m.id));
          return [...prev, ...res.messages.filter((m) => !seen.has(m.id))];
        });
        setStoredTotal(res.total);
      });
      if (raced) void refreshTotal();
      return;
    }
  } finally {
    setLoadingMore(false);
  }
}

const RECONNECT_BASE_DELAY = 2000;
const MAX_RECONNECT_DELAY = 30000;
let reconnectDelay = RECONNECT_BASE_DELAY;
let currentWs: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let hasConnected = false;

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

function connectWebSocket() {
  disconnectWebSocket();

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/api/v1/ws`);
  currentWs = ws;

  ws.onopen = () => {
    reconnectDelay = RECONNECT_BASE_DELAY;
    if (hasConnected) {
      fetchMessages().catch(() =>
        notify("Reconnected, but could not reload the inbox."),
      );
    }
    hasConnected = true;
  };

  ws.onmessage = (e) => {
    const event = readEvent(e.data);
    if (event === undefined) {
      console.error("Discarded a WebSocket frame the UI could not read");
      return;
    }

    switch (event.type) {
      case "message:new":
        if (search()) {
          fetchMessages();
        } else {
          setMessages((prev) => [event.data, ...prev]);
          setStoredTotal((t) => t + 1);
          if (countNeedsRefresh) void refreshTotal();
        }
        break;
      case "message:delete":
        cancelUndo(event.data.id);
        reconcileDeletion(event.data.id);
        break;
      case "message:read":
        setMessages((prev) =>
          prev.map((m) =>
            m.id === event.data.id ? { ...m, is_read: event.data.is_read } : m,
          ),
        );
        break;
      case "message:starred":
        setMessages((prev) =>
          prev.map((m) =>
            m.id === event.data.id
              ? { ...m, is_starred: event.data.is_starred }
              : m,
          ),
        );
        break;
      case "message:tags":
        setMessages((prev) =>
          prev.map((m) =>
            m.id === event.data.id ? { ...m, tags: event.data.tags } : m,
          ),
        );
        break;
      case "messages:clear":
        batch(() => {
          dropPendingDeletes();
          setMessages([]);
          setStoredTotal(0);
          setSelectedId(null);
        });
        break;
    }
  };

  ws.onclose = () => {
    currentWs = null;
    const jitter = reconnectDelay * (0.5 + Math.random() * 0.5);
    reconnectTimer = setTimeout(connectWebSocket, jitter);
    reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY);
  };

  return ws;
}

function disconnectWebSocket() {
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

export {
  UNDO_WINDOW_MS,
  NOTICE_SUBJECT_MAX,
  LIST_READ_ATTEMPTS,
  flushPendingDelete,
  messages,
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
  loadMore,
  search,
  setSearch,
  filters,
  hasActiveFilters,
  clearFilters,
  clearTagFilters,
  toggleFilter,
  toggleTagFilter,
  allTags,
  fetchMessages,
  connectWebSocket,
  disconnectWebSocket,
};
