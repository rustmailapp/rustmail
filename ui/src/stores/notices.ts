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

export interface Notice {
  id: number;
  text: string;
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
 * Puts a failure on screen.
 *
 * A repeat of a notice already up restarts its dwell instead of stacking:
 * walking the inbox with the server down writes one failed read per row, and
 * the same sentence three deep says nothing the first one did not.
 */
function notify(text: string): void {
  const showing = notices().find((notice) => notice.text === text);
  if (showing !== undefined) {
    startDwell(showing.id);
    return;
  }

  const notice: Notice = { id: nextId++, text };
  const current = notices();
  const overflow = Math.max(0, current.length + 1 - MAX_NOTICES);
  for (const dropped of current.slice(0, overflow)) clearDwell(dropped.id);

  setNotices([...current.slice(overflow), notice]);
  startDwell(notice.id);
}

export { notices, notify, dismissNotice, NOTICE_TTL_MS, MAX_NOTICES };
