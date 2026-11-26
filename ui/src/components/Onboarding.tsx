import { createMemo, createSignal, Show } from "solid-js";
import { smtpEndpoint } from "../stores/server";
import { smtpAddress, smtpSnippets, type SnippetStack } from "../lib/smtp";
import CopyButton from "./CopyButton";
import SegmentedControl from "./SegmentedControl";
import { EnvelopeIcon } from "./icons";

const SNIPPET_PANEL_ID = "smtp-snippet-panel";

function snippetTabId(stack: SnippetStack): string {
  return `smtp-snippet-tab-${stack}`;
}

/**
 * The inbox's empty state: where to point an app so its mail lands here.
 *
 * The address is the one this browser reaches the server by, which is not
 * always the one an app reaches it by; the snippets beside it say when.
 */
export function EmptyInbox() {
  let address: HTMLSpanElement | undefined;
  return (
    <div class="flex flex-col items-center px-6 text-center">
      <EnvelopeIcon class="size-12 mb-3 opacity-30" />
      <p class="text-sm">No messages yet</p>
      <Show
        when={smtpEndpoint()}
        fallback={
          <p class="text-xs mt-1">
            Send an email to the SMTP port to get started
          </p>
        }
      >
        {(smtp) => (
          <>
            <p class="text-xs mt-1">Point your app's SMTP settings at</p>
            <div class="mt-3 flex max-w-full items-center gap-1 rounded-lg border border-zinc-200 dark:border-zinc-700/80 bg-zinc-50 dark:bg-zinc-800/50 py-1 pl-3 pr-1">
              <span
                ref={address}
                data-testid="smtp-address"
                class="min-w-0 break-all font-mono text-sm text-zinc-800 dark:text-zinc-100 select-all"
              >
                {smtpAddress(smtp().host, smtp().port)}
              </span>
              <CopyButton
                text={smtpAddress(smtp().host, smtp().port)}
                label="Copy the SMTP address"
                source={() => address}
              />
            </div>
            <p class="text-[11px] mt-1.5">As reached from this browser</p>
          </>
        )}
      </Show>
    </div>
  );
}

/**
 * Config snippets for common stacks, shown in the message pane while the
 * inbox is empty, where there is no message to show and room for code.
 */
export function ConnectApp() {
  const [stack, setStack] = createSignal<SnippetStack>("node");
  let code: HTMLPreElement | undefined;
  const snippets = createMemo(() => {
    const smtp = smtpEndpoint();
    return smtp ? smtpSnippets(smtp.host, smtp.port) : [];
  });
  const stackOptions = createMemo(() =>
    snippets().map((snippet) => ({ id: snippet.stack, label: snippet.label })),
  );
  const current = () =>
    snippets().find((s) => s.stack === stack()) ?? snippets()[0];

  return (
    <section
      aria-label="Connect your app"
      class="app-panel flex-1 min-w-0 overflow-y-auto"
    >
      <div class="mx-auto flex min-h-full max-w-xl flex-col justify-center px-6 py-10">
        <h2 class="text-base font-semibold text-zinc-900 dark:text-zinc-50">
          Connect your app
        </h2>
        <p class="mt-1 text-sm text-zinc-600 dark:text-zinc-400">
          Plain SMTP, no username or password. Mail sent here shows up in the
          inbox as it arrives.
        </p>
        <SegmentedControl
          label="Stack"
          options={stackOptions()}
          value={stack()}
          onChange={setStack}
          tabs={{ panelId: SNIPPET_PANEL_ID, tabId: snippetTabId }}
          class="mt-5 self-start"
        />
        <Show when={current()}>
          {(snippet) => (
            <div
              role="tabpanel"
              id={SNIPPET_PANEL_ID}
              aria-labelledby={snippetTabId(snippet().stack)}
              tabIndex={0}
              class="mt-3 overflow-hidden rounded-lg border border-zinc-200 dark:border-zinc-700/80"
            >
              <div class="flex items-center justify-between gap-2 border-b border-zinc-200 dark:border-zinc-700/80 bg-zinc-50 dark:bg-zinc-800/50 py-1 pl-3 pr-1">
                <span class="truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400">
                  {snippet().file}
                </span>
                <Show when={snippet()} keyed>
                  {(shown) => (
                    <CopyButton
                      text={shown.code}
                      label={`Copy the ${shown.label} settings`}
                      source={() => code}
                      showText
                    />
                  )}
                </Show>
              </div>
              <pre
                ref={code}
                class="min-h-30 overflow-x-auto px-3 py-2.5 font-mono text-xs leading-relaxed text-zinc-800 dark:text-zinc-200"
              >
                {snippet().code}
              </pre>
            </div>
          )}
        </Show>
        <p class="mt-4 text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">
          The host and port are the ones this browser uses. In Docker, an app in
          another container uses the Compose service name as the host, and a
          host port remapped with <code class="font-mono">-p</code> replaces the
          port shown. If the server has STARTTLS on, clients that upgrade on
          their own need to trust its certificate.
        </p>
      </div>
    </section>
  );
}
