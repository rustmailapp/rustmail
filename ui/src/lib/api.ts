import type * as z from "zod/mini";
import * as schema from "./schema";
import type {
  Attachment,
  AuthResults,
  ListResponse,
  Message,
  MessageHeader,
} from "./types";

const BASE = "/api/v1";

/** How long a single request may take, headers and body together. */
export const REQUEST_TIMEOUT_MS = 15_000;

/**
 * How long a whole-inbox delete may take.
 *
 * The row count it walks is unbounded and the store takes one writer at a
 * time, so this write can legitimately outlast a read by a wide margin. Held
 * to a read's deadline it would abort a delete that was going to succeed.
 */
export const BULK_REQUEST_TIMEOUT_MS = 120_000;

/**
 * Runs `request` under a deadline, and under `signal` if the caller gave one.
 *
 * The deadline is a `setTimeout` rather than `AbortSignal.timeout` because the
 * latter runs on a native timer that no clock stub reaches, and a deadline no
 * test can advance to is a deadline no test can check. The body read belongs
 * inside the deadline too: a response whose stream stalls after the headers
 * arrive hangs just as long as one that never answers.
 */
async function withDeadline<T>(
  signal: AbortSignal | null | undefined,
  timeoutMs: number,
  request: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  signal?.throwIfAborted();

  const controller = new AbortController();
  const timer = setTimeout(
    () =>
      controller.abort(
        new DOMException(`Request exceeded ${timeoutMs}ms`, "TimeoutError"),
      ),
    timeoutMs,
  );
  const forward = () => controller.abort(signal?.reason);
  signal?.addEventListener("abort", forward, { once: true });

  try {
    return await request(controller.signal);
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", forward);
  }
}

/**
 * A response whose shape is not the one `docs/api.yaml` promises.
 *
 * Names the route and the field that drifted, never the value that was there:
 * a response body carries message content, and this text reaches the console
 * and, for a write, the screen.
 */
export class ResponseShapeError extends Error {
  constructor(route: string, detail: string, options?: ErrorOptions) {
    super(`${route} returned an unexpected shape: ${detail}`, options);
    this.name = "ResponseShapeError";
  }
}

function fieldPath(path: readonly PropertyKey[]): string {
  return path.reduce<string>((acc, step) => {
    if (typeof step === "number") return `${acc}[${step}]`;
    return acc === "" ? String(step) : `${acc}.${String(step)}`;
  }, "");
}

/**
 * Checks a decoded body against `shape`, or rejects naming `route`.
 *
 * Only the first issue is reported. A drifted response usually breaks the same
 * way in every row it returns, and one field named precisely says more than a
 * hundred repetitions of it.
 */
function parse<S extends z.ZodMiniType>(
  route: string,
  shape: S,
  body: unknown,
): z.infer<S> {
  const result = shape.safeParse(body);
  if (result.success) return result.data;

  const issue = result.error.issues[0];
  if (issue === undefined) {
    throw new ResponseShapeError(route, "the body did not match");
  }
  const where = issue.path.length === 0 ? "the body" : fieldPath(issue.path);
  const detail =
    "expected" in issue
      ? `${where} should be ${issue.expected}`
      : `${where} is not one of the shapes this route returns`;
  throw new ResponseShapeError(route, detail);
}

/**
 * Decodes the body, or reports that the route did not return JSON.
 *
 * Only a parse failure is a drifted response. A body stream that breaks after
 * the headers arrived rejects here too, and calling that a shape mismatch
 * would offer the user a reload for a read a retry would have completed.
 */
async function readJson(
  route: string,
  res: Response,
  signal: AbortSignal,
): Promise<unknown> {
  try {
    return await res.json();
  } catch (cause) {
    signal.throwIfAborted();
    if (cause instanceof SyntaxError) {
      throw new ResponseShapeError(route, "the body is not JSON", { cause });
    }
    throw cause;
  }
}

async function fetchJson<S extends z.ZodMiniType>(
  route: string,
  url: string,
  shape: S,
  init?: RequestInit,
): Promise<z.infer<S>> {
  return withDeadline(init?.signal, REQUEST_TIMEOUT_MS, async (signal) => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) {
      throw new Error(`API error: ${res.status} ${res.statusText}`);
    }
    return parse(route, shape, await readJson(route, res, signal));
  });
}

async function fetchText(url: string, init?: RequestInit): Promise<string> {
  return withDeadline(init?.signal, REQUEST_TIMEOUT_MS, async (signal) => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) {
      throw new Error(`API error: ${res.status} ${res.statusText}`);
    }
    return res.text();
  });
}

async function fetchVoid(
  url: string,
  init?: RequestInit,
  timeoutMs = REQUEST_TIMEOUT_MS,
): Promise<void> {
  await withDeadline(init?.signal, timeoutMs, async (signal) => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) {
      throw new Error(`API error: ${res.status} ${res.statusText}`);
    }
  });
}

export async function listMessages(
  limit = 50,
  offset = 0,
  q?: string,
): Promise<ListResponse> {
  const params = new URLSearchParams({
    limit: String(limit),
    offset: String(offset),
  });
  if (q) params.set("q", q);
  return fetchJson(
    "GET /messages",
    `${BASE}/messages?${params}`,
    schema.listResponse,
  );
}

function enc(s: string): string {
  return encodeURIComponent(s);
}

export async function getMessage(
  id: string,
  signal?: AbortSignal,
): Promise<Message> {
  return fetchJson(
    "GET /messages/{id}",
    `${BASE}/messages/${enc(id)}`,
    schema.message,
    { signal },
  );
}

/**
 * Deletes one message.
 *
 * `keepalive` is unconditional: a DELETE cancelled with the closing document
 * leaves the message on the server, and the request is in flight for the whole
 * round trip, not only at unload. It carries no body, so the size limit that
 * makes `keepalive` awkward on larger writes does not reach this one.
 */
export async function deleteMessage(id: string): Promise<void> {
  await fetchVoid(`${BASE}/messages/${enc(id)}`, {
    method: "DELETE",
    keepalive: true,
  });
}

export async function deleteAllMessages(): Promise<void> {
  await fetchVoid(
    `${BASE}/messages`,
    { method: "DELETE" },
    BULK_REQUEST_TIMEOUT_MS,
  );
}

export async function markRead(id: string, is_read: boolean): Promise<void> {
  await fetchVoid(`${BASE}/messages/${enc(id)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ is_read }),
  });
}

export async function markStarred(
  id: string,
  is_starred: boolean,
): Promise<void> {
  await fetchVoid(`${BASE}/messages/${enc(id)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ is_starred }),
  });
}

export async function setTags(id: string, tags: string[]): Promise<void> {
  await fetchVoid(`${BASE}/messages/${enc(id)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ tags }),
  });
}

export async function listAttachments(
  messageId: string,
  signal?: AbortSignal,
): Promise<Attachment[]> {
  return fetchJson(
    "GET /messages/{id}/attachments",
    `${BASE}/messages/${enc(messageId)}/attachments`,
    schema.attachmentList,
    { signal },
  );
}

export async function getAuthResults(
  id: string,
  signal?: AbortSignal,
): Promise<AuthResults> {
  return fetchJson(
    "GET /messages/{id}/auth",
    `${BASE}/messages/${enc(id)}/auth`,
    schema.authResults,
    { signal },
  );
}

export async function getHeaders(
  id: string,
  signal?: AbortSignal,
): Promise<MessageHeader[]> {
  return fetchJson(
    "GET /messages/{id}/headers",
    `${BASE}/messages/${enc(id)}/headers`,
    schema.headerList,
    { signal },
  );
}

/** Fetches a message's raw source, optionally only its first `limitBytes`. */
export async function getRawMessage(
  id: string,
  limitBytes?: number,
  signal?: AbortSignal,
): Promise<string> {
  const query = limitBytes === undefined ? "" : `?limit=${limitBytes}`;
  return fetchText(`${BASE}/messages/${enc(id)}/raw${query}`, { signal });
}

export function exportUrl(messageId: string, format: "eml" | "json"): string {
  return `${BASE}/messages/${enc(messageId)}/export?format=${format}`;
}

export function attachmentUrl(messageId: string, attachmentId: string): string {
  return `${BASE}/messages/${enc(messageId)}/attachments/${enc(attachmentId)}`;
}
