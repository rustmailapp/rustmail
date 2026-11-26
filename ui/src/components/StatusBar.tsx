import { createMemo, Show, type JSX } from "solid-js";
import {
  hasMore,
  socketLive,
  total,
  visibleMessages,
} from "../stores/messages";
import { serverInfo, smtpEndpoint } from "../stores/server";
import { smtpAddress } from "../lib/smtp";
import CopyButton from "./CopyButton";

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
 * A failed read of the server's details says the SMTP address is unknown.
 */
export default function StatusBar() {
  const ready = () => (serverInfo.state === "ready" ? serverInfo() : undefined);
  const endpoint = () => {
    const smtp = smtpEndpoint();
    return smtp ? smtpAddress(smtp.host, smtp.port) : undefined;
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

  let endpointText: HTMLSpanElement | undefined;

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
          <Show when={serverInfo.state === "errored"}>
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
            <CopyButton
              text={address()}
              label="Copy the SMTP address"
              source={() => endpointText}
            />
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
        <Hint keys={["i"]} label="details" />
        <Hint keys={["j", "k"]} label="move" />
        <Hint keys={["/"]} label="search" />
        <Hint keys={["d"]} label="delete" />
      </span>
    </footer>
  );
}
