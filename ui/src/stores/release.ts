import { createSignal } from "solid-js";
import * as api from "../lib/api";
import type { ReleaseTarget } from "../lib/types";
import { notify } from "./notices";

const BAD_REQUEST = 400;
const FORBIDDEN = 403;
const NOT_FOUND = 404;
const BAD_GATEWAY = 502;
const DEFINITE_FAILURES: ReadonlySet<number> = new Set([
  BAD_REQUEST,
  FORBIDDEN,
  NOT_FOUND,
  BAD_GATEWAY,
]);

const [inFlight, setInFlight] = createSignal<ReadonlySet<string>>(new Set());

/** Whether a release of message `id` is waiting on the server. */
export function releasing(id: string): boolean {
  return inFlight().has(id);
}

/** The relay as the user reads it: `host:port`. */
export function relayLabel(target: ReleaseTarget): string {
  return `${target.host}:${target.port}`;
}

/**
 * Whether a failed release may still have reached the relay.
 *
 * Only a refusal the server answers before it contacts the relay, or a relay
 * that refused outright, is a plain failure. A deadline, a dropped
 * connection or any other status leave the relay possibly holding the
 * message, and a retry would then send it twice.
 */
export function outcomeUnknown(error: unknown): boolean {
  if (!(error instanceof api.ApiError)) return true;
  if (error.code === api.RELEASE_OUTCOME_UNKNOWN_CODE) return true;
  return !DEFINITE_FAILURES.has(error.status);
}

/** What a failed release means for the user. */
export function releaseFailure(error: unknown, relay: string): string {
  if (!(error instanceof api.ApiError) || outcomeUnknown(error)) {
    return `The release may have been delivered: no clear answer came back. Check ${relay} before releasing it again.`;
  }
  switch (error.status) {
    case NOT_FOUND:
      return "Could not release the message: it is no longer on the server.";
    case BAD_GATEWAY:
      return `Release failed: ${relay} could not be reached or refused the message. Nothing was delivered.`;
    default:
      return "The server refused the release. Reload and check the relay settings. Nothing was sent.";
  }
}

function recipientCount(count: number): string {
  return count === 1 ? "1 recipient" : `${count} recipients`;
}

/**
 * Sends message `id` to the relay and reports the outcome as a notice.
 *
 * A second call for a message whose release is still in flight does nothing,
 * so a repeated click cannot send it twice.
 */
export async function release(
  id: string,
  target: ReleaseTarget,
  recipients: number,
): Promise<void> {
  if (releasing(id)) return;
  setInFlight((current) => new Set(current).add(id));
  const relay = relayLabel(target);
  try {
    await api.releaseMessage(id, target);
    notify(`Released to ${recipientCount(recipients)} through ${relay}.`, {
      tone: "success",
    });
  } catch (error) {
    notify(releaseFailure(error, relay), {
      persistent: outcomeUnknown(error),
    });
  } finally {
    setInFlight((current) => {
      const next = new Set(current);
      next.delete(id);
      return next;
    });
  }
}
