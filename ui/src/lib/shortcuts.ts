/** The keys of the page-wide shortcuts, as they are shown to the user. */
export const KEYS = {
  next: "j",
  previous: "k",
  delete: "d",
  undo: "u",
  clearInbox: "⇧D",
  star: "s",
  search: "/",
  details: "i",
  escape: "Esc",
} as const;

/** Whether the browser runs on an Apple platform, where Command replaces Control. */
function applePlatform(): boolean {
  return (
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad/.test(navigator.userAgent)
  );
}

/** The command palette's chord as shown to the user on this platform. */
export function commandPaletteKeys(): string {
  return applePlatform() ? "⌘K" : "Ctrl K";
}

/**
 * Whether a key press opens or closes the command palette.
 *
 * Command+K and Control+K both count on every platform, so the chord works
 * whichever key the user reaches for; Shift or Alt make it a different chord.
 */
export function isCommandPaletteChord(
  event: Pick<
    KeyboardEvent,
    "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"
  >,
): boolean {
  return (
    (event.key === "k" || event.key === "K") &&
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey
  );
}

/** Every shortcut, in the order the shortcuts dialog lists them. */
export function shortcutList(): readonly { keys: string[]; label: string }[] {
  return [
    { keys: [commandPaletteKeys()], label: "Open the command palette" },
    { keys: [KEYS.next, KEYS.previous], label: "Select next or previous" },
    { keys: ["↓", "↑"], label: "Move in the message list" },
    { keys: [KEYS.search], label: "Search" },
    { keys: [KEYS.star], label: "Star or unstar" },
    { keys: [KEYS.delete], label: "Delete" },
    { keys: [KEYS.undo], label: "Undo delete" },
    { keys: [KEYS.clearInbox], label: "Clear inbox" },
    { keys: [KEYS.details], label: "Show or hide details" },
    {
      keys: [KEYS.escape],
      label: "Close details, clear filters, or deselect",
    },
  ];
}
