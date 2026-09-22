import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ApiError,
  BULK_REQUEST_TIMEOUT_MS,
  deleteAllMessages,
  getMessage,
  listMessages,
  REQUEST_TIMEOUT_MS,
  ResponseShapeError,
} from "./api";

const fetchMock = vi.fn<typeof fetch>();

/** A message body exactly as `docs/api.yaml` describes one. */
function messageBody(
  over: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: "msg-0000",
    sender: "sender@example.test",
    recipients: ["inbox@example.test"],
    subject: "Subject",
    size: 1024,
    has_attachments: false,
    is_read: false,
    is_starred: false,
    tags: [],
    created_at: "2026-01-01T00:00:00Z",
    text_body: "Body",
    html_body: null,
    ...over,
  };
}

/**
 * Installs a backend that answers nothing, and returns the signal it was given.
 *
 * A request nobody answers is the case the deadline exists for: nothing else
 * can end it, so whether it ends at all is a question about the signal.
 */
function stallFetch(): () => AbortSignal | undefined {
  let seen: AbortSignal | undefined;
  fetchMock.mockImplementation((_url, init) => {
    seen = init?.signal ?? undefined;
    return new Promise((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () =>
        reject(init.signal?.reason),
      );
    });
  });
  return () => seen;
}

beforeEach(() => {
  vi.useFakeTimers();
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("request deadline", () => {
  it("cancels a read that never answers", async () => {
    const signal = stallFetch();

    const read = expect(getMessage("msg-0000")).rejects.toMatchObject({
      name: "TimeoutError",
    });
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);

    await read;
    expect(signal()?.aborted).toBe(true);
  });

  it("holds the read open right up to the deadline", async () => {
    stallFetch();
    const settled = vi.fn();

    const read = getMessage("msg-0000");
    read.then(settled, settled);
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS - 1);

    expect(settled).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(settled).toHaveBeenCalled();
  });

  it("stops the deadline once the read answers", async () => {
    fetchMock.mockResolvedValue(Response.json(messageBody()));

    await expect(getMessage("msg-0000")).resolves.toMatchObject({
      id: "msg-0000",
    });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("stops the deadline once the read fails", async () => {
    fetchMock.mockResolvedValue(new Response("", { status: 404 }));

    await expect(getMessage("msg-0000")).rejects.toThrow("API error: 404");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("passes a caller's cancellation on to the request", async () => {
    const signal = stallFetch();
    const caller = new AbortController();

    const read = expect(
      getMessage("msg-0000", caller.signal),
    ).rejects.toMatchObject({ name: "AbortError" });
    caller.abort(new DOMException("superseded", "AbortError"));

    await read;
    expect(signal()?.aborted).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("passes a caller's cancellation on to a list read", async () => {
    const signal = stallFetch();
    const caller = new AbortController();

    const read = expect(
      listMessages({ limit: 100, q: "invoice" }, caller.signal),
    ).rejects.toMatchObject({ name: "AbortError" });
    caller.abort(new DOMException("superseded", "AbortError"));

    await read;
    expect(signal()?.aborted).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("gives a whole-inbox delete a longer budget than a read", async () => {
    const signal = stallFetch();

    const write = expect(deleteAllMessages()).rejects.toMatchObject({
      name: "TimeoutError",
    });
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);
    expect(signal()?.aborted).toBe(false);

    await vi.advanceTimersByTimeAsync(
      BULK_REQUEST_TIMEOUT_MS - REQUEST_TIMEOUT_MS,
    );

    await write;
    expect(signal()?.aborted).toBe(true);
  });

  it("never opens a request the caller had already given up on", async () => {
    stallFetch();
    const caller = new AbortController();
    caller.abort(new DOMException("superseded", "AbortError"));

    await expect(getMessage("msg-0000", caller.signal)).rejects.toMatchObject({
      name: "AbortError",
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("response shape", () => {
  it("accepts the shape the API documents", async () => {
    fetchMock.mockResolvedValue(Response.json(messageBody()));

    await expect(getMessage("msg-0000")).resolves.toEqual(messageBody());
  });

  it("keeps a field the UI does not know about out of the domain model", async () => {
    fetchMock.mockResolvedValue(
      Response.json(messageBody({ delivered_at: "2026-01-01T00:00:01Z" })),
    );

    await expect(getMessage("msg-0000")).resolves.toEqual(messageBody());
  });

  it("rejects a response missing a field, naming the route", async () => {
    const { created_at: _dropped, ...withoutDate } = messageBody();
    fetchMock.mockImplementation(() =>
      Promise.resolve(Response.json(withoutDate)),
    );

    await expect(getMessage("msg-0000")).rejects.toThrow(ResponseShapeError);
    await expect(getMessage("msg-0000")).rejects.toThrow(
      "GET /messages/{id} returned an unexpected shape: created_at should be string",
    );
  });

  it("names the row a list response drifted in", async () => {
    fetchMock.mockResolvedValue(
      Response.json({
        messages: [messageBody({ size: "1024" })],
        total: 1,
        limit: 1,
        next_cursor: null,
      }),
    );

    await expect(listMessages({ limit: 1 })).rejects.toThrow(
      "GET /messages returned an unexpected shape: messages[0].size should be number",
    );
  });

  it("keeps the response body out of the error it raises", async () => {
    fetchMock.mockResolvedValue(
      Response.json(messageBody({ size: "confidential-subject-line" })),
    );

    await expect(getMessage("msg-0000")).rejects.toThrow(
      expect.not.stringContaining("confidential-subject-line"),
    );
  });

  it("rejects a body that is not JSON at all", async () => {
    fetchMock.mockResolvedValue(new Response("<html>502</html>"));

    await expect(getMessage("msg-0000")).rejects.toThrow(
      "GET /messages/{id} returned an unexpected shape: the body is not JSON",
    );
  });

  it("reports a cancelled body read as the cancellation it was", async () => {
    const res = new Response("");
    let failBody: (reason: unknown) => void = () => {};
    vi.spyOn(res, "json").mockReturnValue(
      new Promise((_resolve, reject) => {
        failBody = reject;
      }),
    );
    fetchMock.mockResolvedValue(res);

    const read = expect(getMessage("msg-0000")).rejects.toMatchObject({
      name: "TimeoutError",
    });
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);
    failBody(new DOMException("body read aborted", "AbortError"));

    await read;
  });

  it("lets a body stream that broke stay the transport error it is", async () => {
    const res = new Response("");
    const broken = new TypeError("network error");
    vi.spyOn(res, "json").mockRejectedValue(broken);
    fetchMock.mockResolvedValue(res);

    await expect(getMessage("msg-0000")).rejects.toBe(broken);
  });
});

describe("list reads", () => {
  function requested(): URL {
    const [input] = fetchMock.mock.calls[0] ?? [];
    return new URL(String(input), "http://inbox.test");
  }

  function listBody(): Record<string, unknown> {
    return {
      messages: [messageBody()],
      total: 1,
      limit: 100,
      next_cursor: "msg-0000",
    };
  }

  it("asks for the page older than a cursor, and no offset", async () => {
    fetchMock.mockResolvedValue(Response.json(listBody()));

    await listMessages({ limit: 100, before: "msg-0099" });

    expect(requested().searchParams.get("before")).toBe("msg-0099");
    expect(requested().searchParams.has("offset")).toBe(false);
  });

  it("sends each active filter, and every selected tag", async () => {
    fetchMock.mockResolvedValue(Response.json(listBody()));

    await listMessages({
      limit: 100,
      filters: {
        starred: true,
        unread: true,
        attachments: true,
        tags: ["alpha", "beta"],
      },
    });

    const params = requested().searchParams;
    expect(params.get("starred")).toBe("true");
    expect(params.get("unread")).toBe("true");
    expect(params.get("has_attachments")).toBe("true");
    expect(params.getAll("tag")).toEqual(["alpha", "beta"]);
  });

  it("leaves inactive filters out of the request", async () => {
    fetchMock.mockResolvedValue(Response.json(listBody()));

    await listMessages({
      limit: 100,
      filters: { starred: false, unread: false, attachments: false, tags: [] },
    });

    expect([...requested().searchParams.keys()]).toEqual(["limit"]);
  });

  it("returns the cursor the server hands back", async () => {
    fetchMock.mockResolvedValue(Response.json(listBody()));

    await expect(listMessages({ limit: 100 })).resolves.toMatchObject({
      next_cursor: "msg-0000",
    });
  });

  it("carries the status of a request the server rejected", async () => {
    fetchMock.mockResolvedValue(
      Response.json({ error: "unknown cursor" }, { status: 400 }),
    );

    const read = listMessages({ limit: 100, before: "msg-gone" });

    await expect(read).rejects.toBeInstanceOf(ApiError);
    await expect(read).rejects.toMatchObject({ status: 400 });
  });

  it("carries the code a rejection names", async () => {
    fetchMock.mockResolvedValue(
      Response.json(
        { error: "unknown cursor", code: "unknown_cursor" },
        { status: 400 },
      ),
    );

    await expect(
      listMessages({ limit: 100, before: "msg-gone" }),
    ).rejects.toMatchObject({ status: 400, code: "unknown_cursor" });
  });

  it("carries no code when a rejection names none", async () => {
    fetchMock.mockResolvedValue(
      Response.json(
        { error: "Too many tag filters (max 20)" },
        { status: 400 },
      ),
    );

    await expect(listMessages({ limit: 100 })).rejects.toMatchObject({
      status: 400,
      code: null,
    });
  });

  it("carries no code when a rejection is not JSON", async () => {
    fetchMock.mockResolvedValue(new Response("Bad Gateway", { status: 502 }));

    await expect(listMessages({ limit: 100 })).rejects.toMatchObject({
      status: 502,
      code: null,
    });
  });
});
