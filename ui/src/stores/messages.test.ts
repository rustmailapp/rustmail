import { createEffect, createRoot, on } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../lib/types";
import { dismissNotice, notices } from "./notices";

const {
  deleteAllMessages,
  deleteMessage,
  listMessages,
  markRead,
  markStarred,
} = vi.hoisted(() => ({
  deleteAllMessages: vi.fn(),
  deleteMessage: vi.fn(),
  listMessages: vi.fn(),
  markRead: vi.fn(),
  markStarred: vi.fn(),
}));

vi.mock("../lib/api", async () => ({
  ApiError: (await vi.importActual<typeof import("../lib/api")>("../lib/api"))
    .ApiError,
  deleteAllMessages,
  deleteMessage,
  listMessages,
  markRead,
  markStarred,
}));

const { ApiError } = await import("../lib/api");
const {
  FRAME_FALLBACK_MS,
  LIST_READ_ATTEMPTS,
  MAX_LIVE_ROWS,
  MAX_QUEUED_EVENTS,
  MAX_TAG_FILTERS,
  NOTICE_SUBJECT_MAX,
  PAGE_SIZE,
  SEARCH_REFRESH_WINDOW_MS,
  SOCKET_OPEN_DEADLINE_MS,
  UNDO_WINDOW_MS,
  clearFilters,
  clearInbox,
  clearInboxPrompt,
  connectWebSocket,
  disconnectWebSocket,
  deleteWithUndo,
  flushPendingDelete,
  fetchMessages,
  filteredMessages,
  filters,
  hasMore,
  loading,
  moveSelection,
  loadMore,
  selectMessage,
  selectedId,
  starMessage,
  setSearch,
  setSelectedId,
  toggleFilter,
  toggleTagFilter,
  allTags,
  listSize,
  liveSummary,
  heldArrivals,
  heldRefresh,
  setLiveHeld,
  total,
  undoDelete,
  undoableId,
} = await import("./messages");

function message(
  n: number,
  over: Partial<MessageSummary> = {},
): MessageSummary {
  return {
    id: `id-${n}`,
    sender: `sender-${n}@example.test`,
    recipients: ["inbox@example.test"],
    subject: `Subject ${n}`,
    size: 1024,
    has_attachments: false,
    is_read: false,
    is_starred: false,
    tags: [],
    created_at: "2026-01-01T00:00:00Z",
    ...over,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}

type Page = {
  messages: MessageSummary[];
  total: number;
  limit: number;
  next_cursor: string | null;
};

/** A list response as the server shapes one, older pages behind `cursor`. */
function page(
  msgs: MessageSummary[],
  total = msgs.length,
  cursor: string | null = null,
): Page {
  return { messages: msgs, total, limit: PAGE_SIZE, next_cursor: cursor };
}

async function seed(msgs: MessageSummary[]): Promise<void> {
  listMessages.mockResolvedValue(page(msgs));
  await fetchMessages();
}

function range(count: number): MessageSummary[] {
  return Array.from({ length: count }, (_, i) => message(i));
}

/** Animation frames the store asked for and the test has not run yet. */
const frames = new Map<number, FrameRequestCallback>();
let lastFrame = 0;

function stubFrames(): void {
  frames.clear();
  vi.stubGlobal("requestAnimationFrame", (run: FrameRequestCallback) => {
    lastFrame += 1;
    frames.set(lastFrame, run);
    return lastFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (handle: number) => {
    frames.delete(handle);
  });
}

/**
 * Fakes the clock but leaves animation frames to {@link nextFrame}.
 *
 * The fake timers would otherwise take over `requestAnimationFrame`, and a
 * frame would then run only when a test happened to advance the clock by one.
 */
function useFakeClock(): void {
  vi.useFakeTimers();
  stubFrames();
}

/** Runs the animation frame the browser would paint next. */
function nextFrame(): void {
  const due = [...frames.values()];
  frames.clear();
  for (const run of due) run(0);
}

beforeEach(async () => {
  stubFrames();
  undoDelete();
  for (const notice of notices()) dismissNotice(notice.id);
  vi.clearAllMocks();
  markRead.mockResolvedValue(undefined);
  markStarred.mockResolvedValue(undefined);
  deleteMessage.mockResolvedValue(undefined);
  deleteAllMessages.mockResolvedValue(undefined);
  setSearch("");
  clearFilters();
  setSelectedId(null);
  await seed([]);
});

afterEach(() => {
  disconnectWebSocket();
  vi.unstubAllGlobals();
});

describe("store reactivity", () => {
  it("recomputes the filtered list when messages land", async () => {
    expect(filteredMessages()).toEqual([]);

    await seed(range(2));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
  });
});

describe("paging by cursor", () => {
  it("reads the next page from the cursor the last page returned", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    listMessages.mockResolvedValue(page([message(2), message(3)], 4));

    await loadMore();

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: "id-1",
      },
      expect.any(AbortSignal),
    );
    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-1",
      "id-2",
      "id-3",
    ]);
  });

  it("does not page on from a cursor read under another search", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    listMessages.mockClear();
    setSearch("invoice");

    await loadMore();

    expect(listMessages).not.toHaveBeenCalled();
  });

  it("does not page on from a cursor read under other filters", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    listMessages.mockClear();
    const dispose = createRoot((dispose) => {
      createEffect(
        on(filteredMessages, () => void loadMore(), { defer: true }),
      );
      return dispose;
    });

    toggleFilter("starred");
    await vi.waitFor(() => expect(loading()).toBe(false));
    dispose();

    for (const [query] of listMessages.mock.calls) {
      expect(query).not.toHaveProperty("before");
    }
  });

  it("says so when an older page does not load", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    listMessages.mockRejectedValue(new Error("offline"));

    await loadMore();

    expect(notices().map((n) => n.text)).toEqual([
      "Could not load older messages.",
    ]);
  });

  it("says so when the cursor keeps moving under an older page", async () => {
    listMessages.mockResolvedValue(page(range(10), 20, "id-9"));
    connectAndOpen();
    await vi.waitFor(() => expect(loading()).toBe(false));
    listMessages.mockImplementation(async ({ before }: { before?: string }) => {
      if (before !== undefined) {
        deliver(
          JSON.stringify({ type: "message:delete", data: { id: before } }),
        );
      }
      return page([message(10)], 20);
    });

    await loadMore();

    expect(listMessages).toHaveBeenCalledWith(
      {
        limit: PAGE_SIZE,
        before: `id-${10 - LIST_READ_ATTEMPTS}`,
      },
      expect.any(AbortSignal),
    );
    expect(notices().map((n) => n.text)).toEqual([
      "Could not load older messages.",
    ]);
  });

  it("abandons an older-page read the filters supersede", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    const read = deferred<Page>();
    listMessages.mockReturnValueOnce(read.promise);
    const paging = loadMore();
    const [, signal] = listMessages.mock.lastCall as [unknown, AbortSignal];

    toggleFilter("starred");
    const aborted = signal.aborted;
    read.resolve(page([]));
    await paging;

    expect(aborted).toBe(true);
  });

  it("stops paging once the server has no older page", async () => {
    await seed(range(2));
    listMessages.mockClear();

    await loadMore();

    expect(hasMore()).toBe(false);
    expect(listMessages).not.toHaveBeenCalled();
  });

  it("moves the cursor back a row when its message is deleted", async () => {
    listMessages.mockResolvedValue(page(range(3), 5, "id-2"));
    await fetchMessages();
    deleteWithUndo("id-2");
    flushPendingDelete();
    await vi.waitFor(() => expect(total()).toBe(4));
    listMessages.mockResolvedValue(page([message(3)], 4));

    await loadMore();

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: "id-1",
      },
      expect.any(AbortSignal),
    );
  });

  it("reads on from the row before a cursor the server no longer knows", async () => {
    listMessages.mockResolvedValue(page(range(3), 5, "id-2"));
    await fetchMessages();
    listMessages.mockRejectedValueOnce(
      new ApiError(new Response(null, { status: 400 }), "unknown_cursor"),
    );
    listMessages.mockResolvedValueOnce(page([message(3)], 4));

    await loadMore();

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: "id-1",
      },
      expect.any(AbortSignal),
    );
    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-1",
      "id-3",
    ]);
  });

  it("keeps the last row when an older page is rejected for another reason", async () => {
    listMessages.mockResolvedValue(page(range(3), 5, "id-2"));
    await fetchMessages();
    listMessages.mockRejectedValueOnce(
      new ApiError(new Response(null, { status: 400 })),
    );

    await loadMore();

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-1",
      "id-2",
    ]);
    expect(total()).toBe(5);
    expect(notices().map((n) => n.text)).toEqual([
      "Could not load older messages.",
    ]);
  });
});

describe("moveSelection", () => {
  it("selects the first message when nothing is selected", async () => {
    await seed(range(3));

    moveSelection("next");

    expect(selectedId()).toBe("id-0");
  });

  it("selects the first message when moving backwards from no selection", async () => {
    await seed(range(3));

    moveSelection("prev");

    expect(selectedId()).toBe("id-0");
  });

  it("advances one message at a time", async () => {
    await seed(range(3));
    setSelectedId("id-0");

    moveSelection("next");
    expect(selectedId()).toBe("id-1");

    moveSelection("next");
    expect(selectedId()).toBe("id-2");
  });

  it("steps backwards one message at a time", async () => {
    await seed(range(3));
    setSelectedId("id-2");

    moveSelection("prev");

    expect(selectedId()).toBe("id-1");
  });

  it("stays on the last message at the end of the list", async () => {
    await seed(range(3));
    setSelectedId("id-2");

    moveSelection("next");

    expect(selectedId()).toBe("id-2");
    expect(markRead).not.toHaveBeenCalled();
  });

  it("stays on the first message at the start of the list", async () => {
    await seed(range(3));
    setSelectedId("id-0");

    moveSelection("prev");

    expect(selectedId()).toBe("id-0");
    expect(markRead).not.toHaveBeenCalled();
  });

  it("jumps to the first and last loaded message", async () => {
    await seed(range(600));
    setSelectedId("id-300");

    moveSelection("first");
    expect(selectedId()).toBe("id-0");

    moveSelection("last");
    expect(selectedId()).toBe("id-599");
  });

  it("reaches rows far outside any rendered window", async () => {
    await seed(range(600));

    for (let i = 0; i < 150; i++) moveSelection("next");

    expect(selectedId()).toBe("id-149");
  });

  it("does nothing on an empty list", () => {
    moveSelection("next");
    moveSelection("last");

    expect(selectedId()).toBeNull();
    expect(markRead).not.toHaveBeenCalled();
  });

  it("marks the newly selected message read", async () => {
    await seed(range(3));

    moveSelection("next");

    expect(markRead).toHaveBeenCalledExactlyOnceWith("id-0", true);
  });

  it("does not re-mark an already read message", async () => {
    await seed([message(0, { is_read: true }), message(1, { is_read: true })]);
    setSelectedId("id-0");

    moveSelection("next");

    expect(selectedId()).toBe("id-1");
    expect(markRead).not.toHaveBeenCalled();
  });

  it("walks the filtered list, skipping messages the filter hides", async () => {
    await seed([
      message(0),
      message(1, { is_read: true }),
      message(2),
      message(3, { is_read: true }),
      message(4),
    ]);
    toggleFilter("unread");
    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-2",
      "id-4",
    ]);
    setSelectedId("id-0");

    moveSelection("next");

    expect(selectedId()).toBe("id-2");
  });

  it("falls back to the first entry when the selection is gone", async () => {
    await seed(range(3));
    setSelectedId("id-does-not-exist");

    moveSelection("next");

    expect(selectedId()).toBe("id-0");
  });

  it("lands on the last visible entry, not the last loaded one", async () => {
    await seed([message(0), message(1), message(2, { is_read: true })]);
    toggleFilter("unread");

    moveSelection("last");

    expect(selectedId()).toBe("id-1");
  });
});

describe("selectMessage", () => {
  it("selects and marks an unread message read", () => {
    selectMessage(message(7));

    expect(selectedId()).toBe("id-7");
    expect(markRead).toHaveBeenCalledExactlyOnceWith("id-7", true);
  });

  it("selects a read message without a write", () => {
    selectMessage(message(7, { is_read: true }));

    expect(selectedId()).toBe("id-7");
    expect(markRead).not.toHaveBeenCalled();
  });

  it("keeps the selection when the read write fails, and says so", async () => {
    markRead.mockRejectedValue(new Error("offline"));

    selectMessage(message(7));
    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not mark the message as read.",
      ]),
    );

    expect(selectedId()).toBe("id-7");
  });
});

describe("filtering on the server", () => {
  const STARRED_ONLY = {
    starred: true,
    unread: false,
    attachments: false,
    tags: [],
  };

  it("asks the server for the starred messages in one read", async () => {
    await seed(range(3));
    listMessages.mockClear();
    listMessages.mockResolvedValue(page([message(7, { is_starred: true })]));

    toggleFilter("starred");
    await vi.waitFor(() => expect(filteredMessages()).toHaveLength(1));

    expect(listMessages).toHaveBeenCalledExactlyOnceWith(
      { limit: PAGE_SIZE, filters: STARRED_ONLY },
      expect.any(AbortSignal),
    );
    expect(total()).toBe(1);
  });

  it("reads the next page under the same filters", async () => {
    toggleFilter("starred");
    listMessages.mockResolvedValue(
      page([message(0, { is_starred: true })], 2, "id-0"),
    );
    await fetchMessages();

    await loadMore();

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: "id-0",
        filters: STARRED_ONLY,
      },
      expect.any(AbortSignal),
    );
  });

  it("does not read again when clearing filters that were never set", async () => {
    listMessages.mockClear();

    clearFilters();

    expect(listMessages).not.toHaveBeenCalled();
  });

  it("says so when the filtered read fails", async () => {
    listMessages.mockRejectedValue(new Error("offline"));

    toggleFilter("unread");

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not apply the filters.",
      ]),
    );
  });

  it("counts a message out when a flag change stops it matching", async () => {
    toggleFilter("starred");
    listMessages.mockResolvedValue(
      page([
        message(0, { is_starred: true }),
        message(1, { is_starred: true }),
      ]),
    );
    await fetchMessages();
    connectAndOpen();

    deliver(
      JSON.stringify({
        type: "message:starred",
        data: { id: "id-0", is_starred: false },
      }),
    );

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });

  it("keeps offering the inbox's tags while filtering by one of them", async () => {
    await seed([
      message(0, { tags: ["alpha"] }),
      message(1, { tags: ["beta"] }),
    ]);
    listMessages.mockResolvedValue(page([message(0, { tags: ["alpha"] })]));

    toggleTagFilter("alpha");
    await vi.waitFor(() => expect(filteredMessages()).toHaveLength(1));

    expect(allTags()).toEqual(["alpha", "beta"]);
  });

  it("does not filter by more tags than the server accepts", async () => {
    const tags = Array.from({ length: MAX_TAG_FILTERS + 1 }, (_, i) => `t${i}`);
    for (const tag of tags.slice(0, MAX_TAG_FILTERS)) toggleTagFilter(tag);
    await vi.waitFor(() => expect(loading()).toBe(false));
    listMessages.mockClear();

    toggleTagFilter(tags[MAX_TAG_FILTERS]);

    expect(filters().tags).toEqual(tags.slice(0, MAX_TAG_FILTERS));
    expect(listMessages).not.toHaveBeenCalled();
  });
});

describe("listSize", () => {
  it("counts the selected row the unread filter keeps after it is read", async () => {
    toggleFilter("unread");
    listMessages.mockResolvedValue(page([message(0), message(1)]));
    await fetchMessages();
    connectAndOpen();
    setSelectedId("id-0");

    deliver(
      JSON.stringify({
        type: "message:read",
        data: { id: "id-0", is_read: true },
      }),
    );

    expect(total()).toBe(1);
    expect(listSize()).toBe(2);
  });
});

describe("live arrivals under filters", () => {
  beforeEach(() => {
    connectAndOpen();
  });

  function arrival(over: Partial<MessageSummary>): string {
    return JSON.stringify({ type: "message:new", data: message(9, over) });
  }

  it("leaves out an arrival the filters exclude, and its count", async () => {
    toggleFilter("starred");
    listMessages.mockResolvedValue(page([message(0, { is_starred: true })]));
    await fetchMessages();

    deliver(arrival({}));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0"]);
    expect(total()).toBe(1);
  });

  it("adds an arrival the filters match", async () => {
    toggleFilter("unread");
    listMessages.mockResolvedValue(page([message(0)]));
    await fetchMessages();

    deliver(arrival({}));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-9", "id-0"]);
    expect(total()).toBe(2);
  });
});

describe("filteredMessages", () => {
  it("keeps the selected message once the filter stops matching it", async () => {
    await seed([message(0, { is_read: true }), message(1), message(2)]);
    setSelectedId("id-0");

    toggleFilter("unread");

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-1",
      "id-2",
    ]);
  });

  it("drops the previous one as the selection moves on", async () => {
    await seed([
      message(0, { is_read: true }),
      message(1, { is_read: true }),
      message(2),
    ]);
    setSelectedId("id-0");
    toggleFilter("unread");
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-2"]);

    setSelectedId("id-1");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1", "id-2"]);
  });

  it("hides a non-matching message that is not selected", async () => {
    await seed([message(0, { is_read: true }), message(1)]);
    setSelectedId(null);

    toggleFilter("unread");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
  });

  it("makes no exception for the starred filter", async () => {
    await seed([message(0), message(1, { is_starred: true })]);
    setSelectedId("id-0");

    toggleFilter("starred");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
  });

  it("makes no exception for the attachments filter", async () => {
    await seed([message(0), message(1, { has_attachments: true })]);
    setSelectedId("id-0");

    toggleFilter("attachments");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
  });

  it("leaves the list empty when nothing matches and only the selection would", async () => {
    await seed([message(0), message(1)]);
    setSelectedId("id-0");

    toggleFilter("starred");

    expect(filteredMessages()).toEqual([]);
  });

  it("drops the selected message when another active filter excludes it", async () => {
    await seed([
      message(0, { is_read: true }),
      message(1, { is_starred: true }),
    ]);
    setSelectedId("id-0");

    toggleFilter("unread");
    toggleFilter("starred");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
  });
});

describe("deleteWithUndo", () => {
  beforeEach(() => {
    useFakeClock();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("takes the message out of the list and holds the DELETE back", async () => {
    await seed(range(3));

    deleteWithUndo("id-0");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1", "id-2"]);
    expect(total()).toBe(2);
    expect(undoableId()).toBe("id-0");
    expect(deleteMessage).not.toHaveBeenCalled();
  });

  it("deletes the message once its undo window closes", async () => {
    await seed(range(3));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(deleteMessage).toHaveBeenCalledWith("id-0");
    expect(undoableId()).toBeNull();
  });

  it("hides a message once even when it is deleted twice", async () => {
    await seed(range(2));

    deleteWithUndo("id-0");
    deleteWithUndo("id-0");

    expect(total()).toBe(1);
  });

  it("writes the pending deletion out when the page goes away", async () => {
    await seed(range(2));

    deleteWithUndo("id-0");
    flushPendingDelete();

    expect(deleteMessage).toHaveBeenCalledWith("id-0");
    expect(undoableId()).toBeNull();
  });

  it("hides the message for as long as the DELETE is in flight", async () => {
    let settle = () => {};
    deleteMessage.mockReturnValue(
      new Promise<void>((resolve) => {
        settle = resolve;
      }),
    );
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(deleteMessage).toHaveBeenCalledWith("id-0");
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);

    settle();
  });

  /**
   * The response and the `message:delete` event travel on separate
   * connections, and this harness has no socket to deliver one: a row that
   * comes back here is a row that comes back on screen whenever the event is
   * late or the socket is down.
   */
  it("keeps the message out once the DELETE settles, event or not", async () => {
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });

  it("brings the message back on undo, and reselects it", async () => {
    await seed(range(3));
    setSelectedId("id-1");

    deleteWithUndo("id-0");
    undoDelete();

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-0",
      "id-1",
      "id-2",
    ]);
    expect(selectedId()).toBe("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    expect(deleteMessage).not.toHaveBeenCalled();
  });

  it("commits the deletion before it when a second one starts", async () => {
    await seed(range(3));

    deleteWithUndo("id-0");
    deleteWithUndo("id-1");

    expect(deleteMessage).toHaveBeenCalledTimes(1);
    expect(deleteMessage).toHaveBeenCalledWith("id-0");
    expect(undoableId()).toBe("id-1");
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-2"]);
  });

  it("puts the message back when the DELETE fails", async () => {
    deleteMessage.mockRejectedValue(new Error("unreachable"));
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(total()).toBe(2);
  });

  it("says why the message came back when the DELETE fails", async () => {
    deleteMessage.mockRejectedValue(new Error("unreachable"));
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(notices().map((n) => n.text)).toEqual([
      "Could not delete \u201CSubject 0\u201D. It is back in the inbox.",
    ]);
  });

  it("does nothing once the undo window has closed", async () => {
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    undoDelete();

    expect(deleteMessage).toHaveBeenCalledTimes(1);
    expect(selectedId()).toBeNull();
  });
});

/** A socket that opens nothing and hands frames straight to the store. */
class FakeSocket {
  static last: FakeSocket | null = null;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;

  constructor() {
    FakeSocket.last = this;
  }

  close(): void {}
}

/** Connects the store to a fake socket and opens it, as the server would. */
function connectAndOpen(): void {
  FakeSocket.last = null;
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
  connectWebSocket();
  openSocket();
}

/** Hands the store a frame off the socket, without running a frame. */
function receive(frame: unknown): void {
  const socket = FakeSocket.last;
  if (socket?.onmessage == null) {
    throw new Error("the store never opened a socket");
  }
  socket.onmessage({ data: frame });
}

/** A frame arrives and the next animation frame runs. */
function deliver(frame: unknown): void {
  receive(frame);
  nextFrame();
}

describe("WebSocket events", () => {
  let logged: ReturnType<typeof vi.spyOn>;

  beforeEach(async () => {
    FakeSocket.last = null;
    logged = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    await seed(range(2));
    connectWebSocket();
  });

  afterEach(() => {
    disconnectWebSocket();
    logged.mockRestore();
    vi.unstubAllGlobals();
  });

  it("applies an event that matches its schema", () => {
    deliver(
      JSON.stringify({
        type: "message:read",
        data: { id: "id-0", is_read: true },
      }),
    );

    expect(filteredMessages().map((m) => m.is_read)).toEqual([true, false]);
    expect(logged).not.toHaveBeenCalled();
  });

  it("adds a new message the server describes in full", () => {
    deliver(JSON.stringify({ type: "message:new", data: message(9) }));

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-9",
      "id-0",
      "id-1",
    ]);
  });

  it("ignores an arrival for a message the list already holds", () => {
    deliver(JSON.stringify({ type: "message:new", data: message(0) }));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(total()).toBe(2);
  });

  it("counts a message delivered twice once", () => {
    const frame = JSON.stringify({ type: "message:new", data: message(9) });

    deliver(frame);
    deliver(frame);

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-9",
      "id-0",
      "id-1",
    ]);
    expect(total()).toBe(3);
  });

  it("discards an event whose payload does not match its schema", () => {
    deliver(
      JSON.stringify({
        type: "message:new",
        data: { ...message(9), size: "2 kB" },
      }),
    );

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(logged).toHaveBeenCalled();
  });

  it("discards an event of a type the UI does not know", () => {
    deliver(JSON.stringify({ type: "message:archived", data: { id: "id-0" } }));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(logged).toHaveBeenCalled();
  });

  it("discards a frame that is not JSON", () => {
    deliver("<html>502</html>");

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(logged).toHaveBeenCalled();
  });

  it("keeps the frame it discarded out of the log", () => {
    deliver(
      JSON.stringify({
        type: "message:new",
        data: { ...message(9), subject: "Board pack Q3", size: "2 kB" },
      }),
    );

    expect(JSON.stringify(logged.mock.calls)).not.toContain("Board pack Q3");
  });
});

function openSocket(): void {
  const socket = FakeSocket.last;
  if (socket?.onopen == null) {
    throw new Error("the store never opened a socket");
  }
  socket.onopen();
}

describe("live events per frame", () => {
  beforeEach(async () => {
    await seed(range(2));
    connectAndOpen();
    await vi.waitFor(() => expect(loading()).toBe(false));
    listMessages.mockClear();
  });

  function arrival(n: number, over: Partial<MessageSummary> = {}): string {
    return JSON.stringify({ type: "message:new", data: message(n, over) });
  }

  it("holds an event until the next animation frame", () => {
    receive(arrival(9));
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);

    nextFrame();

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-9",
      "id-0",
      "id-1",
    ]);
  });

  it("recomputes the list once for a frame of arrivals", () => {
    let recomputed = 0;
    const dispose = createRoot((dispose) => {
      createEffect(
        on(filteredMessages, () => (recomputed += 1), { defer: true }),
      );
      return dispose;
    });

    for (let n = 10; n < 60; n += 1) receive(arrival(n));
    nextFrame();
    dispose();

    expect(recomputed).toBe(1);
    expect(filteredMessages()).toHaveLength(52);
    expect(total()).toBe(52);
  });

  it("applies events when no frame comes, as in a background tab", async () => {
    useFakeClock();
    try {
      receive(arrival(9));

      await vi.advanceTimersByTimeAsync(FRAME_FALLBACK_MS);

      expect(filteredMessages().map((m) => m.id)[0]).toBe("id-9");
    } finally {
      vi.useRealTimers();
    }
  });

  it("counts an arrival repeated within one frame once", () => {
    receive(arrival(9));
    receive(arrival(9));
    nextFrame();

    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-9",
      "id-0",
      "id-1",
    ]);
    expect(total()).toBe(3);
  });

  it("applies a flag change to a message that arrived in the same frame", () => {
    receive(arrival(9));
    receive(
      JSON.stringify({
        type: "message:starred",
        data: { id: "id-9", is_starred: true },
      }),
    );
    nextFrame();

    expect(filteredMessages()[0]).toMatchObject({
      id: "id-9",
      is_starred: true,
    });
  });

  it("applies a deletion in the order it arrived", () => {
    receive(arrival(9));
    receive(JSON.stringify({ type: "message:delete", data: { id: "id-9" } }));
    nextFrame();

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(total()).toBe(2);
  });

  it("reads the list again instead of replaying a backlog too long to hold", async () => {
    listMessages.mockResolvedValue(page([message(5)], 1));

    for (let n = 0; n <= MAX_QUEUED_EVENTS; n += 1) receive(arrival(1000 + n));
    nextFrame();
    await vi.waitFor(() => expect(loading()).toBe(false));

    expect(listMessages).toHaveBeenCalledTimes(1);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-5"]);
  });

  it("drops the events still queued when the socket is closed", () => {
    receive(arrival(9));

    disconnectWebSocket();
    nextFrame();

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
  });
});

describe("the live list's length", () => {
  const LAST_KEPT = MAX_LIVE_ROWS - 2;

  beforeEach(async () => {
    listMessages.mockResolvedValue(
      page(range(MAX_LIVE_ROWS), 2 * MAX_LIVE_ROWS, `id-${MAX_LIVE_ROWS - 1}`),
    );
    await fetchMessages();
    connectAndOpen();
    await vi.waitFor(() => expect(loading()).toBe(false));
  });

  it("lets the oldest row go when an arrival would pass the cap", () => {
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    const ids = filteredMessages().map((m) => m.id);
    expect(ids).toHaveLength(MAX_LIVE_ROWS);
    expect(ids[0]).toBe("id--1");
    expect(ids.at(-1)).toBe(`id-${LAST_KEPT}`);
  });

  it("keeps a row awaiting deletion counted out after letting it go", () => {
    deleteWithUndo(`id-${MAX_LIVE_ROWS - 1}`);
    expect(total()).toBe(2 * MAX_LIVE_ROWS - 1);

    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    expect(total()).toBe(2 * MAX_LIVE_ROWS);
  });

  it("brings back a row awaiting undo that an arrival would have let go", () => {
    const oldest = `id-${MAX_LIVE_ROWS - 1}`;
    deleteWithUndo(oldest);
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    undoDelete();

    expect(filteredMessages().at(-1)?.id).toBe(oldest);
    expect(total()).toBe(2 * MAX_LIVE_ROWS + 1);
  });

  it("lets a row go once it can no longer be undone", () => {
    deleteWithUndo(`id-${MAX_LIVE_ROWS - 1}`);
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));
    undoDelete();

    deliver(JSON.stringify({ type: "message:new", data: message(-2) }));

    expect(filteredMessages()).toHaveLength(MAX_LIVE_ROWS);
  });

  it("keeps the open message's star current once its row is let go", () => {
    const oldest = `id-${MAX_LIVE_ROWS - 1}`;
    setSelectedId(oldest);
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    deliver(
      JSON.stringify({
        type: "message:starred",
        data: { id: oldest, is_starred: true },
      }),
    );

    expect(liveSummary(message(MAX_LIVE_ROWS - 1)).is_starred).toBe(true);
  });

  it("keeps the open message's tags current once its row is let go", () => {
    const oldest = `id-${MAX_LIVE_ROWS - 1}`;
    setSelectedId(oldest);
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    deliver(
      JSON.stringify({
        type: "message:tags",
        data: { id: oldest, tags: ["urgent"] },
      }),
    );

    expect(liveSummary(message(MAX_LIVE_ROWS - 1)).tags).toEqual(["urgent"]);
  });

  it("still counts the rows it let go", () => {
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));

    expect(total()).toBe(2 * MAX_LIVE_ROWS + 1);
  });

  it("drops a page read from a cursor the cap has since moved", async () => {
    const read = deferred<Page>();
    listMessages.mockReturnValueOnce(read.promise);
    listMessages.mockResolvedValueOnce(
      page([message(LAST_KEPT + 1), message(MAX_LIVE_ROWS)], 0),
    );
    const paging = loadMore();

    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));
    read.resolve(page([message(MAX_LIVE_ROWS)], 0));
    await paging;

    expect(
      filteredMessages()
        .map((m) => m.id)
        .slice(-3),
    ).toEqual([
      `id-${LAST_KEPT}`,
      `id-${LAST_KEPT + 1}`,
      `id-${MAX_LIVE_ROWS}`,
    ]);
  });

  it("reads the rows it let go again from the last one kept", async () => {
    deliver(JSON.stringify({ type: "message:new", data: message(-1) }));
    listMessages.mockResolvedValue(page([message(LAST_KEPT + 1)], 0));

    await loadMore();

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: `id-${LAST_KEPT}`,
      },
      expect.any(AbortSignal),
    );
  });
});

describe("arrivals while the reader is scrolled away", () => {
  beforeEach(async () => {
    await seed(range(2));
    connectAndOpen();
    await vi.waitFor(() => expect(loading()).toBe(false));
    listMessages.mockClear();
  });

  afterEach(() => {
    setLiveHeld(false);
  });

  function arrival(n: number, over: Partial<MessageSummary> = {}): string {
    return JSON.stringify({ type: "message:new", data: message(n, over) });
  }

  function ids(): string[] {
    return filteredMessages().map((m) => m.id);
  }

  it("keeps an arrival out of the list, and counts it as new", () => {
    setLiveHeld(true);

    deliver(arrival(9));

    expect(ids()).toEqual(["id-0", "id-1"]);
    expect(heldArrivals()).toBe(1);
    expect(total()).toBe(3);
  });

  /** Answers list reads from `rows`, a page at a time, as the server would. */
  function serve(rows: MessageSummary[]): void {
    listMessages.mockImplementation(
      async ({ limit, before }: { limit: number; before?: string }) => {
        const start =
          before === undefined ? 0 : rows.findIndex((m) => m.id === before) + 1;
        const slice = rows.slice(start, start + limit);
        const more = start + limit < rows.length;
        return page(
          slice,
          rows.length,
          more ? (slice.at(-1)?.id ?? null) : null,
        );
      },
    );
  }

  async function resync(): Promise<void> {
    openSocket();
    await vi.waitFor(() => expect(loading()).toBe(false));
  }

  it("holds what a resync finds new instead of moving the rows", async () => {
    setLiveHeld(true);
    serve([message(9), ...range(2)]);

    await resync();

    expect(ids()).toEqual(["id-0", "id-1"]);
    expect(heldArrivals()).toBe(1);
    expect(total()).toBe(3);
  });

  it("shows what a held resync found once the reader is back on top", async () => {
    setLiveHeld(true);
    serve([message(9), ...range(2)]);
    await resync();

    setLiveHeld(false);

    expect(ids()).toEqual(["id-9", "id-0", "id-1"]);
  });

  it("drops a loaded row a held resync finds deleted", async () => {
    setLiveHeld(true);
    serve([message(0)]);

    await resync();

    expect(ids()).toEqual(["id-0"]);
    expect(total()).toBe(1);
  });

  it("updates a loaded row a held resync finds changed", async () => {
    setLiveHeld(true);
    serve([message(0), message(1, { is_starred: true })]);

    await resync();

    expect(filteredMessages().map((m) => m.is_starred)).toEqual([false, true]);
  });

  it("reconciles loaded rows past the first page on a held resync", async () => {
    const rows = range(PAGE_SIZE + 50);
    serve(rows);
    await fetchMessages();
    await loadMore();
    setLiveHeld(true);
    const gone = `id-${PAGE_SIZE + 20}`;
    serve([message(-1), ...rows.filter((m) => m.id !== gone)]);

    await resync();

    expect(ids()).toEqual(rows.map((m) => m.id).filter((id) => id !== gone));
    expect(heldArrivals()).toBe(1);
    expect(hasMore()).toBe(false);
  });

  it("stops a held resync a page past a deleted tail", async () => {
    listMessages.mockResolvedValue(page(range(3), 3 + 2 * PAGE_SIZE, "id-2"));
    await fetchMessages();
    setLiveHeld(true);
    const older = Array.from({ length: 2 * PAGE_SIZE }, (_, i) =>
      message(1000 + i),
    );
    serve([message(0), message(1), ...older]);
    listMessages.mockClear();

    await resync();

    expect(ids()).toEqual(["id-0", "id-1", "id-2"]);
    expect(listMessages).toHaveBeenCalledTimes(2);
    expect(hasMore()).toBe(true);
  });

  it("forgets a deleted tail a held resync kept once an older page is read", async () => {
    listMessages.mockResolvedValue(page(range(3), 3 + 2 * PAGE_SIZE, "id-2"));
    await fetchMessages();
    setLiveHeld(true);
    const older = Array.from({ length: 2 * PAGE_SIZE }, (_, i) =>
      message(1000 + i),
    );
    serve([message(0), message(1), ...older]);
    await resync();
    listMessages.mockRejectedValueOnce(
      new ApiError(new Response(null, { status: 400 }), "unknown_cursor"),
    );

    await loadMore();

    expect(ids().slice(0, 3)).toEqual(["id-0", "id-1", "id-1000"]);
  });

  it("keeps loaded rows a held resync stops short of past a run of new ones", async () => {
    const rows = range(PAGE_SIZE + 50);
    serve(rows);
    await fetchMessages();
    await loadMore();
    setLiveHeld(true);
    const between = Array.from({ length: 2 * PAGE_SIZE }, (_, i) =>
      message(1000 + i),
    );
    serve([...rows.slice(0, 10), ...between, ...rows.slice(10)]);

    await resync();

    expect(ids()).toEqual(rows.map((m) => m.id));
    expect(hasMore()).toBe(true);
  });

  it("keeps the loaded rows when a held resync finds more arrivals than it holds", async () => {
    setLiveHeld(true);
    const arrivals = Array.from({ length: MAX_LIVE_ROWS + 1 }, (_, i) =>
      message(1000 + i),
    );
    serve([...arrivals, ...range(2)]);

    await resync();

    expect(ids()).toEqual(["id-0", "id-1"]);
  });

  it("puts the held arrivals on top once the reader is back there", () => {
    setLiveHeld(true);
    deliver(arrival(8));
    deliver(arrival(9));

    setLiveHeld(false);

    expect(ids()).toEqual(["id-9", "id-8", "id-0", "id-1"]);
    expect(heldArrivals()).toBe(0);
    expect(total()).toBe(4);
  });

  it("holds only arrivals the filters match", async () => {
    toggleFilter("starred");
    listMessages.mockResolvedValue(page([message(0, { is_starred: true })]));
    await fetchMessages();
    setLiveHeld(true);

    deliver(arrival(8));
    deliver(arrival(9, { is_starred: true }));

    expect(heldArrivals()).toBe(1);
    expect(total()).toBe(2);
  });

  it("counts an arrival the list already holds as nothing new", () => {
    setLiveHeld(true);

    deliver(arrival(0));

    expect(heldArrivals()).toBe(0);
    expect(total()).toBe(2);
  });

  it("applies a flag change to a held arrival", () => {
    setLiveHeld(true);
    deliver(arrival(9));
    deliver(
      JSON.stringify({
        type: "message:starred",
        data: { id: "id-9", is_starred: true },
      }),
    );

    setLiveHeld(false);

    expect(filteredMessages()[0]).toMatchObject({
      id: "id-9",
      is_starred: true,
    });
  });

  it("drops a held arrival that stops matching the filters", async () => {
    toggleFilter("starred");
    listMessages.mockResolvedValue(page([message(0, { is_starred: true })]));
    await fetchMessages();
    setLiveHeld(true);
    deliver(arrival(9, { is_starred: true }));

    deliver(
      JSON.stringify({
        type: "message:starred",
        data: { id: "id-9", is_starred: false },
      }),
    );
    setLiveHeld(false);

    expect(heldArrivals()).toBe(0);
    expect(total()).toBe(1);
    expect(ids()).toEqual(["id-0"]);
  });

  it("forgets a held arrival that is deleted", () => {
    setLiveHeld(true);
    deliver(arrival(9));
    deliver(JSON.stringify({ type: "message:delete", data: { id: "id-9" } }));

    setLiveHeld(false);

    expect(heldArrivals()).toBe(0);
    expect(ids()).toEqual(["id-0", "id-1"]);
    expect(total()).toBe(2);
  });

  it("reads the first page on return when more arrived than it holds", async () => {
    setLiveHeld(true);
    for (let n = 0; n <= MAX_LIVE_ROWS; n += 1) receive(arrival(1000 + n));
    nextFrame();
    listMessages.mockResolvedValue(page([message(5)], 1));

    setLiveHeld(false);
    await vi.waitFor(() => expect(loading()).toBe(false));

    expect(listMessages).toHaveBeenCalledTimes(1);
    expect(ids()).toEqual(["id-5"]);
  });

  it("holds a search refresh back, and runs it on return", async () => {
    useFakeClock();
    try {
      setSearch("invoice");
      await seed([message(0)]);
      listMessages.mockClear();
      setLiveHeld(true);

      deliver(arrival(9));
      await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);
      expect(listMessages).not.toHaveBeenCalled();
      expect(heldRefresh()).toBe(true);

      setLiveHeld(false);
      await vi.advanceTimersByTimeAsync(0);

      expect(listMessages).toHaveBeenCalledTimes(1);
      expect(heldRefresh()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("resync when the socket opens", () => {
  type Page = { messages: MessageSummary[]; total: number };

  beforeEach(() => {
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
  });

  function arrival(n: number): string {
    return JSON.stringify({ type: "message:new", data: message(n) });
  }

  it("reads the list on the first open, not only on a reopen", async () => {
    connectWebSocket();
    listMessages.mockResolvedValue({ messages: range(2), total: 2 });

    openSocket();

    await vi.waitFor(() =>
      expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]),
    );
  });

  it("tells the caller once the first read has landed", async () => {
    const synced = vi.fn();
    connectWebSocket(synced);
    listMessages.mockResolvedValue({ messages: range(2), total: 2 });

    openSocket();

    await vi.waitFor(() => expect(synced).toHaveBeenCalledOnce());
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
  });

  it("says so when the first read fails", async () => {
    connectWebSocket();
    listMessages.mockRejectedValue(new Error("offline"));

    openSocket();

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not load the inbox.",
      ]),
    );
  });

  it("keeps a message that arrives while the read is in flight", async () => {
    const read = deferred<Page>();
    listMessages.mockReturnValueOnce(read.promise);
    connectWebSocket();
    openSocket();

    deliver(arrival(9));
    read.resolve({ messages: range(2), total: 2 });

    await vi.waitFor(() =>
      expect(filteredMessages().map((m) => m.id)).toEqual([
        "id-9",
        "id-0",
        "id-1",
      ]),
    );
    expect(total()).toBe(3);
  });

  it("counts an arrival the read already holds once", async () => {
    const read = deferred<Page>();
    listMessages.mockReturnValueOnce(read.promise);
    connectWebSocket();
    openSocket();

    deliver(arrival(9));
    read.resolve({ messages: [message(9), ...range(2)], total: 3 });

    await vi.waitFor(() => expect(loading()).toBe(false));
    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-9",
      "id-0",
      "id-1",
    ]);
    expect(total()).toBe(3);
  });

  it("keeps a flag change made while the read is in flight", async () => {
    const read = deferred<Page>();
    listMessages.mockReturnValueOnce(read.promise);
    connectWebSocket();
    openSocket();

    deliver(
      JSON.stringify({
        type: "message:read",
        data: { id: "id-0", is_read: true },
      }),
    );
    read.resolve({ messages: range(2), total: 2 });

    await vi.waitFor(() => expect(loading()).toBe(false));
    expect(filteredMessages().map((m) => m.is_read)).toEqual([true, false]);
  });
});

describe("loading while the socket will not open", () => {
  beforeEach(() => {
    useFakeClock();
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    listMessages.mockClear();
    listMessages.mockResolvedValue({ messages: range(2), total: 2 });
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function closeSocket(): void {
    const socket = FakeSocket.last;
    if (socket?.onclose == null) {
      throw new Error("the store never opened a socket");
    }
    socket.onclose();
  }

  it("reads the list over HTTP once the socket misses its deadline", async () => {
    const synced = vi.fn();
    connectWebSocket(synced);

    await vi.advanceTimersByTimeAsync(SOCKET_OPEN_DEADLINE_MS - 1);
    expect(listMessages).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(synced).toHaveBeenCalledOnce();
  });

  it("reads the list over HTTP as soon as the socket fails to open", async () => {
    const synced = vi.fn();
    connectWebSocket(synced);

    closeSocket();
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(synced).toHaveBeenCalledOnce();
  });

  it("skips the HTTP read when the socket opens in time", async () => {
    connectWebSocket();

    openSocket();
    await vi.advanceTimersByTimeAsync(SOCKET_OPEN_DEADLINE_MS);

    expect(listMessages).toHaveBeenCalledOnce();
  });

  it("resyncs once when the socket opens after the HTTP read", async () => {
    const synced = vi.fn();
    connectWebSocket(synced);
    await vi.advanceTimersByTimeAsync(SOCKET_OPEN_DEADLINE_MS);

    openSocket();
    await vi.advanceTimersByTimeAsync(0);

    expect(listMessages).toHaveBeenCalledTimes(2);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
    expect(total()).toBe(2);
    expect(synced).toHaveBeenCalledOnce();
  });

  it("selects on the read that lands, not on one the socket superseded", async () => {
    const fallback = deferred<{ messages: MessageSummary[]; total: number }>();
    const resync = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages
      .mockReturnValueOnce(fallback.promise)
      .mockReturnValueOnce(resync.promise);
    const synced = vi.fn(() => filteredMessages().length);
    connectWebSocket(synced);
    await vi.advanceTimersByTimeAsync(SOCKET_OPEN_DEADLINE_MS);

    openSocket();
    fallback.resolve({ messages: [], total: 0 });
    await vi.advanceTimersByTimeAsync(0);
    expect(synced).not.toHaveBeenCalled();
    resync.resolve({ messages: range(2), total: 2 });
    await vi.advanceTimersByTimeAsync(0);

    expect(synced).toHaveBeenCalledOnce();
    expect(synced).toHaveLastReturnedWith(2);
  });
});

describe("live traffic during a search", () => {
  const ROUND_TRIP_MS = 50;
  const ARRIVAL_INTERVAL_MS = 10;

  beforeEach(async () => {
    useFakeClock();
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    setSearch("invoice");
    await seed([message(0)]);
    connectWebSocket();
    listMessages.mockClear();
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function arrival(n: number): string {
    return JSON.stringify({ type: "message:new", data: message(n) });
  }

  it("folds a burst of arrivals into one trailing refetch", async () => {
    listMessages.mockResolvedValue({ messages: [message(5)], total: 1 });

    for (let n = 100; n < 150; n += 1) deliver(arrival(n));
    expect(listMessages).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);

    expect(listMessages).toHaveBeenCalledExactlyOnceWith(
      { limit: PAGE_SIZE, q: "invoice" },
      expect.any(AbortSignal),
    );
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-5"]);
    expect(loading()).toBe(false);
  });

  it("waits out a refetch in flight, then runs one more after it", async () => {
    const read = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(read.promise);
    listMessages.mockResolvedValue({ messages: [message(6)], total: 1 });

    deliver(arrival(100));
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);
    deliver(arrival(101));
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);
    expect(listMessages).toHaveBeenCalledTimes(1);

    read.resolve({ messages: [message(5)], total: 1 });
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);

    expect(listMessages).toHaveBeenCalledTimes(2);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-6"]);
  });

  it("lands results and settles loading while mail keeps arriving", async () => {
    const TRAFFIC_MS = 5 * SEARCH_REFRESH_WINDOW_MS;
    listMessages.mockImplementation(
      () =>
        new Promise((resolve) =>
          setTimeout(
            () => resolve({ messages: [message(5)], total: 1 }),
            ROUND_TRIP_MS,
          ),
        ),
    );

    for (let at = 0; at < TRAFFIC_MS; at += ARRIVAL_INTERVAL_MS) {
      deliver(arrival(1000 + at));
      await vi.advanceTimersByTimeAsync(ARRIVAL_INTERVAL_MS);
    }

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-5"]);
    expect(listMessages.mock.calls.length).toBeLessThanOrEqual(
      TRAFFIC_MS / SEARCH_REFRESH_WINDOW_MS,
    );
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS + ROUND_TRIP_MS);
    expect(loading()).toBe(false);
  });

  it("drops the pending refetch once the search is cleared", async () => {
    deliver(arrival(100));
    setSearch("");

    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);

    expect(listMessages).not.toHaveBeenCalled();
  });

  it("cancels a scheduled refresh on disconnect", async () => {
    deliver(arrival(100));

    disconnectWebSocket();
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS * 2);

    expect(listMessages).not.toHaveBeenCalled();
  });

  it("aborts an in-flight read on disconnect and never applies its result", async () => {
    const read = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(read.promise);

    deliver(arrival(100));
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);
    expect(listMessages).toHaveBeenCalledTimes(1);

    disconnectWebSocket();
    read.resolve({ messages: [message(5)], total: 1 });
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS * 2);

    expect(listMessages).toHaveBeenCalledTimes(1);
    expect(loading()).toBe(false);
    expect(filteredMessages().map((m) => m.id)).not.toEqual(["id-5"]);
  });

  it("keeps the search stale when the servicing read fails, so the schedule still runs", async () => {
    deliver(arrival(100));

    listMessages.mockRejectedValueOnce(new Error("offline"));
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);
    expect(listMessages).toHaveBeenCalledTimes(1);

    listMessages.mockResolvedValueOnce({ messages: [message(5)], total: 1 });
    await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_WINDOW_MS);

    expect(listMessages).toHaveBeenCalledTimes(2);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-5"]);
  });
});

describe("a superseded list read", () => {
  it("is aborted when a newer read starts", async () => {
    const signals: AbortSignal[] = [];
    listMessages.mockImplementation((_query: unknown, signal: AbortSignal) => {
      signals.push(signal);
      return new Promise((_resolve, reject) =>
        signal.addEventListener("abort", () => reject(signal.reason)),
      );
    });

    const first = fetchMessages();
    const second = fetchMessages();

    expect(signals.map((s) => s.aborted)).toEqual([true, false]);
    await expect(first).resolves.toBeUndefined();
    expect(loading()).toBe(true);

    listMessages.mockResolvedValue({ messages: [message(1)], total: 1 });
    await fetchMessages();
    await expect(second).resolves.toBeUndefined();
    expect(loading()).toBe(false);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
  });

  it("still reports a failure of the read that was current", async () => {
    listMessages.mockRejectedValue(new Error("offline"));

    await expect(fetchMessages()).rejects.toThrow("offline");
    expect(loading()).toBe(false);
  });
});

/**
 * A deletion is reported twice, over two connections that do not order
 * themselves: once by the DELETE response, once by the `message:delete` event.
 * These cover both arrival orders, and the case where the event is about a
 * message this client never asked to delete.
 */
describe("a deletion reported over both connections", () => {
  beforeEach(async () => {
    useFakeClock();
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    await seed(range(2));
    connectWebSocket();
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function deletion(id: string): string {
    return JSON.stringify({ type: "message:delete", data: { id } });
  }

  it("counts the message out once when the response arrives first", async () => {
    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    deliver(deletion("id-0"));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });

  it("counts the message out once when the event arrives first", async () => {
    let settle = () => {};
    deleteMessage.mockReturnValue(
      new Promise<void>((resolve) => {
        settle = resolve;
      }),
    );

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    deliver(deletion("id-0"));
    expect(total()).toBe(1);

    settle();
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });

  it("gives up the undo when the server deletes that message first", async () => {
    deleteWithUndo("id-0");

    deliver(deletion("id-0"));

    expect(undoableId()).toBeNull();
    expect(total()).toBe(1);

    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);

    expect(deleteMessage).not.toHaveBeenCalled();
  });

  it("leaves a cleared inbox alone when an older DELETE settles", async () => {
    let settle = () => {};
    deleteMessage.mockReturnValue(
      new Promise<void>((resolve) => {
        settle = resolve;
      }),
    );

    deleteWithUndo("id-0");
    flushPendingDelete();
    deliver(JSON.stringify({ type: "messages:clear" }));
    deliver(JSON.stringify({ type: "message:new", data: message(9) }));

    settle();
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-9"]);
    expect(total()).toBe(1);
  });

  it("leaves a refetched list alone when an older DELETE settles", async () => {
    let settle = () => {};
    deleteMessage.mockReturnValue(
      new Promise<void>((resolve) => {
        settle = resolve;
      }),
    );

    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed([message(1), message(2)]);

    settle();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1", "id-2"]);
    expect(total()).toBe(2);
  });

  it.each(["http", "socket"])(
    "removes a deleted row after a refresh still containing it, with %s first",
    async (first) => {
      const pending = deferred<void>();
      deleteMessage.mockReturnValue(pending.promise);
      deleteWithUndo("id-0");
      flushPendingDelete();
      await seed(range(2));
      listMessages.mockResolvedValue({ messages: [message(1)], total: 1 });

      if (first === "socket") deliver(deletion("id-0"));
      pending.resolve();
      await vi.advanceTimersByTimeAsync(0);
      if (first === "http") deliver(deletion("id-0"));

      expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
      expect(total()).toBe(1);
      expect(listMessages).toHaveBeenLastCalledWith({ limit: 1 });
    },
  );

  it.each([3, 4])(
    "keeps all loaded pages when their total during the DELETE was %i",
    async (pageTotal) => {
      listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
      await fetchMessages();
      const pending = deferred<void>();
      deleteMessage.mockReturnValue(pending.promise);
      deleteWithUndo("id-0");
      flushPendingDelete();
      listMessages.mockResolvedValue({
        messages: [message(2), message(3)],
        total: pageTotal,
      });
      await loadMore();
      listMessages.mockResolvedValue({ messages: [message(1)], total: 3 });

      pending.resolve();
      await vi.advanceTimersByTimeAsync(0);
      deliver(deletion("id-0"));

      expect(filteredMessages().map((m) => m.id)).toEqual([
        "id-1",
        "id-2",
        "id-3",
      ]);
      expect(total()).toBe(3);
    },
  );

  it("does not subtract a hidden message already absent from a refreshed list", async () => {
    const pending = deferred<void>();
    deleteMessage.mockReturnValue(pending.promise);
    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed([message(1)]);

    expect(total()).toBe(1);
    pending.resolve();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));
    expect(total()).toBe(1);
  });

  it("retries a refresh whose response predates a confirmed deletion", async () => {
    const read = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(read.promise);
    const refreshing = fetchMessages();
    deleteWithUndo("id-0");
    flushPendingDelete();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));
    listMessages.mockResolvedValue({ messages: [message(1)], total: 1 });

    read.resolve({ messages: range(2), total: 2 });
    await refreshing;

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });

  it("re-reads an older page that raced a deletion from the same cursor", async () => {
    listMessages.mockResolvedValue(page(range(2), 4, "id-1"));
    await fetchMessages();
    const read = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(read.promise);
    const paging = loadMore();
    deleteWithUndo("id-0");
    flushPendingDelete();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));
    listMessages.mockResolvedValue({
      messages: [message(2), message(3)],
      total: 3,
    });

    read.resolve({ messages: [message(2), message(3)], total: 4 });
    await paging;

    expect(listMessages).toHaveBeenLastCalledWith(
      {
        limit: PAGE_SIZE,
        before: "id-1",
      },
      expect.any(AbortSignal),
    );
    expect(filteredMessages().map((m) => m.id)).toEqual([
      "id-1",
      "id-2",
      "id-3",
    ]);
    expect(total()).toBe(3);
  });

  it("ignores an old count response after clear and new arrivals", async () => {
    const pending = deferred<void>();
    deleteMessage.mockReturnValue(pending.promise);
    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed(range(2));
    const count = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValue(count.promise);
    pending.resolve();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));
    deliver(JSON.stringify({ type: "messages:clear" }));
    deliver(JSON.stringify({ type: "message:new", data: message(9) }));

    count.resolve({ messages: [], total: 0 });
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-9"]);
    expect(total()).toBe(1);
  });

  it("does not let an older count overwrite a second confirmed deletion", async () => {
    await seed(range(3));
    const pending = deferred<void>();
    deleteMessage.mockReturnValue(pending.promise);
    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed(range(3));
    const oldCount = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(oldCount.promise);
    pending.resolve();
    await vi.advanceTimersByTimeAsync(0);

    listMessages.mockResolvedValue({ messages: [message(2)], total: 1 });
    deleteMessage.mockResolvedValue(undefined);
    deleteWithUndo("id-1");
    flushPendingDelete();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));
    deliver(deletion("id-1"));
    oldCount.resolve({ messages: [message(1)], total: 2 });
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-2"]);
    expect(total()).toBe(1);
  });

  it("preserves a new arrival while an ambiguous count is being refreshed", async () => {
    const pending = deferred<void>();
    deleteMessage.mockReturnValue(pending.promise);
    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed(range(2));
    const oldCount = deferred<{ messages: MessageSummary[]; total: number }>();
    listMessages.mockReturnValueOnce(oldCount.promise);
    pending.resolve();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));

    listMessages.mockResolvedValue({ messages: [message(9)], total: 2 });
    deliver(JSON.stringify({ type: "message:new", data: message(9) }));
    await vi.advanceTimersByTimeAsync(0);
    oldCount.resolve({ messages: [message(1)], total: 1 });
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-9", "id-1"]);
    expect(total()).toBe(2);
  });

  it("keeps a confirmed deletion removed if refreshing the count fails", async () => {
    const pending = deferred<void>();
    deleteMessage.mockReturnValue(pending.promise);
    deleteWithUndo("id-0");
    flushPendingDelete();
    await seed(range(2));
    listMessages.mockRejectedValue(new Error("offline"));
    pending.resolve();
    await vi.advanceTimersByTimeAsync(0);
    deliver(deletion("id-0"));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(notices().map((notice) => notice.text)).toEqual([
      "The message was deleted, but the inbox count could not be refreshed.",
    ]);
  });

  /**
   * Retention purges a batch one `message:delete` at a time, so a list read
   * can lose every race it retries. The read has to stop asking and leave the
   * count to a refresh.
   */
  it("stops re-reading the list when deletions keep confirming", async () => {
    const COUNT_ONLY_READ = 1;
    let pageReads = 0;
    listMessages.mockImplementation(async ({ limit }: { limit: number }) => {
      if (limit === COUNT_ONLY_READ) return { messages: [], total: 1 };
      pageReads += 1;
      deliver(deletion(`purged-${pageReads}`));
      return { messages: [message(1)], total: 1 };
    });

    await fetchMessages();

    expect(pageReads).toBe(LIST_READ_ATTEMPTS);
  });

  it("stays quiet when the deletion landed and only the response did not", async () => {
    let fail: (reason: unknown) => void = () => {};
    deleteMessage.mockReturnValue(
      new Promise<void>((_resolve, reject) => {
        fail = reject;
      }),
    );

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    deliver(deletion("id-0"));

    fail(new Error("connection reset"));
    await vi.advanceTimersByTimeAsync(0);

    expect(notices()).toEqual([]);
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);
  });
});

describe("starMessage", () => {
  it("names the message that could not be starred", async () => {
    await seed(range(2));
    markStarred.mockRejectedValue(new Error("offline"));

    starMessage("id-0", true);

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not star \u201CSubject 0\u201D.",
      ]),
    );
  });

  it("names unstarring when that is what failed", async () => {
    await seed(range(2));
    markStarred.mockRejectedValue(new Error("offline"));

    starMessage("id-0", false);

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not unstar \u201CSubject 0\u201D.",
      ]),
    );
  });

  it("falls back to a plain noun when the message has no subject", async () => {
    await seed([message(0, { subject: null })]);
    markStarred.mockRejectedValue(new Error("offline"));

    starMessage("id-0", true);

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not star the message.",
      ]),
    );
  });

  it("trims a subject too long to sit in a notice", async () => {
    const subject = "x".repeat(NOTICE_SUBJECT_MAX * 2);
    await seed([message(0, { subject })]);
    markStarred.mockRejectedValue(new Error("offline"));

    starMessage("id-0", true);

    await vi.waitFor(() => expect(notices()).toHaveLength(1));
    expect(notices()[0]?.text).toBe(
      `Could not star \u201C${"x".repeat(NOTICE_SUBJECT_MAX)}\u2026\u201D.`,
    );
  });

  it("cuts a long subject on a character, not half an emoji", async () => {
    const subject = "\u{1F4E7}".repeat(NOTICE_SUBJECT_MAX * 2);
    await seed([message(0, { subject })]);
    markStarred.mockRejectedValue(new Error("offline"));

    starMessage("id-0", true);

    await vi.waitFor(() => expect(notices()).toHaveLength(1));
    expect(notices()[0]?.text).toBe(
      `Could not star \u201C${"\u{1F4E7}".repeat(NOTICE_SUBJECT_MAX)}\u2026\u201D.`,
    );
    expect(notices()[0]?.text).not.toContain("\uFFFD");
  });

  it("stays quiet when the write lands", async () => {
    starMessage("id-0", true);

    await vi.waitFor(() => expect(markStarred).toHaveBeenCalled());
    expect(notices()).toEqual([]);
  });
});

describe("clearInbox", () => {
  it("says so when the whole-inbox delete does not land", async () => {
    deleteAllMessages.mockRejectedValue(new Error("offline"));

    await clearInbox();

    expect(notices().map((n) => n.text)).toEqual([
      "Could not clear the inbox. The messages are still here.",
    ]);
  });

  it("stays quiet when the inbox clears", async () => {
    await clearInbox();

    expect(deleteAllMessages).toHaveBeenCalled();
    expect(notices()).toEqual([]);
  });
});

describe("read notices", () => {
  it("stays generic so a walk down the list does not stack them", async () => {
    await seed(range(3));
    markRead.mockRejectedValue(new Error("offline"));

    selectMessage(message(0));
    selectMessage(message(1));
    selectMessage(message(2));

    await vi.waitFor(() =>
      expect(notices().map((n) => n.text)).toEqual([
        "Could not mark the message as read.",
      ]),
    );
  });
});

describe("clearInboxPrompt", () => {
  const everything =
    "Every message in the inbox will be permanently deleted, including those outside the current search and filters.";

  it("counts the inbox when nothing narrows the list", async () => {
    await seed(range(3));

    expect(clearInboxPrompt().message).toBe(
      "All 3 messages will be permanently deleted.",
    );
  });

  it("does not quote a filtered total for a clear that deletes everything", async () => {
    await seed(range(100));
    listMessages.mockResolvedValue(page(range(2), 2));
    toggleFilter("starred");
    await vi.waitFor(() => expect(total()).toBe(2));

    expect(clearInboxPrompt().message).toBe(everything);
  });

  it("does not quote a search total for a clear that deletes everything", async () => {
    await seed(range(100));
    setSearch("invoice");

    expect(clearInboxPrompt().message).toBe(everything);
  });
});

describe("flag changes under a filter", () => {
  beforeEach(async () => {
    useFakeClock();
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    listMessages.mockResolvedValue(page([message(0, { is_starred: true })]));
    toggleFilter("starred");
    await vi.advanceTimersByTimeAsync(0);
    connectWebSocket();
    listMessages.mockClear();
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("asks the server for the count when a message it never loaded is deleted", async () => {
    listMessages.mockResolvedValue(page([], 1));

    deliver(JSON.stringify({ type: "message:delete", data: { id: "id-7" } }));
    await vi.advanceTimersByTimeAsync(0);

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0"]);
    expect(total()).toBe(1);
    expect(listMessages).toHaveBeenCalledWith(
      expect.objectContaining({ limit: 1 }),
    );
  });
});

describe("deleting a row a flag change took out of the filter", () => {
  beforeEach(async () => {
    FakeSocket.last = null;
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "inbox.test" });
    listMessages.mockResolvedValue(
      page([
        message(0, { is_starred: true }),
        message(1, { is_starred: true }),
      ]),
    );
    toggleFilter("starred");
    await vi.waitFor(() => expect(loading()).toBe(false));
    connectWebSocket();
  });

  afterEach(() => {
    disconnectWebSocket();
    vi.unstubAllGlobals();
  });

  it("does not count it out a second time", () => {
    deliver(
      JSON.stringify({
        type: "message:starred",
        data: { id: "id-0", is_starred: false },
      }),
    );
    expect(total()).toBe(1);

    deliver(JSON.stringify({ type: "message:delete", data: { id: "id-0" } }));

    expect(total()).toBe(1);
  });
});
