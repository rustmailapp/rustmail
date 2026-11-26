import { createSignal } from "solid-js";
import { notify } from "../notices";
import { setLoading } from "./rows";
import { reads } from "./readState";
import { readFirstPage } from "./listReads";
import { dropQueuedEvents, enqueueEvent, readEvent } from "./liveEvents";

const RECONNECT_BASE_DELAY = 2000;
const MAX_RECONNECT_DELAY = 30000;
let reconnectDelay = RECONNECT_BASE_DELAY;
let currentWs: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
/** Whether the live connection is open, so the UI can say it is reconnecting. */
export const [socketLive, setSocketLive] = createSignal(false);

/**
 * How long the first socket may take to open before the list is read over
 * HTTP instead.
 *
 * A proxy that drops the upgrade leaves the socket pending or failing forever,
 * and the inbox must still load without live updates.
 */
export const SOCKET_OPEN_DEADLINE_MS = 3000;
let httpFallbackTimer: ReturnType<typeof setTimeout> | null = null;
let inboxLoaded = false;
let onFirstSync: (() => void) | null = null;

/**
 * Opens the live connection, and reads the list each time it opens.
 *
 * The read comes after the open, the first time included: anything stored
 * before the socket subscribed is only in the list the server returns, and
 * anything after it is on the socket. A socket that fails before it first
 * opens, or misses {@link SOCKET_OPEN_DEADLINE_MS}, gets the list read over
 * HTTP once instead. `onSynced` runs once, after the first read that lands.
 */
export function connectWebSocket(onSynced?: () => void): WebSocket {
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
    setSocketLive(true);
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
    setSocketLive(false);
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
  setSocketLive(false);
}

export function disconnectWebSocket(): void {
  stopHttpFallback();
  closeSocket();
  dropQueuedEvents();
  reads.latestFetch += 1;
  reads.currentListRead?.abort();
  reads.currentListRead = null;
  reads.currentPageRead?.abort();
  reads.currentPageRead = null;
  reads.eventsDuringRead = null;
  reads.eventsDuringPageRead = null;
  setLoading(false);
  if (reads.listRefreshTimer !== null) {
    clearTimeout(reads.listRefreshTimer);
    reads.listRefreshTimer = null;
  }
  reads.listStale = false;
}
