import {
  createSignal,
  createMemo,
  For,
  onCleanup,
  onMount,
  Show,
  Switch,
  Match,
  type JSX,
} from "solid-js";
import {
  deleteWithUndo,
  liveSummary,
  selectedId,
  setSelectedId,
  starMessage,
} from "../stores/messages";
import {
  closeDetails,
  detailsDrawerOpen,
  toggleDetails,
  wideLayout,
} from "../stores/layout";
import {
  MOBILE_PREVIEW_WIDTH_PX,
  PREVIEW_WIDTHS,
  previewWidth,
  setPreviewWidth,
} from "../stores/previewWidth";
import * as api from "../lib/api";
import { formatDate, formatSize } from "../lib/format";
import { debounced } from "../lib/reactive";
import type { AuthResults, Message } from "../lib/types";
import {
  createPaneRead,
  failed,
  ReadState,
  settled,
  type PaneRead,
} from "./PaneRead";
import DetailRail, {
  downloadable,
  StatusBadge,
  type RailReads,
} from "./DetailRail";
import {
  DownloadIcon,
  InfoIcon,
  PaperclipIcon,
  StarIcon,
  TrashIcon,
} from "./icons";

type View = "preview" | "text" | "raw";

const VIEWS: readonly SwitchOption<View>[] = [
  { id: "preview", label: "Preview" },
  { id: "text", label: "Text" },
  { id: "raw", label: "Raw" },
];

type SwitchOption<T extends string> = { id: T; label: string };

/**
 * Raw source fetched for the Raw view.
 *
 * Laying out a whole large message is what made the view freeze, so the
 * server is asked for a bounded prefix and the rest stays behind the .eml
 * download.
 */
const RAW_PREVIEW_LIMIT_BYTES = 128 * 1024;

/**
 * How long the selection must hold still before the pane loads it.
 *
 * Short enough to feel immediate on a deliberate press, long enough that
 * arrowing through the inbox does not fetch every row on the way.
 */
const SELECTION_SETTLE_MS = 120;

const DETAILS_ID = "message-details";
const HUE_TURN = 360;
const MESSAGE_LIST_SELECTOR = '[role="listbox"][aria-label="Messages"]';

/** A sender split into the name to show and the address behind it. */
function parseSender(sender: string): { name: string | null; address: string } {
  const named = /^\s*"?(.*?)"?\s*<([^>]+)>\s*$/.exec(sender);
  if (named && named[1]) return { name: named[1], address: named[2] };
  return { name: null, address: named ? named[2] : sender };
}

/** Up to two letters standing for the sender in its avatar. */
function initials(sender: string): string {
  const { name, address } = parseSender(sender);
  const words = (name ?? address.split("@")[0])
    .split(/[\s._+-]+/)
    .filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : words[0];
  return (letters ?? "?").slice(0, 2).toUpperCase();
}

/** A stable hue per address, so the same sender keeps the same colour. */
function senderHue(sender: string): number {
  const { address } = parseSender(sender);
  let hash = 0;
  for (const char of address.toLowerCase()) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return hash % HUE_TURN;
}

/**
 * The message the selection points at, and everything about it.
 *
 * Renders the centre panel and, on a wide viewport, the details rail beside
 * it; on a narrow one the rail is a drawer over the message body instead. Both
 * places read the same selection, so the reads live here and the rail is
 * handed them rather than starting its own.
 */
export default function MessageDetail() {
  const [view, setView] = createSignal<View>("preview");
  const [headersOpen, setHeadersOpen] = createSignal(false);
  const settledId = debounced(selectedId, SELECTION_SETTLE_MS);
  const activeId = createMemo(() =>
    selectedId() === settledId() ? settledId() : null,
  );

  const message = createPaneRead(activeId, api.getMessage);
  const reads: RailReads = {
    attachments: createPaneRead(activeId, api.listAttachments),
    auth: createPaneRead(activeId, api.getAuthResults),
    headers: createPaneRead(
      () => (headersOpen() ? activeId() : null),
      api.getHeaders,
    ),
  };
  const rawSource = createPaneRead(
    () => (view() === "raw" ? activeId() : null),
    (id, signal) => api.getRawMessage(id, RAW_PREVIEW_LIMIT_BYTES, signal),
  );

  const loaded = () => (selectedId() ? settled(message) : undefined);
  const files = createMemo(() => {
    const list = settled(reads.attachments);
    return list === undefined ? undefined : downloadable(list);
  });
  const toggleHeaders = () => setHeadersOpen((open) => !open);

  return (
    <>
      <section
        aria-label="Message"
        class="app-panel flex-1 min-w-0 flex flex-col overflow-hidden"
      >
        <Show when={selectedId()} fallback={<NothingSelected />}>
          <Show
            when={loaded()}
            fallback={<ReadState read={message} label="this message" />}
          >
            {(msg) => (
              <>
                <MessageHeader message={msg()} reads={reads} />
                <div class="flex items-center justify-between gap-3 px-5 pb-3">
                  <div class="flex flex-wrap items-center gap-2">
                    <SegmentedSwitch
                      label="Message view"
                      options={VIEWS}
                      value={view()}
                      onChange={setView}
                    />
                    <Show when={view() === "preview" && msg().html_body}>
                      <SegmentedSwitch
                        label="Preview width"
                        options={PREVIEW_WIDTHS}
                        value={previewWidth()}
                        onChange={setPreviewWidth}
                      />
                    </Show>
                  </div>
                  <Show when={!wideLayout() && !detailsDrawerOpen()}>
                    <CompactChecks
                      auth={reads.auth}
                      attachmentCount={files()?.length ?? 0}
                    />
                  </Show>
                </div>
                <div class="relative flex-1 min-h-0 flex flex-col border-t border-zinc-100 dark:border-zinc-800/70">
                  <div
                    class="flex-1 min-h-0 overflow-auto max-w-4xl mx-auto w-full"
                    inert={detailsDrawerOpen() || undefined}
                  >
                    <MessageBody
                      view={view()}
                      message={msg()}
                      rawSource={rawSource}
                    />
                  </div>
                  <Show when={detailsDrawerOpen()}>
                    <DetailsDrawer>
                      <DetailRail
                        message={msg()}
                        reads={reads}
                        files={files()}
                        headersOpen={headersOpen()}
                        onToggleHeaders={toggleHeaders}
                      />
                    </DetailsDrawer>
                  </Show>
                </div>
              </>
            )}
          </Show>
        </Show>
      </section>
      <Show when={wideLayout()}>
        <aside
          id={DETAILS_ID}
          aria-label="Message details"
          class="app-panel w-80 shrink-0 overflow-y-auto"
        >
          <Show
            when={loaded()}
            fallback={
              <Show
                when={selectedId()}
                fallback={
                  <p class="p-4 text-xs text-zinc-400 dark:text-zinc-500">
                    Details of the selected message show here.
                  </p>
                }
              >
                <ReadState read={message} label="the message details" />
              </Show>
            }
          >
            {(msg) => (
              <DetailRail
                message={msg()}
                reads={reads}
                files={files()}
                headersOpen={headersOpen()}
                onToggleHeaders={toggleHeaders}
              />
            )}
          </Show>
        </aside>
      </Show>
    </>
  );
}

/**
 * The details rail as a drawer over the message body.
 *
 * Opening it moves focus inside, so the keyboard lands where the content is;
 * closing hands focus back to what opened it, or to the message list when the
 * opener is gone. Focus is only taken back when it was in the drawer, so a
 * drawer that closes because the selection cleared does not pull focus out of
 * the search box. Escape is caught here rather than at the document, since the
 * page shortcut skips text fields and would leave the tag input stuck open.
 */
function DetailsDrawer(props: { children: JSX.Element }) {
  let drawer!: HTMLElement;
  let opener: HTMLElement | null = null;

  onMount(() => {
    const active = document.activeElement;
    opener =
      active instanceof HTMLElement && active !== document.body ? active : null;
    drawer.focus({ preventScroll: true });
  });

  onCleanup(() => {
    const active = document.activeElement;
    const focusWasInside =
      active === null || active === document.body || drawer.contains(active);
    if (!focusWasInside) return;
    const target = opener?.isConnected
      ? opener
      : document.querySelector<HTMLElement>(MESSAGE_LIST_SELECTOR);
    target?.focus({ preventScroll: true });
  });

  function closeOnEscape(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    closeDetails();
  }

  return (
    <aside
      ref={drawer}
      id={DETAILS_ID}
      aria-label="Message details"
      tabIndex={-1}
      on:keydown={closeOnEscape}
      class="details-drawer absolute inset-y-0 right-0 z-10 w-80 max-w-full overflow-y-auto border-l border-zinc-200 dark:border-zinc-800 outline-none animate-fade-in"
    >
      {props.children}
    </aside>
  );
}

function NothingSelected() {
  return (
    <div class="flex flex-col items-center justify-center h-full text-zinc-500">
      <svg
        class="size-10 mb-2 opacity-30"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.64 0 8.577 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.64 0-8.577-3.007-9.963-7.178z"
        />
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </svg>
      <p class="text-sm">Select a message to view</p>
    </div>
  );
}

const ACTION_CLASS =
  "p-1.5 transition cursor-pointer first:rounded-l-lg last:rounded-r-lg hover:bg-zinc-100 dark:hover:bg-zinc-800";

function MessageHeader(props: { message: Message; reads: RailReads }) {
  const sender = () => parseSender(props.message.sender);
  const starred = () => liveSummary(props.message).is_starred;
  const meta = () =>
    [
      props.message.recipients.join(", "),
      formatDate(props.message.created_at),
      formatSize(props.message.size),
    ].join(" · ");

  return (
    <div class="flex items-start gap-3 px-5 pt-4 pb-3">
      <div
        class="sender-avatar size-9 shrink-0 rounded-full flex items-center justify-center text-xs font-semibold select-none"
        style={{ "--avatar-hue": String(senderHue(props.message.sender)) }}
        aria-hidden="true"
      >
        {initials(props.message.sender)}
      </div>
      <div class="min-w-0 flex-1">
        <h2 class="text-base font-semibold leading-snug text-zinc-900 dark:text-zinc-50 truncate">
          {props.message.subject || "(no subject)"}
        </h2>
        <p class="mt-0.5 text-xs text-zinc-500 dark:text-zinc-400 truncate">
          <span class="font-medium text-zinc-700 dark:text-zinc-200">
            {sender().name ?? (sender().address || "(no sender)")}
          </span>
          <Show when={sender().name}>
            <span> &lt;{sender().address}&gt;</span>
          </Show>
          <span class="text-zinc-400 dark:text-zinc-500"> {"→"} </span>
          {meta()}
        </p>
      </div>
      <div class="flex items-center gap-2 shrink-0">
        <Show when={!wideLayout()}>
          <button
            onClick={toggleDetails}
            aria-expanded={detailsDrawerOpen()}
            aria-controls={detailsDrawerOpen() ? DETAILS_ID : undefined}
            title="Details (i)"
            class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition cursor-pointer"
            classList={{
              "border-orange-300 dark:border-orange-500/40 bg-orange-50 dark:bg-orange-500/10 text-orange-700 dark:text-orange-300":
                detailsDrawerOpen(),
              "border-zinc-200 dark:border-zinc-700/70 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800":
                !detailsDrawerOpen(),
            }}
          >
            <InfoIcon class="size-3.5" />
            Details
          </button>
        </Show>
        <div class="inline-flex items-center rounded-lg border border-zinc-200 dark:border-zinc-700/70 divide-x divide-zinc-200 dark:divide-zinc-700/70">
          <button
            onClick={() => starMessage(props.message.id, !starred())}
            class={ACTION_CLASS}
            classList={{
              "text-amber-400 hover:text-amber-500": starred(),
              "text-zinc-500 dark:text-zinc-400 hover:text-amber-400":
                !starred(),
            }}
            title={starred() ? "Unstar" : "Star"}
          >
            <StarIcon class="size-4" filled={starred()} />
          </button>
          <a
            href={api.exportUrl(props.message.id, "eml")}
            download={`${props.message.id}.eml`}
            class={`${ACTION_CLASS} text-zinc-500 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-100`}
            title="Download .eml"
          >
            <DownloadIcon class="size-4" />
          </a>
          <button
            onClick={() => {
              setSelectedId(null);
              deleteWithUndo(props.message.id);
            }}
            class={`${ACTION_CLASS} text-zinc-500 dark:text-zinc-400 hover:text-red-600 dark:hover:text-red-400`}
            title="Delete"
          >
            <TrashIcon class="size-4" />
          </button>
        </div>
      </div>
    </div>
  );
}

/** A row of toggle buttons, exactly one of them pressed. */
function SegmentedSwitch<T extends string>(props: {
  label: string;
  options: readonly SwitchOption<T>[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div
      role="group"
      aria-label={props.label}
      class="inline-flex rounded-lg bg-zinc-100 dark:bg-zinc-800/70 p-0.5"
    >
      <For each={props.options}>
        {(option) => (
          <button
            onClick={() => props.onChange(option.id)}
            aria-pressed={props.value === option.id}
            class="rounded-md px-3 py-1 text-xs font-medium transition cursor-pointer"
            classList={{
              "bg-white dark:bg-zinc-700 text-zinc-900 dark:text-zinc-50 shadow-sm":
                props.value === option.id,
              "text-zinc-500 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200":
                props.value !== option.id,
            }}
          >
            {option.label}
          </button>
        )}
      </For>
    </div>
  );
}

/** The SPF and DKIM verdicts and the attachment count, while the rail is away. */
function CompactChecks(props: {
  auth: PaneRead<AuthResults>;
  attachmentCount: number;
}) {
  return (
    <div class="flex items-center gap-1.5 min-w-0">
      <Show when={settled(props.auth)}>
        {(results) => (
          <>
            <StatusBadge
              label="SPF"
              status={results().spf[0]?.status ?? "none"}
            />
            <StatusBadge
              label="DKIM"
              status={results().dkim[0]?.status ?? "none"}
            />
          </>
        )}
      </Show>
      <Show when={props.attachmentCount > 0}>
        <span
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400"
          title={`${props.attachmentCount} ${props.attachmentCount === 1 ? "attachment" : "attachments"}`}
        >
          <PaperclipIcon class="size-3.5" />
          {props.attachmentCount}
        </span>
      </Show>
    </div>
  );
}

function MessageBody(props: {
  view: View;
  message: Message;
  rawSource: PaneRead<string>;
}): JSX.Element {
  const rawReady = createMemo(() => {
    if (failed(props.rawSource.value)) return undefined;
    const raw = props.rawSource.value();
    return raw === undefined ? undefined : { raw };
  });

  return (
    <Switch>
      <Match when={props.view === "preview"}>
        <HtmlPreview
          html={props.message.html_body}
          text={props.message.text_body}
          messageId={props.message.id}
        />
      </Match>
      <Match when={props.view === "text"}>
        <pre class="p-5 text-sm text-zinc-700 dark:text-zinc-300 whitespace-pre-wrap font-mono">
          {props.message.text_body || "(no text body)"}
        </pre>
      </Match>
      <Match when={props.view === "raw"}>
        <Show
          when={rawReady()}
          fallback={<ReadState read={props.rawSource} label="the raw source" />}
        >
          {(value) => (
            <RawView
              raw={value().raw}
              messageId={props.message.id}
              size={props.message.size}
            />
          )}
        </Show>
      </Match>
    </Switch>
  );
}

function rewriteCidUrls(html: string, messageId: string): string {
  const inlineUrl = (cid: string) =>
    `/api/v1/messages/${encodeURIComponent(messageId)}/inline/${encodeURIComponent(cid)}`;

  let result = html.replace(
    /(?:src|background)=["']cid:([^"']+)["']/gi,
    (match, cid) => {
      const attr = match.startsWith("src") ? "src" : "background";
      return `${attr}="${inlineUrl(cid)}"`;
    },
  );

  result = result.replace(
    /url\((['"]?)cid:([^)'"]+)\1\)/gi,
    (_, quote, cid) => `url(${quote}${inlineUrl(cid)}${quote})`,
  );

  return result;
}

function rewriteToAbsoluteUrls(html: string): string {
  let result = html.replace(
    /(?:src|background)=["'](\/api\/v1\/[^"']+)["']/gi,
    (match, path) => {
      const attr = match.startsWith("src") ? "src" : "background";
      return `${attr}="${location.origin}${path}"`;
    },
  );

  result = result.replace(
    /url\((['"]?)(\/api\/v1\/[^)'"]+)\1\)/gi,
    (_, quote, path) => `url(${quote}${location.origin}${path}${quote})`,
  );

  return result;
}

function cspMeta(): string {
  const origin = location.origin;
  return `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src ${origin} data: http: https:; font-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none';">`;
}

const LINK_TARGET_BLANK = '<base target="_blank">';

function HtmlPreview(props: {
  html: string | null;
  text: string | null;
  messageId: string;
}) {
  const rewritten = createMemo(() =>
    props.html ? rewriteCidUrls(props.html, props.messageId) : null,
  );

  const srcdoc = createMemo(() => {
    const r = rewritten();
    if (r === null) return null;
    return cspMeta() + LINK_TARGET_BLANK + rewriteToAbsoluteUrls(r);
  });
  const mobile = () => previewWidth() === "mobile";

  return (
    <Show
      when={srcdoc()}
      fallback={
        <pre class="p-5 text-sm text-zinc-700 dark:text-zinc-300 whitespace-pre-wrap">
          {props.text || "(no content)"}
        </pre>
      }
    >
      {(html) => (
        <div
          class="h-full flex justify-center motion-safe:transition-[padding] motion-safe:duration-200 motion-safe:ease-out"
          classList={{ "p-4": mobile() }}
        >
          <div
            class="h-full max-w-full shrink-0 overflow-hidden motion-safe:transition-[width] motion-safe:duration-200 motion-safe:ease-out"
            classList={{
              "rounded-lg ring-1 ring-zinc-200 dark:ring-zinc-700/70 shadow-sm":
                mobile(),
            }}
            style={{
              width: mobile() ? `${MOBILE_PREVIEW_WIDTH_PX}px` : "100%",
            }}
          >
            <iframe
              sandbox="allow-popups allow-popups-to-escape-sandbox"
              srcdoc={html()}
              class="w-full h-full border-0 bg-white"
              title="Email HTML preview"
            />
          </div>
        </div>
      )}
    </Show>
  );
}

function RawView(props: { raw: string; messageId: string; size: number }) {
  const truncated = () => props.size > RAW_PREVIEW_LIMIT_BYTES;

  return (
    <>
      <Show when={truncated()}>
        <div class="border-b border-amber-200 dark:border-amber-900/50 bg-amber-50 dark:bg-amber-900/20 px-5 py-2 text-xs text-amber-800 dark:text-amber-300">
          Showing the first {formatSize(RAW_PREVIEW_LIMIT_BYTES)} of{" "}
          {formatSize(props.size)}.{" "}
          <a
            href={api.exportUrl(props.messageId, "eml")}
            download={`${props.messageId}.eml`}
            class="font-medium underline underline-offset-2 hover:text-amber-900 dark:hover:text-amber-200"
          >
            Download the full source
          </a>
        </div>
      </Show>
      <pre class="p-5 text-xs text-zinc-600 dark:text-zinc-400 whitespace-pre-wrap font-mono leading-relaxed">
        {props.raw}
      </pre>
    </>
  );
}
