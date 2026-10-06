import {
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  Show,
  type JSX,
} from "solid-js";
import {
  hasMore,
  socketLive,
  total,
  visibleMessages,
} from "../stores/messages";
import { wideLayout } from "../stores/layout";
import * as api from "../lib/api";
import {
  COPIED_FEEDBACK_MS,
  MANUAL_COPY_HINT,
  MANUAL_COPY_HINT_MS,
  writeClipboard,
  type CopyFeedback,
} from "../lib/clipboard";
import { CopyIcon } from "./icons";

const ENDPOINT_TITLE =
  "The SMTP port this server listens on. Docker or proxy port mappings can expose a different one.";

function Key(props: { children: JSX.Element }) {
  return (
    <kbd class="rounded border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 px-1 font-mono text-[10px] leading-4 text-zinc-600 dark:text-zinc-300">
      {props.children}
    </kbd>
  );
}

function Hint(props: { keys: string[]; label: string }) {
  return (
    <span class="inline-flex items-center gap-1">
      {props.keys.map((key) => (
        <Key>{key}</Key>
      ))}
      <span>{props.label}</span>
    </span>
  );
}

/**
 * The bottom bar: whether mail arrives live, where to send it, and the keys.
 *
 * The server's details are read once. A failed read says the SMTP address is
 * unknown rather than retrying: nothing on the page depends on it, and the
 * reload that would fix a stale server fixes this too.
 */
export default function StatusBar() {
  const [info] = createResource(() => api.getInfo());
  const ready = () => (info.state === "ready" ? info() : undefined);
  const endpoint = () => {
    const server = ready();
    return server ? `${location.hostname}:${server.smtp_port}` : undefined;
  };
  const unread = createMemo(
    () => visibleMessages().filter((m) => !m.is_read).length,
  );
  let seenLive = false;
  const connection = createMemo(() => {
    if (socketLive()) {
      seenLive = true;
      return "Live";
    }
    return seenLive ? "Reconnecting" : "Connecting";
  });

  const [feedback, setFeedback] = createSignal<CopyFeedback | null>(null);
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;
  let endpointText: HTMLSpanElement | undefined;
  onCleanup(() => clearTimeout(feedbackTimer));

  function showFeedback(kind: CopyFeedback, durationMs: number): void {
    setFeedback(kind);
    clearTimeout(feedbackTimer);
    feedbackTimer = setTimeout(() => setFeedback(null), durationMs);
  }

  async function copy(text: string): Promise<void> {
    if (await writeClipboard(text)) {
      showFeedback("copied", COPIED_FEEDBACK_MS);
      return;
    }
    if (endpointText) window.getSelection()?.selectAllChildren(endpointText);
    showFeedback("manual", MANUAL_COPY_HINT_MS);
  }

  return (
    <footer class="app-panel flex items-center gap-4 px-3 h-8 shrink-0 text-[11px] text-zinc-500 dark:text-zinc-400 whitespace-nowrap overflow-hidden">
      <span aria-live="polite" class="inline-flex items-center gap-1.5">
        <span
          class="size-1.5 rounded-full"
          classList={{
            "bg-emerald-500": socketLive(),
            "bg-amber-500 animate-pulse": !socketLive(),
          }}
        />
        {connection()}
      </span>

      <Show
        when={endpoint()}
        fallback={
          <Show when={info.state === "errored"}>
            <span class="text-zinc-500 dark:text-zinc-400">SMTP unknown</span>
          </Show>
        }
      >
        {(address) => (
          <span class="inline-flex items-center gap-1">
            <span title={ENDPOINT_TITLE}>
              SMTP{" "}
              <span
                ref={endpointText}
                class="font-mono text-zinc-700 dark:text-zinc-200"
              >
                {address()}
              </span>
            </span>
            <button
              onClick={() => copy(address())}
              aria-label={
                feedback() === "copied"
                  ? "Copied"
                  : feedback() === "manual"
                    ? MANUAL_COPY_HINT
                    : "Copy the SMTP address"
              }
              title={feedback() === "copied" ? "Copied" : "Copy"}
              class="rounded p-0.5 text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
            >
              <CopyIcon class="size-3" />
            </button>
            <Show when={feedback() === "manual"}>
              <span class="text-zinc-600 dark:text-zinc-300">
                {MANUAL_COPY_HINT}
              </span>
            </Show>
          </span>
        )}
      </Show>

      <span class="tabular-nums">
        {total()} {total() === 1 ? "message" : "messages"} {"·"} {unread()}
        {hasMore() ? "+" : ""} unread
      </span>

      <Show when={ready()}>
        {(server) => (
          <span class="text-zinc-500 dark:text-zinc-400">
            v{server().version}
          </span>
        )}
      </Show>

      <span class="ml-auto inline-flex items-center gap-3 text-zinc-500 dark:text-zinc-400">
        <Show when={!wideLayout()}>
          <Hint keys={["i"]} label="details" />
        </Show>
        <Hint keys={["j", "k"]} label="move" />
        <Hint keys={["/"]} label="search" />
        <Hint keys={["d"]} label="delete" />
      </span>
    </footer>
  );
}
