import {
  createEffect,
  createSignal,
  on,
  onCleanup,
  type Accessor,
} from "solid-js";

/**
 * Mirrors `source`, but only after it has held the same value for `delayMs`.
 *
 * Values the source passes through on the way somewhere else are dropped. The
 * inbox drives fetches off the selection, and a held-down arrow key walks the
 * selection at the OS key-repeat rate, so following every intermediate value
 * would fire a request per row transited.
 */
export function debounced<T>(
  source: Accessor<T>,
  delayMs: number,
): Accessor<T> {
  const [settled, setSettled] = createSignal(source());

  createEffect(
    on(
      source,
      (next) => {
        const timer = setTimeout(() => setSettled(() => next), delayMs);
        onCleanup(() => clearTimeout(timer));
      },
      { defer: true },
    ),
  );

  return settled;
}
