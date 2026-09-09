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

vi.mock("../lib/api", () => ({
  deleteAllMessages,
  deleteMessage,
  listMessages,
  markRead,
  markStarred,
}));

const {
  LIST_READ_ATTEMPTS,
  NOTICE_SUBJECT_MAX,
  UNDO_WINDOW_MS,
  clearFilters,
  clearInbox,
  connectWebSocket,
  disconnectWebSocket,
  deleteWithUndo,
  flushPendingDelete,
  fetchMessages,
  filteredMessages,
  moveSelection,
  loadMore,
  selectMessage,
  selectedId,
  starMessage,
  setSearch,
  setSelectedId,
  toggleFilter,
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

async function seed(msgs: MessageSummary[]): Promise<void> {
  listMessages.mockResolvedValue({ messages: msgs, total: msgs.length });
  await fetchMessages();
}

function range(count: number): MessageSummary[] {
  return Array.from({ length: count }, (_, i) => message(i));
}

beforeEach(async () => {
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

describe("store reactivity", () => {
  it("recomputes the filtered list when messages land", async () => {
    expect(filteredMessages()).toEqual([]);

    await seed(range(2));

    expect(filteredMessages().map((m) => m.id)).toEqual(["id-0", "id-1"]);
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
    vi.useFakeTimers();
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

function deliver(frame: unknown): void {
  const socket = FakeSocket.last;
  if (socket?.onmessage == null) {
    throw new Error("the store never opened a socket");
  }
  socket.onmessage({ data: frame });
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

/**
 * A deletion is reported twice, over two connections that do not order
 * themselves: once by the DELETE response, once by the `message:delete` event.
 * These cover both arrival orders, and the case where the event is about a
 * message this client never asked to delete.
 */
describe("a deletion reported over both connections", () => {
  beforeEach(async () => {
    vi.useFakeTimers();
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
      expect(listMessages).toHaveBeenLastCalledWith(1, 0, undefined);
    },
  );

  it.each([3, 4])(
    "keeps all loaded pages when their total during the DELETE was %i",
    async (pageTotal) => {
      listMessages.mockResolvedValue({ messages: range(2), total: 4 });
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

  it("retries an old page using the offset after deletion", async () => {
    listMessages.mockResolvedValue({ messages: range(2), total: 4 });
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

    expect(listMessages).toHaveBeenLastCalledWith(100, 1, undefined);
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
    listMessages.mockImplementation(async (limit: number) => {
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
