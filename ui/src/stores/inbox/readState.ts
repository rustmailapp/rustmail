import type { ReplayableEvent } from "./liveEvents";

/** List-read bookkeeping shared by the reads, deletions and live events. */
interface ReadState {
  /** Identifies the latest server count applied to the list. */
  snapshot: number;
  /** Invalidates reads started before a confirmed deletion or an inbox clear. */
  deletionRevision: number;
  latestFetch: number;
  currentListRead: AbortController | null;
  /** The older-page read in flight, abandoned once a first-page read supersedes it. */
  currentPageRead: AbortController | null;
  listStale: boolean;
  listRefreshTimer: ReturnType<typeof setTimeout> | null;
  /** Live events applied while the current list read is in flight. */
  eventsDuringRead: ReplayableEvent[] | null;
  /** Live events applied while the current older-page read is in flight. */
  eventsDuringPageRead: ReplayableEvent[] | null;
  countNeedsRefresh: boolean;
}

export const reads: ReadState = {
  snapshot: 0,
  deletionRevision: 0,
  latestFetch: 0,
  currentListRead: null,
  currentPageRead: null,
  listStale: false,
  listRefreshTimer: null,
  eventsDuringRead: null,
  eventsDuringPageRead: null,
  countNeedsRefresh: false,
};
