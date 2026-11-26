import type { JSX } from "solid-js";

/** A key or chord, drawn as a keycap on a panel surface. */
export default function Kbd(props: { children: JSX.Element }) {
  return (
    <kbd class="rounded border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 px-1 font-mono text-[10px] leading-4 text-zinc-600 dark:text-zinc-300">
      {props.children}
    </kbd>
  );
}
