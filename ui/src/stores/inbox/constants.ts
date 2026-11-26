export const PAGE_SIZE = 100;

/**
 * The most rows live mail keeps loaded at the top of the list.
 *
 * Each arrival pushes the oldest loaded row out past this, and scrolling down
 * reads it back from the server, so memory and every per-frame pass over the
 * list stay bounded however long the traffic runs. Only the reader scrolling
 * down loads past it.
 */
export const MAX_LIVE_ROWS = 5 * PAGE_SIZE;

/** The most `tag` filters `GET /messages` accepts; one more is a `400`. */
export const MAX_TAG_FILTERS = 20;

/** The `code` `GET /messages` rejects a `before` it does not know with. */
export const UNKNOWN_CURSOR_CODE = "unknown_cursor";

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
export const LIST_READ_ATTEMPTS = 3;

/** How much of a subject a notice quotes before trimming it. */
export const NOTICE_SUBJECT_MAX = 50;

/**
 * How long a deleted message can be brought back.
 *
 * It is the DELETE that waits out the window, not the row: the message leaves
 * the list on the keystroke, so undo costs nothing but dropping a timer.
 */
export const UNDO_WINDOW_MS = 5000;

/**
 * The most often live events refetch a list they cannot update in place.
 *
 * Every arrival makes a search stale, and at hundreds per second a refetch
 * each would never let one land while flooding the server with full-text
 * queries.
 */
export const LIST_REFRESH_WINDOW_MS = 1000;

/**
 * The most live events held for the next frame.
 *
 * Only a tab that gets no frames, one in the background, comes near this; a
 * backlog that long is cheaper to replace with one list read than to replay.
 */
export const MAX_QUEUED_EVENTS = 10_000;

/**
 * How long a queued event waits for an animation frame before it is applied
 * anyway.
 *
 * A hidden tab gets no frames at all, and its list must still be current when
 * it comes back.
 */
export const FRAME_FALLBACK_MS = 250;
