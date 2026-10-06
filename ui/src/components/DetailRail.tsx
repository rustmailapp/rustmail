import { createSignal, For, Show, type JSX } from "solid-js";
import { liveSummary } from "../stores/messages";
import { notify } from "../stores/notices";
import * as api from "../lib/api";
import { formatDateTime, formatSize } from "../lib/format";
import type {
  Attachment,
  AuthCheck,
  AuthResults,
  Message,
  MessageHeader,
  MessageSummary,
} from "../lib/types";
import { ReadState, settled, type PaneRead } from "./PaneRead";
import { PaperclipIcon } from "./icons";

/** The selection's reads that the rail shows, owned by the message pane. */
export interface RailReads {
  attachments: PaneRead<Attachment[]>;
  auth: PaneRead<AuthResults>;
  headers: PaneRead<MessageHeader[]>;
}

/**
 * The attachments a reader can download.
 *
 * An inline part with a content id and no filename is an image the HTML body
 * already shows, so it is not offered on its own.
 */
export function downloadable(attachments: readonly Attachment[]): Attachment[] {
  return attachments.filter((a) => a.filename || !a.content_id);
}

const STATUS_COLORS: Record<string, string> = {
  pass: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
  fail: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
  hardfail: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
  softfail:
    "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  neutral:
    "bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300",
  temperror:
    "bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300",
  permerror:
    "bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300",
  info: "bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300",
};
const NEUTRAL_STATUS_COLOR =
  "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400";

/** A check result, coloured by how it went; `label` names the check. */
export function StatusBadge(props: { status: string; label?: string }) {
  const color = () => {
    const key = props.status.toLowerCase().replace(/^arc:/, "");
    return Object.hasOwn(STATUS_COLORS, key)
      ? STATUS_COLORS[key]
      : NEUTRAL_STATUS_COLOR;
  };

  return (
    <span
      class={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] font-semibold uppercase tracking-wide ${color()}`}
    >
      <Show when={props.label}>
        <span class="opacity-70">{props.label}</span>
      </Show>
      {props.status}
    </span>
  );
}

function Section(props: {
  label: string;
  aside?: JSX.Element;
  children: JSX.Element;
}) {
  return (
    <section class="px-4 py-3.5">
      <div class="flex items-center justify-between gap-2 mb-2 min-h-5">
        <h3 class="text-[10px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          {props.label}
        </h3>
        {props.aside}
      </div>
      {props.children}
    </section>
  );
}

const RAIL_READ_FRAME = "py-1";

/**
 * Everything about the selected message that is not its body.
 *
 * `files` is the downloadable attachments, or `undefined` until the read
 * settles; the pane works it out once so its own count cannot disagree.
 */
export default function DetailRail(props: {
  message: Message;
  reads: RailReads;
  files: Attachment[] | undefined;
  headersOpen: boolean;
  onToggleHeaders: () => void;
}) {
  return (
    <div class="divide-y divide-zinc-100 dark:divide-zinc-800/70">
      <Section label="Summary">
        <SummaryList message={props.message} />
      </Section>
      <Section label="Authentication">
        <Show
          when={settled(props.reads.auth)}
          fallback={
            <ReadState
              read={props.reads.auth}
              label="authentication results"
              class={RAIL_READ_FRAME}
            />
          }
        >
          {(results) => <AuthList results={results()} />}
        </Show>
      </Section>
      <AttachmentsSection
        message={props.message}
        read={props.reads.attachments}
        files={props.files}
      />
      <Section
        label="Headers"
        aside={
          <button
            onClick={() => props.onToggleHeaders()}
            aria-expanded={props.headersOpen}
            class="rounded-md px-1.5 py-0.5 text-[11px] font-medium text-orange-600 dark:text-orange-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition cursor-pointer"
          >
            {props.headersOpen ? "Hide headers" : "Show headers"}
          </button>
        }
      >
        <Show
          when={props.headersOpen}
          fallback={
            <p class="text-xs text-zinc-400 dark:text-zinc-500">
              Every header the message carries, as received.
            </p>
          }
        >
          <Show
            when={settled(props.reads.headers)}
            fallback={
              <ReadState
                read={props.reads.headers}
                label="headers"
                class={RAIL_READ_FRAME}
              />
            }
          >
            {(headers) => <HeaderList headers={headers()} />}
          </Show>
        </Show>
      </Section>
    </div>
  );
}

function SummaryList(props: { message: Message }) {
  return (
    <dl class="grid grid-cols-[3.25rem_minmax(0,1fr)] gap-x-3 gap-y-2 text-xs">
      <dt class="text-zinc-400 dark:text-zinc-500">From</dt>
      <dd class="text-zinc-800 dark:text-zinc-200 break-all">
        {props.message.sender || "(no sender)"}
      </dd>
      <dt class="text-zinc-400 dark:text-zinc-500">To</dt>
      <dd class="text-zinc-800 dark:text-zinc-200 break-all">
        <For each={props.message.recipients}>{(r) => <div>{r}</div>}</For>
      </dd>
      <dt class="text-zinc-400 dark:text-zinc-500">Date</dt>
      <dd class="text-zinc-800 dark:text-zinc-200">
        {formatDateTime(props.message.created_at)}
      </dd>
      <dt class="text-zinc-400 dark:text-zinc-500">Size</dt>
      <dd class="text-zinc-800 dark:text-zinc-200">
        {formatSize(props.message.size)}
      </dd>
      <dt class="text-zinc-400 dark:text-zinc-500 pt-0.5">Tags</dt>
      <dd>
        <TagEditor message={props.message} />
      </dd>
    </dl>
  );
}

function AuthList(props: { results: AuthResults }) {
  const kinds = (): [string, AuthCheck[]][] => [
    ["DKIM", props.results.dkim],
    ["SPF", props.results.spf],
    ["DMARC", props.results.dmarc],
    ...(props.results.arc.length > 0
      ? [["ARC", props.results.arc] as [string, AuthCheck[]]]
      : []),
  ];
  const isEmpty = () => kinds().every(([, checks]) => checks.length === 0);

  return (
    <Show
      when={!isEmpty()}
      fallback={
        <div class="text-xs text-zinc-400 dark:text-zinc-500">
          <p>No authentication headers found.</p>
          <p class="mt-1">
            DKIM, SPF, and DMARC headers are typically added by receiving mail
            servers.
          </p>
        </div>
      }
    >
      <div class="space-y-2.5">
        <For each={kinds()}>
          {([kind, checks]) => (
            <div class="flex items-start gap-2">
              <span class="w-11 shrink-0 pt-0.5 text-[11px] font-medium text-zinc-500 dark:text-zinc-400">
                {kind}
              </span>
              <div class="min-w-0 flex-1 space-y-1.5">
                <Show
                  when={checks.length > 0}
                  fallback={<StatusBadge status="none" />}
                >
                  <For each={checks}>
                    {(check) => (
                      <div>
                        <StatusBadge status={check.status} />
                        <p
                          class="mt-1 font-mono text-[10px] leading-relaxed text-zinc-500 dark:text-zinc-400 break-all line-clamp-2"
                          title={check.details}
                        >
                          {check.details}
                        </p>
                      </div>
                    )}
                  </For>
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

function AttachmentsSection(props: {
  message: Message;
  read: PaneRead<Attachment[]>;
  files: Attachment[] | undefined;
}) {
  return (
    <Section
      label="Attachments"
      aside={
        <Show when={props.files}>
          {(list) => (
            <span class="text-[11px] tabular-nums text-zinc-400 dark:text-zinc-500">
              {list().length}
            </span>
          )}
        </Show>
      }
    >
      <Show
        when={props.files}
        fallback={
          <ReadState
            read={props.read}
            label="attachments"
            class={RAIL_READ_FRAME}
          />
        }
      >
        {(list) => (
          <Show
            when={list().length > 0}
            fallback={
              <p class="text-xs text-zinc-400 dark:text-zinc-500">
                No attachments
              </p>
            }
          >
            <ul class="-mx-2 space-y-0.5">
              <For each={list()}>
                {(att) => (
                  <li>
                    <a
                      href={api.attachmentUrl(props.message.id, att.id)}
                      download={att.filename || "attachment"}
                      class="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition"
                    >
                      <PaperclipIcon class="size-3.5 shrink-0 text-zinc-400 dark:text-zinc-500" />
                      <span class="min-w-0 flex-1 truncate">
                        {att.filename || "attachment"}
                      </span>
                      <Show when={att.size}>
                        {(size) => (
                          <span class="shrink-0 tabular-nums text-zinc-400 dark:text-zinc-500">
                            {formatSize(size())}
                          </span>
                        )}
                      </Show>
                    </a>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        )}
      </Show>
    </Section>
  );
}

function HeaderList(props: { headers: MessageHeader[] }) {
  return (
    <dl class="space-y-2 font-mono text-[11px] leading-relaxed">
      <For each={props.headers}>
        {(h) => (
          <div>
            <dt class="font-medium text-zinc-500 dark:text-zinc-400">
              {h.name}
            </dt>
            <dd class="text-zinc-700 dark:text-zinc-300 break-all">
              {h.value}
            </dd>
          </div>
        )}
      </For>
    </dl>
  );
}

function TagEditor(props: { message: MessageSummary }) {
  const [input, setInput] = createSignal("");
  const tags = () => liveSummary(props.message).tags;

  async function addTag(value: string) {
    const tag = value.trim().toLowerCase();
    if (!tag || tags().includes(tag)) return;
    try {
      await api.setTags(props.message.id, [...tags(), tag]);
      setInput("");
    } catch {
      notify(`Could not add the tag "${tag}".`);
    }
  }

  async function removeTag(tag: string) {
    try {
      await api.setTags(
        props.message.id,
        tags().filter((t) => t !== tag),
      );
    } catch {
      notify(`Could not remove the tag "${tag}".`);
    }
  }

  return (
    <div class="flex items-center gap-1.5 flex-wrap">
      <For each={tags()}>
        {(tag) => (
          <span class="inline-flex items-center gap-1 rounded-md bg-orange-100 dark:bg-orange-900/40 px-2 py-0.5 text-xs font-medium text-orange-700 dark:text-orange-300">
            {tag}
            <button
              onClick={() => removeTag(tag)}
              aria-label={`Remove the tag ${tag}`}
              class="hover:text-red-500 dark:hover:text-red-400 transition cursor-pointer"
            >
              <svg class="size-3" viewBox="0 0 20 20" fill="currentColor">
                <path d="M6.28 5.22a.75.75 0 00-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 101.06 1.06L10 11.06l3.72 3.72a.75.75 0 101.06-1.06L11.06 10l3.72-3.72a.75.75 0 00-1.06-1.06L10 8.94 6.28 5.22z" />
              </svg>
            </button>
          </span>
        )}
      </For>
      <input
        type="text"
        placeholder="Add tag..."
        value={input()}
        onInput={(e) => setInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            addTag(input());
          }
          if (e.key === "Backspace" && input() === "" && tags().length > 0) {
            removeTag(tags()[tags().length - 1]);
          }
        }}
        class="bg-transparent text-xs text-zinc-700 dark:text-zinc-300 placeholder-zinc-400 dark:placeholder-zinc-600 outline-none min-w-[80px] flex-1 rounded-md px-1.5 py-0.5 transition focus:bg-zinc-100 dark:focus:bg-zinc-800/60"
      />
    </div>
  );
}
