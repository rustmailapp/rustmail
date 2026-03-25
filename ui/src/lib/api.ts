import type { Attachment, AuthResults, ListResponse, Message } from "./types";

const BASE = "/api/v1";

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  if (!res.ok) {
    throw new Error(`API error: ${res.status} ${res.statusText}`);
  }
  return res.json();
}

async function fetchVoid(url: string, init?: RequestInit): Promise<void> {
  const res = await fetch(url, init);
  if (!res.ok) {
    throw new Error(`API error: ${res.status} ${res.statusText}`);
  }
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

export async function getMessage(id: string): Promise<Message> {
  return fetchJson(`${BASE}/messages/${enc(id)}`);
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
): Promise<Attachment[]> {
  return fetchJson(`${BASE}/messages/${enc(messageId)}/attachments`);
}

export async function getAuthResults(id: string): Promise<AuthResults> {
  return fetchJson(`${BASE}/messages/${enc(id)}/auth`);
}

export async function getRawMessage(id: string): Promise<string> {
  const res = await fetch(`${BASE}/messages/${enc(id)}/raw`);
  if (!res.ok) throw new Error(`API error: ${res.status}`);
  return res.text();
}

export function exportUrl(messageId: string, format: "eml" | "json"): string {
  return `${BASE}/messages/${enc(messageId)}/export?format=${format}`;
}

export function attachmentUrl(messageId: string, attachmentId: string): string {
  return `${BASE}/messages/${enc(messageId)}/attachments/${enc(attachmentId)}`;
}
