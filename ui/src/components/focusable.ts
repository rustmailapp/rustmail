const FOCUSABLE =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

/** The elements inside `root` that Tab stops on, in tab order. */
export function focusableIn(root: ParentNode): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) =>
      !element.matches(":disabled") &&
      element.tabIndex !== -1 &&
      element.getClientRects().length > 0,
  );
}
