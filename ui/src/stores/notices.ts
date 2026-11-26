import { createSignal } from "solid-js";

/**
 * How long a notice stays up on its own.
 *
 * Long enough to read a sentence after looking away from the keystroke that
 * caused it, short enough that it is gone before the next one matters.
 */
const NOTICE_TTL_MS = 6000;

/** The most notices on screen at once; a new one pushes the oldest off. */
const MAX_NOTICES = 3;

/** Whether a notice reports something that failed or something that worked. */
export type NoticeTone = "error" | "success";

export interface Notice {
  id: number;
  text: string;
  tone: NoticeTone;
}

const [notices, setNotices] = createSignal<readonly Notice[]>([]);
const dwells = new Map<number, ReturnType<typeof setTimeout>>();
let nextId = 0;

function clearDwell(id: number): void {
  const dwell = dwells.get(id);
  if (dwell === undefined) return;
  clearTimeout(dwell);
  dwells.delete(id);
}

/** Takes a notice off screen, whether it timed out or was dismissed. */
function dismissNotice(id: number): void {
  clearDwell(id);
  setNotices((current) => current.filter((notice) => notice.id !== id));
}

function startDwell(id: number): void {
  clearDwell(id);
  dwells.set(
    id,
    setTimeout(() => dismissNotice(id), NOTICE_TTL_MS),
  );
}

/**
 * Puts a notice on screen, a failure unless `tone` says otherwise.
 *
 * A repeat of a notice already up restarts its dwell instead of stacking:
 * walking the inbox with the server down writes one failed read per row, and
 * the same sentence three deep says nothing the first one did not. A
 * `persistent` notice has no dwell and stays until it is dismissed or pushed
 * off, for a warning the user must not miss by looking away.
 */
function notify(
  text: string,
  {
    tone = "error",
    persistent = false,
  }: { tone?: NoticeTone; persistent?: boolean } = {},
): void {
  const showing = notices().find((notice) => notice.text === text);
  if (showing !== undefined) {
    if (!persistent) startDwell(showing.id);
    return;
  }

  const notice: Notice = { id: nextId++, text, tone };
  const current = notices();
  const overflow = Math.max(0, current.length + 1 - MAX_NOTICES);
  for (const dropped of current.slice(0, overflow)) clearDwell(dropped.id);

  setNotices([...current.slice(overflow), notice]);
  if (!persistent) startDwell(notice.id);
}

export { notices, notify, dismissNotice, NOTICE_TTL_MS, MAX_NOTICES };
