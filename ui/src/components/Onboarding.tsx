import { createMemo, createSignal, For, Show } from "solid-js";
import { smtpEndpoint } from "../stores/server";
import { smtpAddress, smtpSnippets, type SnippetStack } from "../lib/smtp";
import CopyButton from "./CopyButton";

const SNIPPET_PANEL_ID = "smtp-snippet-panel";

function snippetTabId(stack: SnippetStack): string {
  return `smtp-snippet-tab-${stack}`;
}

/** Keys that move between snippet tabs, as where they land in the list. */
const TAB_KEYS: Record<
  string,
  ((at: number, count: number) => number) | undefined
> = {
  ArrowRight: (at, count) => (at + 1) % count,
  ArrowLeft: (at, count) => (at - 1 + count) % count,
  Home: () => 0,
  End: (_, count) => count - 1,
};

function MailIcon() {
  return (
    <svg
      class="size-12 mb-3 opacity-30"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0L3.32 8.91a2.25 2.25 0 01-1.07-1.916V6.75"
      />
    </svg>
  );
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
      <MailIcon />
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
  const current = () =>
    snippets().find((s) => s.stack === stack()) ?? snippets()[0];

  function focusTab(event: KeyboardEvent): void {
    const land = TAB_KEYS[event.key];
    if (land === undefined) return;
    event.preventDefault();
    const list = snippets();
    const at = list.findIndex((s) => s.stack === stack());
    const next = list[land(at, list.length)];
    if (!next) return;
    setStack(next.stack);
    document.getElementById(snippetTabId(next.stack))?.focus();
  }

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
        <div
          role="tablist"
          aria-label="Stack"
          onKeyDown={focusTab}
          class="mt-5 inline-flex self-start rounded-lg bg-zinc-100 dark:bg-zinc-800/70 p-0.5"
        >
          <For each={snippets()}>
            {(snippet) => (
              <button
                role="tab"
                id={snippetTabId(snippet.stack)}
                aria-selected={stack() === snippet.stack}
                aria-controls={SNIPPET_PANEL_ID}
                tabIndex={stack() === snippet.stack ? 0 : -1}
                onClick={() => setStack(snippet.stack)}
                class="rounded-md px-3 py-1 text-xs font-medium transition cursor-pointer"
                classList={{
                  "bg-white dark:bg-zinc-700 text-zinc-900 dark:text-zinc-50 shadow-sm":
                    stack() === snippet.stack,
                  "text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200":
                    stack() !== snippet.stack,
                }}
              >
                {snippet.label}
              </button>
            )}
          </For>
        </div>
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
