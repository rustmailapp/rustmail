import type { Page, WebSocketRoute } from "@playwright/test";
import type { Message, MessageSummary, WsEvent } from "../src/lib/types";

/** Messages the fake backend holds — enough to force several pages. */
export const TOTAL_MESSAGES = 600;

const API = /\/api\/v1\//;
const WS = /\/api\/v1\/ws$/;
const EPOCH = Date.UTC(2026, 0, 1);
const MESSAGE_SIZE_BYTES = 2048;
const NO_CONTENT = 204;
const NOT_FOUND = 404;
/** Mirrors the clamp the real handler applies to `limit`. */
const MIN_LIMIT = 1;
const MAX_LIMIT = 200;

function clampLimit(limit: number): number {
  return Math.min(Math.max(limit, MIN_LIMIT), MAX_LIMIT);
}

export function messageId(index: number): string {
  return `msg-${String(index).padStart(4, "0")}`;
}

export function optionSelector(index: number): string {
  return `#msg-option-${messageId(index)}`;
}

function summary(index: number): MessageSummary {
  return {
    id: messageId(index),
    sender: `sender-${index}@example.test`,
    recipients: ["inbox@example.test"],
    subject: `Message ${index}`,
    size: MESSAGE_SIZE_BYTES,
    has_attachments: false,
    is_read: false,
    is_starred: false,
    tags: [],
    created_at: new Date(EPOCH + index * 1000).toISOString(),
  };
}

/** Records the writes the UI attempted, so tests can assert on side effects. */
export interface ApiCalls {
  deleted: string[];
  patched: string[];
  fetched: string[];
}

/** Handle on the fake backend: what the UI wrote, and a way to push events. */
export interface InboxBackend {
  calls: ApiCalls;
  push(event: WsEvent): Promise<void>;
}

const SOCKET_WAIT_MS = 5000;
const SOCKET_POLL_MS = 25;

/**
 * Serves the inbox API from an in-process fixture.
 *
 * The UI's only backend contact is this REST surface plus one WebSocket, so
 * stubbing both keeps the browser, its layout engine and the virtualizer real
 * while making the data deterministic. Must be called before navigating.
 */
export async function mockInbox(
  page: Page,
  total: number = TOTAL_MESSAGES,
): Promise<InboxBackend> {
  const all = Array.from({ length: total }, (_, i) => summary(i));
  const calls: ApiCalls = { deleted: [], patched: [], fetched: [] };
  let socket: WebSocketRoute | undefined;

  await page.routeWebSocket(WS, (ws) => {
    socket = ws;
  });

  await page.route(API, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");

    if (request.method() === "DELETE") {
      calls.deleted.push(path);
      return route.fulfill({ status: NO_CONTENT, body: "" });
    }
    if (request.method() === "PATCH") {
      calls.patched.push(path);
      return route.fulfill({ status: NO_CONTENT, body: "" });
    }
    if (path === "/messages") {
      const limit = clampLimit(Number(url.searchParams.get("limit")));
      const offset = Number(url.searchParams.get("offset"));
      return route.fulfill({
        json: {
          messages: all.slice(offset, offset + limit),
          total: all.length,
        },
      });
    }
    if (/^\/messages\/[^/]+\/attachments$/.test(path)) {
      return route.fulfill({ json: [] });
    }
    const single = /^\/messages\/([^/]+)$/.exec(path);
    if (single) {
      calls.fetched.push(path);
      const found = all.find((m) => m.id === single[1]);
      if (!found) return route.fulfill({ status: NOT_FOUND, json: {} });
      const message: Message = {
        ...found,
        text_body: `Body of ${found.subject}`,
        html_body: null,
      };
      return route.fulfill({ json: message });
    }
    return route.fulfill({ status: NOT_FOUND, json: {} });
  });

  return {
    calls,
    async push(event: WsEvent) {
      const deadline = Date.now() + SOCKET_WAIT_MS;
      while (!socket && Date.now() < deadline) {
        await page.waitForTimeout(SOCKET_POLL_MS);
      }
      if (!socket) throw new Error("the page never opened its WebSocket");
      socket.send(JSON.stringify(event));
    },
  };
}
