import { afterEach, describe, expect, it, vi } from "vitest";
import { writeClipboard } from "./clipboard";

function stubClipboard(clipboard: Partial<Clipboard> | undefined): void {
  vi.stubGlobal("navigator", { clipboard });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("writeClipboard", () => {
  it("reports success once the text is written", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard({ writeText });

    await expect(writeClipboard("127.0.0.1:1025")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("127.0.0.1:1025");
  });

  it("reports failure when the page has no clipboard", async () => {
    stubClipboard(undefined);

    await expect(writeClipboard("127.0.0.1:1025")).resolves.toBe(false);
  });

  it("reports failure when the browser refuses the write", async () => {
    stubClipboard({
      writeText: vi.fn().mockRejectedValue(new DOMException("denied")),
    });

    await expect(writeClipboard("127.0.0.1:1025")).resolves.toBe(false);
  });
});
