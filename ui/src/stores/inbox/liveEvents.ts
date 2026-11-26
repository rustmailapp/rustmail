import { batch } from "solid-js";
import type {
  FilterState,
  MessageSummary,
  ReleaseRecord,
  WsEvent,
} from "../../lib/types";
import { newerRelease, recordedRelease } from "../../lib/release";
import { notify } from "../notices";
import * as schema from "../../lib/schema";
import { FRAME_FALLBACK_MS, MAX_QUEUED_EVENTS } from "./constants";
import {
  dropHeldArrivals,
  findMessage,
  heldOverflowed,
  heldRefresh,
  heldRows,
  holdArrivals,
  liveHeld,
  openRow,
  positions,
  prependRows,
  replaceRow,
  replaceRows,
  search,
  setHeldArrivals,
  setLiveHeldSignal,
  setNextCursor,
  setOpenRow,
  setSelectedId,
  setStoredTotal,
  trimRows,
} from "./rows";
import { filters, matchesFilters } from "./view";
import { reads } from "./readState";
import {
  cancelUndo,
  dropPendingDeletes,
  reconcileDeletion,
  reconcileDeletions,
  refreshTotal,
} from "./deletions";
import { fetchMessages, scheduleListRefresh } from "./listReads";

/**
 * Holds live arrivals back while `held`, and shows what waited once it is not.
 *
 * The list tells the store where the reader is; the store decides nothing
 * from scroll positions itself.
 */
export function setLiveHeld(held: boolean): void {
  if (held === liveHeld()) return;
  setLiveHeldSignal(held);
  if (!held) revealArrivals();
}

/**
 * Puts the waiting arrivals on top, or reads the first page again when what
 * waited cannot be shown from memory: a stale search, or more than was kept.
 */
export function revealArrivals(): void {
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

/**
 * The event a frame carries, or `undefined` if it carries nothing usable.
 *
 * A frame reaches the store as text nobody has checked, and the list is built
 * straight out of it — `message:new` is appended as a message. The frame never
 * reaches the log either: it holds a sender, its recipients and a subject.
 */
export function readEvent(frame: unknown): WsEvent | undefined {
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
    scheduleListRefresh();
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
  if (reads.countNeedsRefresh) void refreshTotal();
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
function patchMessage(
  id: string,
  patch:
    | Partial<MessageSummary>
    | ((current: MessageSummary) => Partial<MessageSummary>),
): void {
  const apply = (current: MessageSummary): MessageSummary => ({
    ...current,
    ...(typeof patch === "function" ? patch(current) : patch),
  });
  const open = openRow();
  if (open?.id === id) setOpenRow(apply(open));
  const waiting = heldRows.get(id);
  if (waiting !== undefined) {
    const patched = apply(waiting);
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
  if (current === undefined) {
    if (typeof patch !== "function" && mayStartMatching(patch, filters())) {
      scheduleListRefresh();
    }
    return;
  }
  const patched = apply(current);
  const f = filters();
  const was = matchesFilters(current, f);
  const is = matchesFilters(patched, f);
  batch(() => {
    replaceRow(patched);
    if (was !== is) setStoredTotal((t) => Math.max(0, t + (is ? 1 : -1)));
  });
}

/**
 * Whether `patch` could make a message the list does not hold match `f`.
 *
 * Only a change to a field a filter narrows on can, and only towards the
 * filter: a message marked read cannot enter the unread filter. That keeps the
 * echo of a change that took a row out of the filters, after the list let the
 * row go, from reading the list again.
 */
function mayStartMatching(
  patch: Partial<MessageSummary>,
  f: FilterState,
): boolean {
  const narrowed =
    (f.starred && patch.is_starred !== undefined) ||
    (f.unread && patch.is_read !== undefined) ||
    (f.tags.length > 0 && patch.tags !== undefined);
  if (!narrowed) return false;
  if (f.starred && patch.is_starred === false) return false;
  if (f.unread && patch.is_read === true) return false;
  const tags = patch.tags;
  return (
    tags === undefined ||
    f.tags.length === 0 ||
    f.tags.some((t) => tags.includes(t))
  );
}

/** The events a list read may have been served without. */
export type ReplayableEvent = Extract<
  WsEvent,
  {
    type:
      | "message:new"
      | "message:read"
      | "message:starred"
      | "message:tags"
      | "message:released";
  }
>;

function isReplayable(event: WsEvent): event is ReplayableEvent {
  return (
    event.type === "message:new" ||
    event.type === "message:read" ||
    event.type === "message:starred" ||
    event.type === "message:tags" ||
    event.type === "message:released"
  );
}

/**
 * Applies `events` in order, each run of arrivals as one prepend.
 *
 * Applying a replayable event twice is harmless, which is what lets
 * {@link fetchMessages} replay the ones that arrived while its read was in
 * flight.
 */
export function applyEvents(events: readonly WsEvent[]): void {
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

/**
 * Applies the events that arrived while a list read was in flight over the
 * list it returned.
 *
 * Arrivals are announced in the order they commit, so every arrival up to the
 * last one among the rows the read returned, `read`, was already in the read,
 * and counted in its total even when it sits past the rows it returned; only
 * the arrivals after it are new.
 */
export function replayEvents(
  events: readonly ReplayableEvent[],
  read: ReadonlySet<string>,
): void {
  const lastInRead = events.findLastIndex(
    (event) => event.type === "message:new" && read.has(event.data.id),
  );
  applyEvents(
    events.filter((event, i) => i > lastInRead || event.type !== "message:new"),
  );
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
    case "message:released": {
      const { id, ...incoming } = event.data;
      patchMessage(id, (current) =>
        newerRelease(recordedRelease(current), incoming),
      );
      break;
    }
    case "message:delete":
      cancelUndo(event.data.id);
      reconcileDeletion(event.data.id);
      break;
    case "messages:delete":
      for (const id of event.data.ids) cancelUndo(id);
      reconcileDeletions(event.data.ids);
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

/**
 * Records a release this tab made, as the server answered it.
 *
 * The server broadcasts the same record to every tab, this one included, but
 * the answer is here first and a warning against a second send must not wait
 * on the socket. Applied as the event it stands for, so the broadcast that
 * follows changes nothing and a list read in flight is replayed with it.
 */
export function recordRelease(id: string, record: ReleaseRecord): void {
  const event: ReplayableEvent = {
    type: "message:released",
    data: { id, ...record },
  };
  reads.eventsDuringRead?.push(event);
  reads.eventsDuringPageRead?.push(event);
  applyEvent(event);
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
export function enqueueEvent(event: WsEvent): void {
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
  if (reads.eventsDuringRead !== null || reads.eventsDuringPageRead !== null) {
    const replayable = events.filter(isReplayable);
    reads.eventsDuringRead?.push(...replayable);
    reads.eventsDuringPageRead?.push(...replayable);
  }
  batch(() => applyEvents(events));
}

export function dropQueuedEvents(): void {
  cancelFlush();
  queuedEvents = [];
  queueOverflowed = false;
}
