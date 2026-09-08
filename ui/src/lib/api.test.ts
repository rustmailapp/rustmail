import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  BULK_REQUEST_TIMEOUT_MS,
  deleteAllMessages,
  getMessage,
  REQUEST_TIMEOUT_MS,
} from "./api";

const fetchMock = vi.fn<typeof fetch>();

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
    fetchMock.mockResolvedValue(Response.json({ id: "msg-0000" }));

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
