import { createSignal, onCleanup, Show } from "solid-js";
import {
  COPIED_FEEDBACK_MS,
  MANUAL_COPY_HINT,
  MANUAL_COPY_HINT_MS,
  writeClipboard,
  type CopyFeedback,
} from "../lib/clipboard";
import { CopyIcon } from "./icons";

/**
 * A copy button that says whether the copy landed.
 *
 * Without a clipboard it selects `source` instead, so a press of Cmd/Ctrl+C
 * finishes the job, and says so next to the button.
 */
export default function CopyButton(props: {
  text: string;
  label: string;
  source: () => HTMLElement | undefined;
  showText?: boolean;
}) {
  const [feedback, setFeedback] = createSignal<CopyFeedback | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  function show(kind: CopyFeedback, durationMs: number): void {
    setFeedback(kind);
    clearTimeout(timer);
    timer = setTimeout(() => setFeedback(null), durationMs);
  }

  async function copy(): Promise<void> {
    if (await writeClipboard(props.text)) {
      show("copied", COPIED_FEEDBACK_MS);
      return;
    }
    const source = props.source();
    if (source) window.getSelection()?.selectAllChildren(source);
    show("manual", MANUAL_COPY_HINT_MS);
  }

  return (
    <>
      <Show when={feedback() === "manual"}>
        <span class="text-[11px] text-zinc-600 dark:text-zinc-300">
          {MANUAL_COPY_HINT}
        </span>
      </Show>
      <button
        onClick={copy}
        aria-label={
          feedback() === "copied"
            ? "Copied"
            : feedback() === "manual"
              ? MANUAL_COPY_HINT
              : props.label
        }
        title={feedback() === "copied" ? "Copied" : "Copy"}
        class="inline-flex shrink-0 items-center gap-1 rounded-md p-1 text-[11px] font-medium text-zinc-500 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-100 hover:bg-zinc-200/70 dark:hover:bg-zinc-700/60 transition cursor-pointer"
      >
        <CopyIcon class="size-3.5" />
        <Show when={props.showText}>
          <span aria-hidden="true" class="pr-0.5">
            {feedback() === "copied" ? "Copied" : "Copy"}
          </span>
        </Show>
      </button>
      <span role="status" class="sr-only">
        {feedback() === "copied" ? "Copied" : ""}
      </span>
    </>
  );
}
