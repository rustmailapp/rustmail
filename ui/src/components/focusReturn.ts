import { onCleanup } from "solid-js";

/**
 * Hands focus back, on cleanup, to whatever had it when this was called.
 *
 * Call it while a dialog's content is set up, before anything inside takes
 * focus. Focus only goes back when it is still inside `container` or nowhere,
 * so a dialog that closes because another one opened leaves that one alone.
 */
export function returnFocusOnCleanup(
  container: () => HTMLElement | undefined,
): void {
  const focused = document.activeElement;
  const opener =
    focused instanceof HTMLElement && focused !== document.body
      ? focused
      : null;
  onCleanup(() => {
    const now = document.activeElement;
    const focusWasInside =
      now === null ||
      now === document.body ||
      (container()?.contains(now) ?? false);
    if (focusWasInside && opener?.isConnected) {
      opener.focus({ preventScroll: true });
    }
  });
}
