import { batch } from "solid-js";
import { notify } from "../notices";
import * as api from "../../lib/api";
import { UNDO_WINDOW_MS } from "./constants";
import {
  findMessage,
  heldRows,
  indexOfRow,
  letGoWhileHidden,
  messages,
  nextCursor,
  openRow,
  removeRow,
  search,
  selectedId,
  setHeldArrivals,
  setHiddenIds,
  setNextCursor,
  setOpenRow,
  setSelectedId,
  setStoredTotal,
  setUndoableId,
  undoableId,
} from "./rows";
import {
  currentView,
  filteredMessages,
  filters,
  hasActiveFilters,
  isCurrentView,
  matchesFilters,
  viewQuery,
} from "./view";
import { reads } from "./readState";
import { selectMessage } from "./selection";
import { quoted } from "./writes";

let undoTimer: ReturnType<typeof setTimeout> | null = null;

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

/** Distinguishes deletions made before the entire inbox was cleared. */
let clearRevision = 0;
let latestCountRead = 0;
/** A local deletion and the count against which it was issued. */
type IssuedDelete = {
  snapshot: number;
  clearRevision: number;
  confirmed: boolean;
};

/** Local deletions awaiting their HTTP response or WebSocket echo. */
const issuedDeletes = new Map<string, IssuedDelete>();

/** Removes a confirmed deletion without assuming the current total includes it. */
export function forgetMessage(id: string): void {
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
export async function refreshTotal(): Promise<void> {
  const request = ++latestCountRead;
  const currentSnapshot = reads.snapshot;
  const currentRevision = reads.deletionRevision;
  const view = currentView();
  try {
    const response = await api.listMessages({ limit: 1, ...viewQuery(view) });
    if (
      request !== latestCountRead ||
      currentSnapshot !== reads.snapshot ||
      currentRevision !== reads.deletionRevision ||
      !isCurrentView(view)
    )
      return;
    reads.snapshot += 1;
    reads.countNeedsRefresh = false;
    setStoredTotal(response.total);
  } catch {
    if (
      request === latestCountRead &&
      currentSnapshot === reads.snapshot &&
      currentRevision === reads.deletionRevision &&
      isCurrentView(view)
    ) {
      notify(
        "The message was deleted, but the inbox count could not be refreshed.",
      );
    }
  }
}

/**
 * Whether the total counts `id`, or `undefined` when only the server knows.
 *
 * A row the list holds counts while it matches the filters: one a flag change
 * took out of them was already subtracted, though it stays on screen. A row
 * the list never loaded counts in an unnarrowed view, which counts everything,
 * but a search or a filter cannot tell whether it was among its matches.
 */
function countedInTotal(id: string): boolean | undefined {
  const row = findMessage(id) ?? heldRows.get(id);
  if (row !== undefined) return matchesFilters(row, filters());
  return search() !== "" || hasActiveFilters() ? undefined : true;
}

/**
 * Applies either confirmation once, removing the row regardless of the snapshot.
 *
 * The total drops only by a message it counts, per {@link countedInTotal}, and
 * is read from the server when that cannot be told locally.
 */
export function reconcileDeletion(id: string): void {
  reconcileDeletions([id]);
}

/**
 * Applies {@link reconcileDeletion} to every id in `ids` as one change, reading
 * the total from the server at most once for all of them.
 */
export function reconcileDeletions(ids: readonly string[]): void {
  let countUnknown = false;
  batch(() => {
    for (const id of ids) countUnknown = settleDeletion(id) || countUnknown;
  });
  if (countUnknown) void refreshTotal();
}

/**
 * Applies one confirmed deletion, resolving whether the total now needs a read
 * from the server.
 */
function settleDeletion(id: string): boolean {
  const issued = issuedDeletes.get(id);
  if (issued?.confirmed === true) {
    issuedDeletes.delete(id);
    return false;
  }

  const cleared =
    issued !== undefined && issued.clearRevision !== clearRevision;
  const counted = countedInTotal(id);
  const countIsCurrent =
    counted !== undefined &&
    !reads.countNeedsRefresh &&
    (issued === undefined || issued.snapshot === reads.snapshot);
  if (issued) issued.confirmed = true;
  if (!cleared) {
    reads.deletionRevision += 1;
    if (!countIsCurrent) reads.countNeedsRefresh = true;
  }
  batch(() => {
    forgetMessage(id);
    if (!cleared && countIsCurrent && counted) {
      setStoredTotal((current) => Math.max(0, current - 1));
    }
  });
  return !cleared && !countIsCurrent;
}

/**
 * Drops the undo offer for a message the server has already deleted.
 *
 * Another client, or the retention sweep, can delete a message inside its undo
 * window. Bringing it back is no longer possible, and the DELETE the timer
 * would fire has nothing left to delete.
 */
export function cancelUndo(id: string): void {
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
export function deleteWithUndo(id: string): void {
  commitDelete();
  setHiddenIds((ids) => (ids.includes(id) ? ids : [...ids, id]));
  setUndoableId(id);
  undoTimer = setTimeout(() => commitDelete(), UNDO_WINDOW_MS);
}

/**
 * Deletes the selected message with undo, and selects the one after it, or
 * the one before when it was the last.
 */
export function deleteSelected(): void {
  const id = selectedId();
  if (!id) return;
  const msgs = filteredMessages();
  const index = msgs.findIndex((m) => m.id === id);
  const next =
    index === -1 ? null : (msgs[index + 1] ?? msgs[index - 1] ?? null);
  if (next) {
    selectMessage(next);
  } else {
    setSelectedId(null);
  }
  deleteWithUndo(id);
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
  issuedDeletes.set(id, {
    snapshot: reads.snapshot,
    clearRevision,
    confirmed: false,
  });
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
export function flushPendingDelete(): void {
  commitDelete();
}

/** Brings the last deleted message back, while its window is still open. */
export function undoDelete(): void {
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
export function dropPendingDeletes(): void {
  clearUndoTimer();
  setUndoableId(null);
  setHiddenIds([]);
  reads.snapshot += 1;
  reads.deletionRevision += 1;
  clearRevision += 1;
  reads.countNeedsRefresh = false;
}
