import { batch } from "solid-js";
import type { ListResponse, MessageSummary } from "../../lib/types";
import { notify } from "../notices";
import * as api from "../../lib/api";
import {
  LIST_READ_ATTEMPTS,
  LIST_REFRESH_WINDOW_MS,
  MAX_LIVE_ROWS,
  PAGE_SIZE,
  UNKNOWN_CURSOR_CODE,
} from "./constants";
import {
  appendRows,
  dropHeldArrivals,
  holdArrivals,
  indexOfRow,
  liveHeld,
  loading,
  loadingMore,
  messages,
  nextCursor,
  positions,
  replaceRows,
  search,
  setHeldRefresh,
  setListedWhole,
  setLoading,
  setLoadingMore,
  setRows,
  setStoredTotal,
} from "./rows";
import {
  currentView,
  cursorView,
  hasActiveFilters,
  hasMore,
  isCurrentView,
  setPageCursor,
  viewQuery,
  type View,
} from "./view";
import { reads } from "./readState";
import { forgetMessage, refreshTotal } from "./deletions";
import { applyEvents, replayEvents, revealArrivals } from "./liveEvents";

/** The view {@link scheduleListRefresh} last found stale. */
let staleView: View = currentView();
/**
 * Reads the first page again, replacing the list.
 *
 * Live events that arrive while the read is in flight are replayed over the
 * page it returns, since the server may have answered from before them.
 * Starting a read aborts the one before it, which then resolves quietly: its
 * answer would be dropped anyway, and only the current read's failure is worth
 * reporting.
 */
export async function fetchMessages(): Promise<void> {
  await readFirstPage();
}

/**
 * A list read that has landed: the count it read, the ids of the rows it
 * returned, how to show it, and whether the count and the rows come from one
 * response.
 */
type LandedRead = {
  total: number;
  ids: ReadonlySet<string>;
  show: () => void;
  onePage: boolean;
};

async function readTopPage(
  view: View,
  signal: AbortSignal,
): Promise<LandedRead> {
  const res = await api.listMessages(
    { limit: PAGE_SIZE, ...viewQuery(view) },
    signal,
  );
  return {
    total: res.total,
    ids: new Set(res.messages.map((m) => m.id)),
    show: () => {
      replaceRows(res.messages);
      dropHeldArrivals();
      setPageCursor(res.next_cursor, view);
    },
    onePage: true,
  };
}

/**
 * Reads the list down through the loaded rows, reconciling them in place.
 *
 * This is the read while live mail is held: the reader is scrolled into these
 * rows, so replacing them with the first page would move the list under them.
 * Rows above the first loaded one are arrivals and wait behind the pill. From
 * the first loaded row the read meets to the last, the server's rows replace
 * the loaded ones: a loaded row the read meets takes the server's copy, one it
 * passes without meeting is gone from the view, and a row between them that
 * was never loaded, such as one starred elsewhere under the starred filter,
 * joins the list where it falls. The read stops at the loaded tail, at the end
 * of the list, after {@link PAGE_SIZE} rows past the last loaded one it met,
 * which is where a deleted tail leaves it, or once more has arrived than the
 * pill keeps, which only a fresh read on the way back to the top can show.
 * Only the first two pass every loaded row: after the others, a loaded row
 * below the last one met may lie further down or be gone, so it stays until an
 * older-page read finds out.
 *
 * The total is the first page's: only that page can hold an arrival, so one
 * that commits while older pages are read is in neither the rows nor the count,
 * and replaying its event counts it once. A flag change made between two pages
 * may already show in a later page's rows but not in that count, so the read
 * reports when it spanned more than one page.
 */
async function readLoadedWindow(
  view: View,
  signal: AbortSignal,
): Promise<LandedRead> {
  const tail = messages().at(-1)?.id;
  const read: MessageSummary[] = [];
  let total: number | undefined;
  let pages = 0;
  let metAny = false;
  let sinceLoaded = 0;
  let before: string | undefined;
  for (;;) {
    const res = await api.listMessages(
      {
        limit: PAGE_SIZE,
        ...viewQuery(view),
        ...(before === undefined ? {} : { before }),
      },
      signal,
    );
    total ??= res.total;
    pages += 1;
    let metTail = false;
    for (const m of res.messages) {
      read.push(m);
      metTail ||= m.id === tail;
      if (positions.has(m.id)) {
        metAny = true;
        sinceLoaded = 0;
      } else if (metAny) {
        sinceLoaded += 1;
      }
    }
    const ended = res.next_cursor === null;
    const passedAll = ended || metTail;
    const outranLoaded = metAny
      ? sinceLoaded >= PAGE_SIZE
      : read.length > MAX_LIVE_ROWS;
    if (passedAll || outranLoaded) {
      return {
        total,
        ids: new Set(read.map((m) => m.id)),
        show: () => reconcileWindow(read, passedAll, ended, view),
        onePage: pages === 1,
      };
    }
    before = res.next_cursor ?? undefined;
  }
}

/**
 * Applies what {@link readLoadedWindow} read, `passedAll` saying whether it got
 * past every loaded row and `ended` whether it reached the end of the list.
 */
function reconcileWindow(
  read: readonly MessageSummary[],
  passedAll: boolean,
  ended: boolean,
  view: View,
): void {
  const loaded = messages();
  const firstLoaded = read.findIndex((m) => positions.has(m.id));
  const lastLoaded = read.findLastIndex((m) => positions.has(m.id));
  const arrivals = firstLoaded < 0 ? read : read.slice(0, firstLoaded);
  const met = firstLoaded < 0 ? [] : read.slice(firstLoaded, lastLoaded + 1);
  const lastMet = lastLoaded < 0 ? -1 : indexOfRow(read[lastLoaded].id);
  const unmet = passedAll ? [] : loaded.slice(lastMet + 1);
  const kept = [...met, ...unmet];
  const last = kept.at(-1) ?? arrivals.at(-1);
  const lastRead =
    last === undefined ? -1 : read.findIndex((m) => m.id === last.id);
  const more = !ended || lastRead < read.length - 1;
  setRows(kept);
  dropHeldArrivals();
  holdArrivals(arrivals.toReversed());
  setPageCursor(more && last !== undefined ? last.id : null, view);
  if (!liveHeld()) revealArrivals();
}

/** Does the work of {@link fetchMessages}, resolving whether its page landed. */
export async function readFirstPage(): Promise<boolean> {
  const request = ++reads.latestFetch;
  const view = currentView();
  const inPlace =
    liveHeld() && isCurrentView(cursorView) && messages().length > 0;
  const readList = inPlace ? readLoadedWindow : readTopPage;
  reads.currentListRead?.abort();
  reads.currentPageRead?.abort();
  const controller = new AbortController();
  reads.currentListRead = controller;
  setLoading(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (request !== reads.latestFetch || !isCurrentView(view)) return false;
      const revision = reads.deletionRevision;
      reads.eventsDuringRead = [];
      const landed = await readList(view, controller.signal);
      if (request !== reads.latestFetch || !isCurrentView(view)) return false;
      const raced = revision !== reads.deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      const countUnsettled =
        raced || (!landed.onePage && (reads.eventsDuringRead?.length ?? 0) > 0);
      batch(() => {
        reads.snapshot += 1;
        reads.countNeedsRefresh = countUnsettled;
        landed.show();
        setListedWhole(search() === "" && !hasActiveFilters());
        setStoredTotal(landed.total);
        reads.listStale = false;
        replayEvents(reads.eventsDuringRead ?? [], landed.ids);
      });
      if (countUnsettled) void refreshTotal();
      return true;
    }
    return false;
  } catch (error) {
    if (request === reads.latestFetch) throw error;
    return false;
  } finally {
    if (request === reads.latestFetch) {
      reads.currentListRead = null;
      reads.eventsDuringRead = null;
      setLoading(false);
      if (reads.listStale) scheduleListRefresh();
    }
  }
}

/**
 * Marks the list stale and reads it again once the window closes.
 *
 * Some live events cannot be placed here: a search cannot tell whether an
 * arrival matches, and a filter cannot show a message it never loaded that a
 * flag change has made match. They are folded into one trailing read per
 * {@link LIST_REFRESH_WINDOW_MS}. A read still in flight when the window
 * closes is left alone; the stale flag outlives it and schedules the next.
 * A refresh is for the view it was scheduled in, and another view drops it.
 */
export function scheduleListRefresh(): void {
  reads.listStale = true;
  staleView = currentView();
  if (reads.listRefreshTimer !== null) return;
  reads.listRefreshTimer = setTimeout(() => {
    reads.listRefreshTimer = null;
    if (!reads.listStale || loading() || !isCurrentView(staleView)) return;
    if (liveHeld()) {
      setHeldRefresh(true);
      return;
    }
    fetchMessages().catch(() => notify("Could not refresh the list."));
  }, LIST_REFRESH_WINDOW_MS);
}

const OLDER_PAGE_FAILED = "Could not load older messages.";

/**
 * Reads the page older than the last loaded row, and appends it.
 *
 * A cursor the server no longer knows belongs to a message deleted before its
 * event got here, so the row goes and the read starts again from the one
 * before it. A page read from a cursor that has since moved, because its row
 * was deleted or let go, would not join the rows above it, so it is read again
 * from where the list now ends, and reported once
 * {@link LIST_READ_ATTEMPTS} reads have all missed. Any other failure is
 * reported rather than thrown, since the list's scroll position is what calls
 * this.
 *
 * A flag change that lands while the page is read finds none of its rows yet,
 * so it is applied again over the page once it is appended, and the count is
 * read again: the page may have been served before or after it, and an arrival
 * made meanwhile may be missing from the page's total.
 */
export async function loadMore(): Promise<void> {
  if (loading() || loadingMore() || !hasMore()) return;
  if (!isCurrentView(cursorView)) return;
  const view = currentView();
  const startedOn = reads.latestFetch;
  const controller = new AbortController();
  reads.currentPageRead = controller;
  setLoadingMore(true);
  try {
    for (let attempt = 0; attempt < LIST_READ_ATTEMPTS; attempt += 1) {
      if (!isCurrentView(view) || startedOn !== reads.latestFetch) return;
      const before = nextCursor();
      if (before === null) return;
      const revision = reads.deletionRevision;
      reads.eventsDuringPageRead = [];
      let res: ListResponse;
      try {
        res = await api.listMessages(
          { limit: PAGE_SIZE, ...viewQuery(view), before },
          controller.signal,
        );
      } catch (error) {
        if (!isUnknownCursor(error)) throw error;
        if (!isCurrentView(view) || startedOn !== reads.latestFetch) return;
        forgetMessage(before);
        continue;
      }
      if (!isCurrentView(view) || startedOn !== reads.latestFetch) return;
      if (nextCursor() !== before) continue;
      const raced = revision !== reads.deletionRevision;
      const last = attempt === LIST_READ_ATTEMPTS - 1;
      if (raced && !last) continue;
      const missed = reads.eventsDuringPageRead ?? [];
      const countUnsettled = raced || missed.length > 0;
      batch(() => {
        reads.snapshot += 1;
        reads.countNeedsRefresh = countUnsettled;
        appendRows(res.messages.filter((m) => !positions.has(m.id)));
        setPageCursor(res.next_cursor, view);
        setStoredTotal(res.total);
        applyEvents(missed.filter((event) => event.type !== "message:new"));
      });
      if (countUnsettled) void refreshTotal();
      return;
    }
    notify(OLDER_PAGE_FAILED);
  } catch {
    if (isCurrentView(view) && startedOn === reads.latestFetch) {
      notify(OLDER_PAGE_FAILED);
    }
  } finally {
    if (reads.currentPageRead === controller) {
      reads.currentPageRead = null;
      reads.eventsDuringPageRead = null;
    }
    setLoadingMore(false);
  }
}

function isUnknownCursor(error: unknown): boolean {
  return error instanceof api.ApiError && error.code === UNKNOWN_CURSOR_CODE;
}
