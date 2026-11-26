/**
 * The inbox store: the message list, its filters and selection, deletions
 * with undo, list reads and live events. Each part lives in `./inbox/`;
 * this module keeps the one import path the UI and the tests use.
 */
export {
  UNDO_WINDOW_MS,
  NOTICE_SUBJECT_MAX,
  FRAME_FALLBACK_MS,
  MAX_LIVE_ROWS,
  MAX_QUEUED_EVENTS,
  MAX_TAG_FILTERS,
  LIST_READ_ATTEMPTS,
  PAGE_SIZE,
  LIST_REFRESH_WINDOW_MS,
} from "./inbox/constants";
export {
  liveSummary,
  selectedId,
  setSelectedId,
  undoableId,
  loading,
  loadingMore,
  heldArrivals,
  heldRefresh,
  search,
  setSearch,
} from "./inbox/rows";
export {
  visibleMessages,
  filteredMessages,
  inboxEmpty,
  total,
  hasMore,
  listSize,
  filters,
  hasActiveFilters,
  tagFiltersFull,
  allTags,
} from "./inbox/view";
export {
  clearFilters,
  clearTagFilters,
  toggleFilter,
  toggleTagFilter,
} from "./inbox/filters";
export {
  selectMessage,
  moveSelection,
  type SelectionTarget,
} from "./inbox/selection";
export {
  starMessage,
  setMessageRead,
  clearInbox,
  clearInboxPrompt,
} from "./inbox/writes";
export {
  flushPendingDelete,
  deleteWithUndo,
  deleteSelected,
  undoDelete,
} from "./inbox/deletions";
export { loadMore, fetchMessages } from "./inbox/listReads";
export { recordRelease, setLiveHeld } from "./inbox/liveEvents";
export {
  SOCKET_OPEN_DEADLINE_MS,
  connectWebSocket,
  disconnectWebSocket,
  socketLive,
} from "./inbox/socket";
