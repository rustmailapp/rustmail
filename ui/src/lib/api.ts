import type * as z from "zod/mini";
import * as schema from "./schema";
import type {
  Attachment,
  AuthResults,
  FilterState,
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
 * A response whose status says the request did not succeed.
 *
 * Carries the status so a caller can tell a request the server rejected from
 * one that never got an answer, and the body's `code`, when it names one, so a
 * caller can tell one rejection from another without parsing the message.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string | null;

  constructor(res: Response, code: string | null = null) {
    super(`API error: ${res.status} ${res.statusText}`);
    this.name = "ApiError";
    this.status = res.status;
    this.code = code;
  }
}

/**
 * The error a rejected response stands for, with the `code` its body names.
 *
 * A body that is not JSON, or names no code, is still a rejection: only the
 * code is missing, so that is the only thing left out.
 */
async function rejection(
  res: Response,
  signal: AbortSignal,
): Promise<ApiError> {
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    signal.throwIfAborted();
    return new ApiError(res);
  }
  const parsed = schema.errorCode.safeParse(body);
  return new ApiError(res, parsed.success ? parsed.data.code : null);
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
    if (!res.ok) throw await rejection(res, signal);
    return parse(route, shape, await readJson(route, res, signal));
  });
}

async function fetchText(url: string, init?: RequestInit): Promise<string> {
  return withDeadline(init?.signal, REQUEST_TIMEOUT_MS, async (signal) => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) throw await rejection(res, signal);
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
    if (!res.ok) throw await rejection(res, signal);
  });
}

/** What a list read asks `GET /messages` for. */
export interface ListQuery {
  limit: number;
  /** Full-text search; an empty string reads the whole inbox. */
  q?: string;
  /** The `next_cursor` of the page before: read the messages older than it. */
  before?: string;
  /** Narrows the read, and its `total`, to the messages these match. */
  filters?: FilterState;
}

function setFilterParams(params: URLSearchParams, filters: FilterState): void {
  if (filters.starred) params.set("starred", "true");
  if (filters.unread) params.set("unread", "true");
  if (filters.attachments) params.set("has_attachments", "true");
  for (const tag of filters.tags) params.append("tag", tag);
}

export async function listMessages(
  query: ListQuery,
  signal?: AbortSignal,
): Promise<ListResponse> {
  const params = new URLSearchParams({ limit: String(query.limit) });
  if (query.q) params.set("q", query.q);
  if (query.before !== undefined) params.set("before", query.before);
  if (query.filters) setFilterParams(params, query.filters);
  return fetchJson(
    "GET /messages",
    `${BASE}/messages?${params}`,
    schema.listResponse,
    { signal },
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
