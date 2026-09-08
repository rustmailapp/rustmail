import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../lib/types";

const { listMessages, markRead } = vi.hoisted(() => ({
  listMessages: vi.fn(),
  markRead: vi.fn(),
}));

vi.mock("../lib/api", () => ({ listMessages, markRead }));

const {
  clearFilters,
  fetchMessages,
  filteredMessages,
  moveSelection,
  selectMessage,
  selectedId,
  setSearch,
  setSelectedId,
  toggleFilter,
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
  vi.clearAllMocks();
  markRead.mockResolvedValue(undefined);
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

  it("falls back to the first entry when the selection is filtered out", async () => {
    await seed([message(0, { is_read: true }), message(1), message(2)]);
    setSelectedId("id-0");
    toggleFilter("unread");

    moveSelection("next");

    expect(selectedId()).toBe("id-1");
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
