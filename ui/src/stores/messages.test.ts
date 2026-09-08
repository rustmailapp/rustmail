import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../lib/types";

const { deleteMessage, listMessages, markRead } = vi.hoisted(() => ({
  deleteMessage: vi.fn(),
  listMessages: vi.fn(),
  markRead: vi.fn(),
}));

vi.mock("../lib/api", () => ({ deleteMessage, listMessages, markRead }));

const {
  UNDO_WINDOW_MS,
  clearFilters,
  deleteWithUndo,
  flushPendingDelete,
  fetchMessages,
  filteredMessages,
  moveSelection,
  selectMessage,
  selectedId,
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

async function seed(msgs: MessageSummary[]): Promise<void> {
  listMessages.mockResolvedValue({ messages: msgs, total: msgs.length });
  await fetchMessages();
}

function range(count: number): MessageSummary[] {
  return Array.from({ length: count }, (_, i) => message(i));
}

beforeEach(async () => {
  undoDelete();
  vi.clearAllMocks();
  markRead.mockResolvedValue(undefined);
  deleteMessage.mockResolvedValue(undefined);
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

  it("keeps the selection when the read write fails", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    markRead.mockRejectedValue(new Error("offline"));

    selectMessage(message(7));
    await vi.waitFor(() => expect(logged).toHaveBeenCalled());

    expect(selectedId()).toBe("id-7");
    logged.mockRestore();
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

    expect(deleteMessage).toHaveBeenCalledWith("id-0", { keepalive: false });
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

    expect(deleteMessage).toHaveBeenCalledWith("id-0", { keepalive: true });
    expect(undoableId()).toBeNull();
  });

  /**
   * The row is taken out by the `message:delete` event, not by the response,
   * and this harness has no socket to deliver one: the message coming back is
   * what "the hiding is over" looks like here.
   */
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
    expect(deleteMessage).toHaveBeenCalledWith("id-0", { keepalive: false });
    expect(filteredMessages().map((m) => m.id)).toEqual(["id-1"]);
    expect(total()).toBe(1);

    settle();
    await vi.advanceTimersByTimeAsync(0);

    expect(total()).toBe(2);
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
    expect(deleteMessage).toHaveBeenCalledWith("id-0", { keepalive: false });
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

  it("does nothing once the undo window has closed", async () => {
    await seed(range(2));

    deleteWithUndo("id-0");
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    undoDelete();

    expect(deleteMessage).toHaveBeenCalledTimes(1);
    expect(selectedId()).toBeNull();
  });
});
