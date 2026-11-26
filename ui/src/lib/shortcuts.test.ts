import { describe, expect, it } from "vitest";
import { isCommandPaletteChord } from "./shortcuts";

function press(
  key: string,
  modifiers: Partial<
    Record<"metaKey" | "ctrlKey" | "altKey" | "shiftKey", boolean>
  > = {},
) {
  return {
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  };
}

describe("isCommandPaletteChord", () => {
  it("takes Command+K and Control+K", () => {
    expect(isCommandPaletteChord(press("k", { metaKey: true }))).toBe(true);
    expect(isCommandPaletteChord(press("k", { ctrlKey: true }))).toBe(true);
    expect(isCommandPaletteChord(press("K", { metaKey: true }))).toBe(true);
  });

  it("leaves a bare k to the selection shortcut", () => {
    expect(isCommandPaletteChord(press("k"))).toBe(false);
  });

  it("leaves chords with Shift or Alt alone", () => {
    expect(
      isCommandPaletteChord(press("K", { metaKey: true, shiftKey: true })),
    ).toBe(false);
    expect(
      isCommandPaletteChord(press("k", { ctrlKey: true, altKey: true })),
    ).toBe(false);
  });
});
