import { clearInbox, clearInboxPrompt } from "../stores/messages";
import { confirm } from "./ConfirmDialog";

/**
 * Asks before deleting every message, then deletes them if confirmed.
 *
 * Kept out of the messages store so its tests never load the dialog.
 */
export async function askToClearInbox(): Promise<void> {
  if (await confirm(clearInboxPrompt())) await clearInbox();
}
