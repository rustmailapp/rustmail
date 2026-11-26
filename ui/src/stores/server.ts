import { createResource, createRoot } from "solid-js";
import * as api from "../lib/api";
import { smtpHost } from "../lib/smtp";
import type { ReleaseTarget } from "../lib/types";

/**
 * The server's own details, read once.
 *
 * A failed read is not retried: nothing on the page depends on it, and the
 * reload that would fix a stale server fixes this too.
 */
const [serverInfo] = createRoot(() => createResource(() => api.getInfo()));

/** Where to send mail, as reached from this browser, once the port is known. */
function smtpEndpoint(): { host: string; port: number } | undefined {
  if (serverInfo.state !== "ready") return undefined;
  return { host: smtpHost(location.hostname), port: serverInfo().smtp_port };
}

/** The relay a release goes through, or `null` while it is off or unknown. */
function releaseTarget(): ReleaseTarget | null {
  return serverInfo.state === "ready" ? serverInfo().release : null;
}

export { serverInfo, smtpEndpoint, releaseTarget };
