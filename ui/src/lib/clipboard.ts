import { createSignal, onCleanup, type Accessor } from "solid-js";

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

/**
 * A copy action and what it is showing after a press.
 *
 * Says "copied" for a moment when the write lands. Without a clipboard it
 * shows the manual hint, then selects `source`, so a press of Cmd/Ctrl+C
 * finishes the job; `source` may be an element only the hint renders. Call
 * it while a component is set up.
 */
export function createCopy(
  text: () => string,
  source: () => HTMLElement | undefined,
): { feedback: Accessor<CopyFeedback | null>; copy: () => Promise<void> } {
  const [feedback, setFeedback] = createSignal<CopyFeedback | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  function show(kind: CopyFeedback, durationMs: number): void {
    setFeedback(kind);
    clearTimeout(timer);
    timer = setTimeout(() => setFeedback(null), durationMs);
  }

  async function copy(): Promise<void> {
    if (await writeClipboard(text())) {
      show("copied", COPIED_FEEDBACK_MS);
      return;
    }
    show("manual", MANUAL_COPY_HINT_MS);
    const element = source();
    if (element) window.getSelection()?.selectAllChildren(element);
  }

  return { feedback, copy };
}
