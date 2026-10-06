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
import { notify } from "../stores/notices";
import * as api from "../lib/api";
import { CopyIcon } from "./icons";

/** How long the copy button says it copied before it goes back to "Copy". */
const COPIED_FEEDBACK_MS = 1500;

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

  const [copied, setCopied] = createSignal(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(copiedTimer));

  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      notify("Could not copy the SMTP address.");
      return;
    }
    setCopied(true);
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
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
        {socketLive() ? "Live" : "Reconnecting"}
      </span>

      <Show
        when={endpoint()}
        fallback={
          <Show when={info.state === "errored"}>
            <span class="text-zinc-400 dark:text-zinc-500">SMTP unknown</span>
          </Show>
        }
      >
        {(address) => (
          <span class="inline-flex items-center gap-1">
            <span>SMTP</span>
            <span class="font-mono text-zinc-700 dark:text-zinc-200">
              {address()}
            </span>
            <button
              onClick={() => copy(address())}
              aria-label={copied() ? "Copied" : "Copy the SMTP address"}
              title={copied() ? "Copied" : "Copy"}
              class="rounded p-0.5 text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
            >
              <CopyIcon class="size-3" />
            </button>
          </span>
        )}
      </Show>

      <span class="tabular-nums">
        {total()} {total() === 1 ? "message" : "messages"} {"·"} {unread()}
        {hasMore() ? "+" : ""} unread
      </span>

      <Show when={ready()}>
        {(server) => (
          <span class="text-zinc-400 dark:text-zinc-500">
            v{server().version}
          </span>
        )}
      </Show>

      <span class="ml-auto inline-flex items-center gap-3 text-zinc-400 dark:text-zinc-500">
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
