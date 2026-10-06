/** How long a copy button says it copied before it goes back to "Copy". */
export const COPIED_FEEDBACK_MS = 1500;
/** How long the hint to copy by hand stays up when the clipboard is out of reach. */
export const MANUAL_COPY_HINT_MS = 4000;
/** What a copy button says when the text has to be copied by hand. */
export const MANUAL_COPY_HINT = "Press Cmd/Ctrl+C to copy";

/** What a copy button is showing after a press: done, or copy it yourself. */
export type CopyFeedback = "copied" | "manual";

/**
 * Writes `text` to the clipboard, reporting whether it landed.
 *
 * The async clipboard exists only in secure contexts, so a portal opened over
 * plain http from another machine on the LAN has none.
 */
export async function writeClipboard(text: string): Promise<boolean> {
  if (!navigator.clipboard) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
