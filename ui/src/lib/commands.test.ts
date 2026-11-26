import { describe, expect, it, vi } from "vitest";
import {
  availableCommands,
  type CommandActions,
  type CommandState,
} from "./commands";
import type { MessageSummary } from "./types";

const MESSAGE: MessageSummary = {
  id: "msg-1",
  sender: "alice@example.test",
  recipients: ["bob@example.test"],
  subject: "Welcome",
  size: 1024,
  has_attachments: false,
  is_read: true,
  is_starred: false,
  tags: [],
  created_at: "2026-01-01T00:00:00Z",
  released_at: null,
  release_outcome: null,
};

function state(overrides: Partial<CommandState> = {}): CommandState {
  return {
    selected: null,
    releaseConfigured: false,
    releasing: false,
    detailsShown: false,
    canUndo: false,
    total: 0,
    filters: { starred: false, unread: false, attachments: false, tags: [] },
    tags: [],
    tagFiltersFull: false,
    theme: "system",
    palette: "rustmail",
    palettes: [
      { id: "rustmail", label: "RustMail" },
      { id: "graphite", label: "Graphite" },
      { id: "tide", label: "Tide" },
    ],
    rusted: false,
    previewWidth: "desktop",
    previewWidths: [
      { id: "desktop", label: "Desktop" },
      { id: "mobile", label: "Mobile" },
    ],
    ...overrides,
  };
}

function fakeActions(): CommandActions {
  return {
    setStarred: vi.fn(),
    setRead: vi.fn(),
    deleteSelected: vi.fn(),
    undoDelete: vi.fn(),
    downloadEml: vi.fn(),
    release: vi.fn(),
    toggleDetails: vi.fn(),
    toggleFilter: vi.fn(),
    toggleTagFilter: vi.fn(),
    clearFilters: vi.fn(),
    clearInbox: vi.fn(),
    setTheme: vi.fn(),
    setPalette: vi.fn(),
    setRusted: vi.fn(),
    setPreviewWidth: vi.fn(),
    openSettings: vi.fn(),
    showShortcuts: vi.fn(),
  };
}

function ids(s: CommandState): string[] {
  return availableCommands(s, fakeActions()).map((c) => c.id);
}

function find(s: CommandState, id: string, actions = fakeActions()) {
  const command = availableCommands(s, actions).find((c) => c.id === id);
  if (!command) throw new Error(`no command ${id}`);
  return command;
}

describe("availableCommands", () => {
  it("hides every message action while nothing is selected", () => {
    expect(ids(state()).filter((id) => id.startsWith("message."))).toEqual([]);
  });

  it("offers the message actions for the selected message", () => {
    expect(
      ids(state({ selected: MESSAGE })).filter((id) =>
        id.startsWith("message."),
      ),
    ).toEqual([
      "message.star",
      "message.read",
      "message.delete",
      "message.download",
      "message.details",
    ]);
  });

  it("names the star and read toggles after what they will do", () => {
    const plain = state({ selected: MESSAGE });
    expect(find(plain, "message.star").label).toBe("Star message");
    expect(find(plain, "message.read").label).toBe("Mark as unread");

    const flagged = state({
      selected: { ...MESSAGE, is_starred: true, is_read: false },
    });
    expect(find(flagged, "message.star").label).toBe("Unstar message");
    expect(find(flagged, "message.read").label).toBe("Mark as read");
  });

  it("flips the flags of the selected message", () => {
    const actions = fakeActions();
    const s = state({ selected: MESSAGE });

    find(s, "message.star", actions).run();
    find(s, "message.read", actions).run();

    expect(actions.setStarred).toHaveBeenCalledWith("msg-1", true);
    expect(actions.setRead).toHaveBeenCalledWith("msg-1", false);
  });

  it("offers release only with a relay and no release in flight", () => {
    expect(ids(state({ selected: MESSAGE }))).not.toContain("message.release");
    expect(
      ids(state({ selected: MESSAGE, releaseConfigured: true })),
    ).toContain("message.release");
    expect(
      ids(
        state({ selected: MESSAGE, releaseConfigured: true, releasing: true }),
      ),
    ).not.toContain("message.release");
  });

  it("hands release the selected message rather than sending", () => {
    const actions = fakeActions();
    find(
      state({ selected: MESSAGE, releaseConfigured: true }),
      "message.release",
      actions,
    ).run();
    expect(actions.release).toHaveBeenCalledExactlyOnceWith(MESSAGE);
  });

  it("shows each command's existing shortcut", () => {
    const s = state({ selected: MESSAGE, canUndo: true, total: 3 });
    expect(find(s, "message.star").shortcut).toBe("s");
    expect(find(s, "message.delete").shortcut).toBe("d");
    expect(find(s, "message.details").shortcut).toBe("i");
    expect(find(s, "inbox.undo").shortcut).toBe("u");
    expect(find(s, "inbox.clear").shortcut).toBe("⇧D");
  });

  it("offers undo only while a deletion can be taken back", () => {
    expect(ids(state())).not.toContain("inbox.undo");
    expect(ids(state({ canUndo: true }))).toContain("inbox.undo");
  });

  it("offers clearing the inbox only when it holds mail", () => {
    expect(ids(state())).not.toContain("inbox.clear");
    expect(ids(state({ total: 1 }))).toContain("inbox.clear");
  });

  it("offers clearing filters only while one is on", () => {
    expect(ids(state())).not.toContain("filter.clear");
    const filtered = state({
      filters: { starred: false, unread: true, attachments: false, tags: [] },
    });
    expect(ids(filtered)).toContain("filter.clear");
    expect(find(filtered, "filter.unread").label).toBe("Remove unread filter");
    expect(find(filtered, "filter.starred").label).toBe("Filter by starred");
  });

  it("offers a filter per tag, and only removals once the cap is reached", () => {
    const tags = ["billing", "signup"];
    expect(ids(state({ tags }))).toEqual(
      expect.arrayContaining(["filter.tag.billing", "filter.tag.signup"]),
    );

    const full = state({
      tags,
      tagFiltersFull: true,
      filters: {
        starred: false,
        unread: false,
        attachments: false,
        tags: ["billing"],
      },
    });
    expect(find(full, "filter.tag.billing").label).toBe(
      "Remove tag filter: billing",
    );
    expect(ids(full)).not.toContain("filter.tag.signup");
  });

  it("hides the theme, palette and preview width already in use", () => {
    const all = ids(state({ theme: "dark", palette: "tide" }));
    expect(all).toEqual(
      expect.arrayContaining([
        "theme.light",
        "theme.system",
        "palette.rustmail",
        "palette.graphite",
        "preview.mobile",
      ]),
    );
    expect(all).not.toContain("theme.dark");
    expect(all).not.toContain("palette.tide");
    expect(all).not.toContain("preview.desktop");
  });

  it("toggles Rusted to the other state", () => {
    const actions = fakeActions();
    const on = find(state({ rusted: true }), "rusted", actions);
    expect(on.label).toBe("Turn Rusted off");
    on.run();
    expect(actions.setRusted).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("names the details toggle after the current layout state", () => {
    expect(find(state({ selected: MESSAGE }), "message.details").label).toBe(
      "Show details",
    );
    expect(
      find(state({ selected: MESSAGE, detailsShown: true }), "message.details")
        .label,
    ).toBe("Hide details");
  });

  it("always offers settings and the shortcut list", () => {
    expect(ids(state())).toEqual(
      expect.arrayContaining(["settings", "shortcuts"]),
    );
  });

  it("never lists a command twice", () => {
    const all = ids(
      state({
        selected: MESSAGE,
        releaseConfigured: true,
        canUndo: true,
        total: 5,
        tags: ["a", "b"],
        filters: {
          starred: true,
          unread: true,
          attachments: true,
          tags: ["a"],
        },
      }),
    );
    expect(new Set(all).size).toBe(all.length);
  });
});
