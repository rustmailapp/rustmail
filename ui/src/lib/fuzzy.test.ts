import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("matches a subsequence regardless of case", () => {
    expect(fuzzyScore("tdk", "Theme: Dark")).not.toBeNull();
    expect(fuzzyScore("THEME", "theme: dark")).not.toBeNull();
  });

  it("rejects text missing a character of the query", () => {
    expect(fuzzyScore("xyz", "Theme: Dark")).toBeNull();
  });

  it("rejects characters that only appear out of order", () => {
    expect(fuzzyScore("kd", "Dark")).toBeNull();
  });

  it("needs every word of the query, in any order", () => {
    expect(fuzzyScore("dark theme", "Theme: Dark")).not.toBeNull();
    expect(fuzzyScore("dark palette", "Theme: Dark")).toBeNull();
  });

  it("scores consecutive characters above scattered ones", () => {
    const consecutive = fuzzyScore("star", "Star message");
    const scattered = fuzzyScore("star", "Show starred, then archive");
    const spread = fuzzyScore("star", "Set the alarm");
    expect(consecutive).toBeGreaterThan(spread ?? Infinity);
    expect(scattered).toBeGreaterThan(spread ?? Infinity);
  });

  it("scores word starts above the middle of a word", () => {
    expect(fuzzyScore("d", "Delete")).toBeGreaterThan(
      fuzzyScore("d", "Hide") ?? Infinity,
    );
  });

  it("treats a blank query as a match", () => {
    expect(fuzzyScore("  ", "anything")).toBe(0);
  });
});

describe("fuzzyFilter", () => {
  const labels = ["Mark as unread", "Filter by unread", "Undo delete"];

  it("keeps the order as given for a blank query", () => {
    expect(fuzzyFilter(labels, "", (l) => l)).toEqual(labels);
  });

  it("drops the items that do not match and ranks the rest", () => {
    expect(fuzzyFilter(labels, "undo", (l) => l)).toEqual(["Undo delete"]);
    expect(fuzzyFilter(labels, "unread", (l) => l)).toEqual([
      "Mark as unread",
      "Filter by unread",
    ]);
  });

  it("puts the better match first", () => {
    expect(
      fuzzyFilter(["Hide details", "Delete message"], "del", (l) => l),
    ).toEqual(["Delete message", "Hide details"]);
  });
});
