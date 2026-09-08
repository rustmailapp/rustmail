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
  request: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  signal?.throwIfAborted();

  const controller = new AbortController();
  const timer = setTimeout(
    () =>
      controller.abort(
        new DOMException(
          `Request exceeded ${REQUEST_TIMEOUT_MS}ms`,
          "TimeoutError",
        ),
      ),
    REQUEST_TIMEOUT_MS,
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

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  return withDeadline(init?.signal, async (signal): Promise<T> => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) {
      throw new Error(`API error: ${res.status} ${res.statusText}`);
    }
    return res.json();
  });
}

async function fetchText(url: string, init?: RequestInit): Promise<string> {
  return withDeadline(init?.signal, async (signal) => {
    const res = await fetch(url, { ...init, signal });
    if (!res.ok) {
      throw new Error(`API error: ${res.status} ${res.statusText}`);
    }
    return res.text();
  });
}

async function fetchVoid(url: string, init?: RequestInit): Promise<void> {
  await withDeadline(init?.signal, async (signal) => {
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
  return fetchJson(`${BASE}/messages?${params}`);
}

function enc(s: string): string {
  return encodeURIComponent(s);
}

export async function getMessage(
  id: string,
  signal?: AbortSignal,
): Promise<Message> {
  return fetchJson(`${BASE}/messages/${enc(id)}`, { signal });
}

export async function deleteMessage(id: string): Promise<void> {
  await fetchVoid(`${BASE}/messages/${enc(id)}`, { method: "DELETE" });
}

export async function deleteAllMessages(): Promise<void> {
  await fetchVoid(`${BASE}/messages`, { method: "DELETE" });
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
  return fetchJson(`${BASE}/messages/${enc(messageId)}/attachments`, {
    signal,
  });
}

export async function getAuthResults(
  id: string,
  signal?: AbortSignal,
): Promise<AuthResults> {
  return fetchJson(`${BASE}/messages/${enc(id)}/auth`, { signal });
}

export async function getHeaders(
  id: string,
  signal?: AbortSignal,
): Promise<MessageHeader[]> {
  return fetchJson(`${BASE}/messages/${enc(id)}/headers`, { signal });
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
