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
  it("takes Command+K on Apple platforms", () => {
    expect(isCommandPaletteChord(press("k", { metaKey: true }), true)).toBe(
      true,
    );
    expect(isCommandPaletteChord(press("K", { metaKey: true }), true)).toBe(
      true,
    );
  });

  it("leaves Control+K to the text field on Apple platforms", () => {
    expect(isCommandPaletteChord(press("k", { ctrlKey: true }), true)).toBe(
      false,
    );
  });

  it("takes Control+K elsewhere, and not the Windows key", () => {
    expect(isCommandPaletteChord(press("k", { ctrlKey: true }), false)).toBe(
      true,
    );
    expect(isCommandPaletteChord(press("k", { metaKey: true }), false)).toBe(
      false,
    );
  });

  it("leaves a bare k to the selection shortcut", () => {
    expect(isCommandPaletteChord(press("k"), true)).toBe(false);
    expect(isCommandPaletteChord(press("k"), false)).toBe(false);
  });

  it("leaves chords with another modifier alone", () => {
    expect(
      isCommandPaletteChord(
        press("K", { metaKey: true, shiftKey: true }),
        true,
      ),
    ).toBe(false);
    expect(
      isCommandPaletteChord(press("k", { ctrlKey: true, altKey: true }), false),
    ).toBe(false);
    expect(
      isCommandPaletteChord(press("k", { metaKey: true, ctrlKey: true }), true),
    ).toBe(false);
  });
});
