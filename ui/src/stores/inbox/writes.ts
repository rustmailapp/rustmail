import { notify } from "../notices";
import type { ConfirmDialogOptions } from "../../components/ConfirmDialog";
import * as api from "../../lib/api";
import { NOTICE_SUBJECT_MAX } from "./constants";
import { findMessage, search } from "./rows";
import { hasActiveFilters, total } from "./view";

/**
 * How a message reads inside a notice.
 *
 * Only writes the user aimed at one message name it. Marking read fires once
 * per row while the selection walks, so naming the subject there would defeat
 * the deduplication and bury the screen where one sentence does the job; a
 * star or a delete is a single deliberate act, and "which one" is the part
 * worth saying.
 */
export function quoted(id: string): string {
  const subject = findMessage(id)?.subject?.trim();
  if (!subject) return "the message";

  const characters = [...subject];
  if (characters.length <= NOTICE_SUBJECT_MAX) return `“${subject}”`;
  return `“${characters.slice(0, NOTICE_SUBJECT_MAX).join("")}…”`;
}

/** What a whole-inbox delete asks before it runs. */
export function clearInboxPrompt(): ConfirmDialogOptions {
  return {
    title: "Clear all messages",
    message:
      search() || hasActiveFilters()
        ? "Every message in the inbox will be permanently deleted, including those outside the current search and filters."
        : `All ${total()} messages will be permanently deleted.`,
    confirmLabel: "Clear all",
  };
}

/** Deletes every message, saying so when the write does not land. */
export async function clearInbox(): Promise<void> {
  try {
    await api.deleteAllMessages();
  } catch {
    notify("Could not clear the inbox. The messages are still here.");
  }
}

/** Stars or unstars a message, saying so when the write does not land. */
export function starMessage(id: string, starred: boolean): void {
  const label = quoted(id);
  api
    .markStarred(id, starred)
    .catch(() =>
      notify(
        starred ? `Could not star ${label}.` : `Could not unstar ${label}.`,
      ),
    );
}

/** Marks a message read or unread, saying so when the write does not land. */
export function setMessageRead(id: string, read: boolean): void {
  const label = quoted(id);
  api
    .markRead(id, read)
    .catch(() =>
      notify(
        read
          ? `Could not mark ${label} as read.`
          : `Could not mark ${label} as unread.`,
      ),
    );
}
