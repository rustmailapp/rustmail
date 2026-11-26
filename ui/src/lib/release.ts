import { formatDateTime } from "./format";
import type { ReleaseRecord } from "./types";

/** A message's recorded release, both fields `null` while it was never released. */
export interface RecordedRelease {
  released_at: string | null;
  release_outcome: ReleaseRecord["release_outcome"] | null;
}

/** The release `message` records, or `null` while it was never released. */
export function recordedRelease(
  message: RecordedRelease,
): ReleaseRecord | null {
  if (message.released_at === null || message.release_outcome === null) {
    return null;
  }
  return {
    released_at: message.released_at,
    release_outcome: message.release_outcome,
  };
}

/**
 * Which of two records of the same message's release is the current one.
 *
 * The later release wins; at the same instant a delivery wins over an unknown
 * outcome, as the server never downgrades one. Deciding by the records rather
 * than by arrival order keeps a replayed or repeated record from undoing a
 * newer one.
 */
export function newerRelease(
  current: ReleaseRecord | null,
  incoming: ReleaseRecord,
): ReleaseRecord {
  if (current === null) return incoming;
  const currentAt = Date.parse(current.released_at);
  const incomingAt = Date.parse(incoming.released_at);
  if (currentAt > incomingAt) return current;
  if (currentAt < incomingAt) return incoming;
  return current.release_outcome === "sent" ? current : incoming;
}

function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** When a release happened: the local time if it was today, else date and time. */
export function releaseTime(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  if (!sameLocalDay(at, now)) return formatDateTime(iso);
  return at.toLocaleTimeString(undefined, { timeStyle: "short" });
}

/** What the release confirmation warns about a message released before. */
export function releaseWarning(
  record: ReleaseRecord,
  now: Date = new Date(),
): string {
  const at = releaseTime(record.released_at, now);
  return record.release_outcome === "sent"
    ? `Already released at ${at}`
    : `A release at ${at} may have been delivered`;
}

/** The recorded release as the details rail lists it. */
export function releaseSummary(
  record: ReleaseRecord,
  now: Date = new Date(),
): string {
  const outcome =
    record.release_outcome === "sent" ? "sent" : "outcome unknown";
  return `${releaseTime(record.released_at, now)}, ${outcome}`;
}
