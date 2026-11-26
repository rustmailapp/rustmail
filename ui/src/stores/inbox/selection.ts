import type { MessageSummary } from "../../lib/types";
import { notify } from "../notices";
import * as api from "../../lib/api";
import { selectedId, setSelectedId } from "./rows";
import { filteredMessages } from "./view";

/** Where {@link moveSelection} should land, relative to the current selection. */
export type SelectionTarget = "next" | "prev" | "first" | "last";

/**
 * Selects a message and marks it read.
 *
 * The read flag is written optimistically: the WebSocket `message:read` event
 * is what syncs the list, so a failed write only costs the unread dot.
 */
export function selectMessage(msg: MessageSummary): void {
  setSelectedId(msg.id);
  if (!msg.is_read) {
    api
      .markRead(msg.id, true)
      .catch(() => notify("Could not mark the message as read."));
  }
}

function targetIndex(
  to: SelectionTarget,
  current: number,
  count: number,
): number {
  switch (to) {
    case "first":
      return 0;
    case "last":
      return count - 1;
    case "next":
      return current < 0 ? 0 : Math.min(current + 1, count - 1);
    case "prev":
      return current < 0 ? 0 : Math.max(current - 1, 0);
  }
}

/**
 * Moves the selection within {@link filteredMessages}.
 *
 * Navigation is index-based rather than DOM-based so it stays correct while the
 * inbox list is virtualized and most rows are unmounted. `last` lands on the
 * last *loaded* message, not the last on the server.
 */
export function moveSelection(to: SelectionTarget): void {
  const msgs = filteredMessages();
  if (msgs.length === 0) return;

  const current = msgs.findIndex((m) => m.id === selectedId());
  const index = targetIndex(to, current, msgs.length);
  if (index === current) return;

  const msg = msgs[index];
  if (msg) selectMessage(msg);
}
