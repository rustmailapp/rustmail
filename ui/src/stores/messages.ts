import { createSignal, createMemo } from "solid-js";
import type { MessageSummary, FilterState, WsEvent } from "../lib/types";
import * as api from "../lib/api";

const PAGE_SIZE = 100;

const [messages, setMessages] = createSignal<MessageSummary[]>([]);
const [total, setTotal] = createSignal(0);
const [selectedId, setSelectedId] = createSignal<string | null>(null);
const [loading, setLoading] = createSignal(false);
const [loadingMore, setLoadingMore] = createSignal(false);
const [search, setSearch] = createSignal("");

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

const filteredMessages = createMemo(() => {
  const f = filters();
  if (!f.starred && !f.unread && !f.attachments && f.tags.length === 0) {
    return messages();
  }
  return messages().filter((m) => {
    if (f.starred && !m.is_starred) return false;
    if (f.unread && m.is_read) return false;
    if (f.attachments && !m.has_attachments) return false;
    if (f.tags.length > 0 && !f.tags.some((t) => m.tags.includes(t)))
      return false;
    return true;
  });
});

const allTags = createMemo(() => {
  const counts = new Map<string, number>();
  for (const m of messages()) {
    for (const t of m.tags) {
      counts.set(t, (counts.get(t) || 0) + 1);
    }
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([tag]) => tag);
});

const hasMore = createMemo(() => messages().length < total());

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
    setTotal(res.total);
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
    setTotal(res.total);
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
    let event: WsEvent;
    try {
      event = JSON.parse(e.data);
    } catch {
      console.error("Failed to parse WebSocket message:", e.data);
      return;
    }

    switch (event.type) {
      case "message:new":
        if (search()) {
          fetchMessages();
        } else {
          setMessages((prev) => [event.data, ...prev]);
          setTotal((t) => t + 1);
        }
        break;
      case "message:delete":
        setMessages((prev) => prev.filter((m) => m.id !== event.data.id));
        setTotal((t) => Math.max(0, t - 1));
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
        setMessages([]);
        setTotal(0);
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
  messages,
  filteredMessages,
  total,
  selectedId,
  setSelectedId,
  selectMessage,
  moveSelection,
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
