import { createResource, createRoot, type Resource } from "solid-js";
import * as api from "../lib/api";
import type { ReleaseTarget, ServerInfo } from "../lib/types";

let info: Resource<ServerInfo> | undefined;

/**
 * What the server reports about itself, read once per page and shared.
 *
 * The read starts on first use rather than at import. A failed read is not
 * retried: everything it reports is fixed for the life of the server process,
 * and the reload that would pick up a restarted server retries it too.
 */
export function serverInfo(): Resource<ServerInfo> {
  info ??= createRoot(() => createResource(() => api.getInfo())[0]);
  return info;
}

/** The relay a release goes through, or `null` while it is off or unknown. */
export function releaseTarget(): ReleaseTarget | null {
  const current = serverInfo();
  return current.state === "ready" ? current().release : null;
}
