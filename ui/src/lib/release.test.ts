import { describe, expect, it } from "vitest";
import { formatDateTime } from "./format";
import {
  newerRelease,
  recordedRelease,
  releaseSummary,
  releaseTime,
  releaseWarning,
} from "./release";

const RELEASED_AT = new Date(2026, 10, 26, 13, 5, 9);
const SAME_DAY = new Date(2026, 10, 26, 23, 59);
const NEXT_DAY = new Date(2026, 10, 27, 0, 1);

function localTime(at: Date): string {
  return at.toLocaleTimeString(undefined, { timeStyle: "short" });
}

const sent = {
  released_at: RELEASED_AT.toISOString(),
  release_outcome: "sent",
} as const;
const unknown = { ...sent, release_outcome: "unknown" } as const;

describe("recordedRelease", () => {
  it("is nothing while the message was never released", () => {
    expect(
      recordedRelease({ released_at: null, release_outcome: null }),
    ).toBeNull();
  });

  it("is the release both fields record", () => {
    expect(recordedRelease(sent)).toEqual(sent);
  });
});

describe("newerRelease", () => {
  const later = {
    released_at: new Date(RELEASED_AT.getTime() + 60_000).toISOString(),
    release_outcome: "unknown",
  } as const;

  it("takes the first record of a message", () => {
    expect(newerRelease(null, unknown)).toEqual(unknown);
  });

  it("takes the later of two releases, whatever their outcomes", () => {
    expect(newerRelease(sent, later)).toEqual(later);
    expect(newerRelease(later, sent)).toEqual(later);
  });

  it("keeps a delivery over an unknown outcome at the same instant", () => {
    expect(newerRelease(sent, unknown)).toEqual(sent);
    expect(newerRelease(unknown, sent)).toEqual(sent);
  });

  it("compares instants, not how the server spelled them", () => {
    const fractional = {
      released_at: "2026-11-26T13:05:09.000Z",
      release_outcome: "unknown",
    } as const;
    const whole = {
      released_at: "2026-11-26T13:05:09Z",
      release_outcome: "sent",
    } as const;

    expect(newerRelease(whole, fractional)).toEqual(whole);
  });
});

describe("releaseTime", () => {
  it("is the local time alone for a release earlier today", () => {
    expect(releaseTime(sent.released_at, SAME_DAY)).toBe(
      localTime(RELEASED_AT),
    );
  });

  it("names the date for a release on another day", () => {
    expect(releaseTime(sent.released_at, NEXT_DAY)).toBe(
      formatDateTime(sent.released_at),
    );
  });
});

describe("releaseWarning", () => {
  it("says a delivered release happened, at the time it did today", () => {
    expect(releaseWarning(sent, SAME_DAY)).toBe(
      `Already released at ${localTime(RELEASED_AT)}`,
    );
  });

  it("says an unknown outcome may have been delivered", () => {
    expect(releaseWarning(unknown, SAME_DAY)).toBe(
      `A release at ${localTime(RELEASED_AT)} may have been delivered`,
    );
  });

  it("names the date of a release on another day", () => {
    expect(releaseWarning(sent, NEXT_DAY)).toBe(
      `Already released at ${formatDateTime(sent.released_at)}`,
    );
  });
});

describe("releaseSummary", () => {
  it("lists the time and the outcome", () => {
    const at = localTime(RELEASED_AT);

    expect(releaseSummary(sent, SAME_DAY)).toBe(`${at}, sent`);
    expect(releaseSummary(unknown, SAME_DAY)).toBe(`${at}, outcome unknown`);
  });
});
