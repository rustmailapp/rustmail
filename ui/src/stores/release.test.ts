import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissNotice, notices } from "./notices";

const { releaseMessage, recordRelease } = vi.hoisted(() => ({
  releaseMessage: vi.fn(),
  recordRelease: vi.fn(),
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ApiError: actual.ApiError,
    RELEASE_OUTCOME_UNKNOWN_CODE: actual.RELEASE_OUTCOME_UNKNOWN_CODE,
    ReleaseError: actual.ReleaseError,
    releaseMessage,
  };
});

vi.mock("./messages", () => ({ recordRelease }));

const { ApiError, ReleaseError, RELEASE_OUTCOME_UNKNOWN_CODE } =
  await import("../lib/api");
const { release, releaseFailure, releasing } = await import("./release");
const { NOTICE_TTL_MS } = await import("./notices");

const TARGET = { host: "smtp.example.test", port: 465 };
const RELAY = "smtp.example.test:465";
const OUTCOME_UNKNOWN =
  /may have been delivered.*Check smtp\.example\.test:465/;

function rejection(status: number, code: string | null = null) {
  return new ApiError(new Response("", { status }), code);
}

function texts(): string[] {
  return notices().map((notice) => notice.text);
}

const SENT = {
  released_at: "2026-11-26T13:05:09Z",
  release_outcome: "sent",
} as const;

beforeEach(() => {
  releaseMessage.mockReset();
  recordRelease.mockReset();
  for (const notice of notices()) dismissNotice(notice.id);
});

afterEach(() => {
  for (const notice of notices()) dismissNotice(notice.id);
});

describe("release", () => {
  it("sends the confirmed relay and reports how many recipients it reached", async () => {
    releaseMessage.mockResolvedValue(undefined);

    await release("msg-1", TARGET, 3);

    expect(releaseMessage).toHaveBeenCalledWith("msg-1", TARGET);
    expect(notices()).toMatchObject([
      { text: `Released to 3 recipients through ${RELAY}.`, tone: "success" },
    ]);
  });

  it("records the release the server answered with at once", async () => {
    releaseMessage.mockResolvedValue(SENT);

    await release("msg-1", TARGET, 1);

    expect(recordRelease).toHaveBeenCalledExactlyOnceWith("msg-1", SENT);
  });

  it("records nothing when the server answers without a record", async () => {
    releaseMessage.mockResolvedValue(null);

    await release("msg-1", TARGET, 1);

    expect(recordRelease).not.toHaveBeenCalled();
    expect(notices()[0]?.tone).toBe("success");
  });

  it("records the release an unknown outcome left on the message", async () => {
    const unknown = { ...SENT, release_outcome: "unknown" } as const;
    releaseMessage.mockRejectedValue(
      new ReleaseError(
        new Response("", { status: 504 }),
        RELEASE_OUTCOME_UNKNOWN_CODE,
        unknown,
      ),
    );

    await release("msg-1", TARGET, 1);

    expect(recordRelease).toHaveBeenCalledExactlyOnceWith("msg-1", unknown);
    expect(texts()).toEqual([expect.stringMatching(OUTCOME_UNKNOWN)]);
  });

  it("records nothing when the relay refused the message", async () => {
    releaseMessage.mockRejectedValue(
      new ReleaseError(new Response("", { status: 502 }), null, null),
    );

    await release("msg-1", TARGET, 1);

    expect(recordRelease).not.toHaveBeenCalled();
  });

  it("is in flight until the server answers, and only then", async () => {
    let answer!: () => void;
    releaseMessage.mockReturnValue(
      new Promise<void>((resolve) => (answer = resolve)),
    );

    const sending = release("msg-1", TARGET, 1);
    expect(releasing("msg-1")).toBe(true);
    expect(releasing("msg-2")).toBe(false);

    answer();
    await sending;
    expect(releasing("msg-1")).toBe(false);
  });

  it("never sends a message twice while its release is in flight", async () => {
    let answer!: () => void;
    releaseMessage.mockReturnValue(
      new Promise<void>((resolve) => (answer = resolve)),
    );

    const first = release("msg-1", TARGET, 1);
    await release("msg-1", TARGET, 1);
    answer();
    await first;

    expect(releaseMessage).toHaveBeenCalledTimes(1);
  });

  it("says the outcome is unknown when the browser stops waiting", async () => {
    releaseMessage.mockRejectedValue(
      new DOMException("Request exceeded 45000ms", "TimeoutError"),
    );

    await release("msg-1", TARGET, 1);

    expect(texts()).toEqual([expect.stringMatching(OUTCOME_UNKNOWN)]);
    expect(notices()[0]?.tone).toBe("error");
    expect(releasing("msg-1")).toBe(false);
  });

  it("keeps an unknown outcome on screen until it is dismissed", async () => {
    vi.useFakeTimers();
    releaseMessage.mockRejectedValue(new TypeError("Failed to fetch"));

    await release("msg-1", TARGET, 1);
    await vi.advanceTimersByTimeAsync(NOTICE_TTL_MS * 2);

    expect(texts()).toEqual([expect.stringMatching(OUTCOME_UNKNOWN)]);
    vi.useRealTimers();
  });

  it("lets a plain failure go on its own", async () => {
    vi.useFakeTimers();
    releaseMessage.mockRejectedValue(rejection(502));

    await release("msg-1", TARGET, 1);
    await vi.advanceTimersByTimeAsync(NOTICE_TTL_MS);

    expect(texts()).toEqual([]);
    vi.useRealTimers();
  });
});

describe("releaseFailure", () => {
  it("calls a server-side relay timeout an unknown outcome", () => {
    expect(
      releaseFailure(rejection(504, RELEASE_OUTCOME_UNKNOWN_CODE), RELAY),
    ).toMatch(OUTCOME_UNKNOWN);
  });

  it("calls a dropped connection an unknown outcome", () => {
    expect(releaseFailure(new TypeError("Failed to fetch"), RELAY)).toMatch(
      OUTCOME_UNKNOWN,
    );
  });

  it("names the relay when it could not be reached or refused", () => {
    expect(releaseFailure(rejection(502), RELAY)).toBe(
      `Release failed: ${RELAY} could not be reached or refused the message. Nothing was delivered.`,
    );
  });

  it("calls any other server error an unknown outcome", () => {
    expect(releaseFailure(rejection(500), RELAY)).toMatch(OUTCOME_UNKNOWN);
    expect(releaseFailure(rejection(503), RELAY)).toMatch(OUTCOME_UNKNOWN);
  });

  it("says nothing was sent when the server refused before sending", () => {
    expect(releaseFailure(rejection(400), RELAY)).toMatch(/Nothing was sent/);
    expect(releaseFailure(rejection(403), RELAY)).toMatch(/Nothing was sent/);
  });
});
