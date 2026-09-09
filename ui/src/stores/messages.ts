import { createSignal, createMemo } from "solid-js";
import type { MessageSummary, FilterState, WsEvent } from "../lib/types";
import * as api from "../lib/api";
import * as schema from "../lib/schema";

const PAGE_SIZE = 100;

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

const total = createMemo(() => Math.max(0, storedTotal() - hiddenIds().length));

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
      .catch(() => console.error(`Failed to mark message ${msg.id} as read`));
  }
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
 * back for a round trip on the way out. Once it has settled the hiding has
 * nothing left to do either way: on success the `message:delete` event has
 * taken the message out of the list, and on failure the server still holds it,
 * so keeping it hidden would claim a deletion that never happened.
 */
function commitDelete(keepalive = false): void {
  const id = undoableId();
  clearUndoTimer();
  setUndoableId(null);
  if (id === null) return;

  api
    .deleteMessage(id, { keepalive })
    .catch(() => console.error(`Failed to delete message ${id}`))
    .finally(() => unhide(id));
}

/**
 * Commits the pending deletion in a way that outlives the page.
 *
 * A plain write is cancelled when the document goes away, so a message
 * deleted seconds before a tab closes would be back on the next visit —
 * `keepalive` is what makes the deletion mean what it said.
 */
function flushPendingDelete(): void {
  commitDelete(true);
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

/** Forgets every pending deletion, for when the whole store is gone anyway. */
function dropPendingDeletes(): void {
  clearUndoTimer();
  setUndoableId(null);
  setHiddenIds([]);
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
  setLoading(true);
  try {
    const q = search() || undefined;
    const res = await api.listMessages(PAGE_SIZE, 0, q);
    setMessages(res.messages);
    setStoredTotal(res.total);
  } finally {
    setLoading(false);
  }
}

async function loadMore() {
  if (loading() || loadingMore() || !hasMore()) return;
  setLoadingMore(true);
  try {
    const q = search() || undefined;
    const res = await api.listMessages(PAGE_SIZE, messages().length, q);
    setMessages((prev) => {
      const seen = new Set(prev.map((m) => m.id));
      return [...prev, ...res.messages.filter((m) => !seen.has(m.id))];
    });
    setStoredTotal(res.total);
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
        console.error("Failed to resync inbox after reconnect"),
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
        }
        break;
      case "message:delete":
        setMessages((prev) => prev.filter((m) => m.id !== event.data.id));
        setStoredTotal((t) => Math.max(0, t - 1));
        if (selectedId() === event.data.id) setSelectedId(null);
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
        dropPendingDeletes();
        setMessages([]);
        setStoredTotal(0);
        setSelectedId(null);
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
  flushPendingDelete,
  messages,
  visibleMessages,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  selectMessage,
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
