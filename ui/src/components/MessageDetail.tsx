import {
  createSignal,
  createResource,
  Show,
  For,
  Switch,
  Match,
} from "solid-js";
import { selectedId, setSelectedId, messages } from "../stores/messages";
import * as api from "../lib/api";
import { confirm } from "./ConfirmDialog";
import { formatDate, formatSize } from "../lib/format";
import type { Attachment, AuthCheck, AuthResults } from "../lib/types";

type Tab = "html" | "text" | "headers" | "auth" | "raw";

const TAB_LABELS: Record<Tab, string> = {
  html: "HTML",
  text: "Text",
  headers: "Headers",
  auth: "Auth",
  raw: "Raw",
};

export default function MessageDetail() {
  const [tab, setTab] = createSignal<Tab>("html");

  const [message] = createResource(selectedId, async (id) => {
    if (!id) return null;
    return api.getMessage(id);
  });

  const [attachments] = createResource(selectedId, async (id) => {
    if (!id) return [];
    return api.listAttachments(id);
  });

  const [rawSource] = createResource(selectedId, async (id) => {
    if (!id) return null;
    return api.getRawMessage(id);
  });

  const authSource = () => (tab() === "auth" ? selectedId() : null);
  const [authResults] = createResource(authSource, async (id) => {
    if (!id) return null;
    return api.getAuthResults(id);
  });

  return (
    <Show
      when={selectedId()}
      fallback={
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
      }
    >
      <Show
        when={message()}
        fallback={<div class="p-4 text-zinc-500 text-sm">Loading...</div>}
      >
        {(msg) => {
          const recipients = () => msg().recipients;
          const downloadableAttachments = () =>
            (attachments() ?? []).filter((a) => a.filename || !a.content_id);

          return (
            <div class="flex flex-col h-full">
              <div class="flex-shrink-0 border-b border-zinc-200 dark:border-zinc-800 px-4 py-3">
                <div class="flex items-start justify-between gap-2">
                  <div class="min-w-0">
                    <h2 class="text-base font-medium text-zinc-900 dark:text-zinc-100 truncate">
                      {msg().subject || "(no subject)"}
                    </h2>
                    <p class="text-sm text-zinc-500 dark:text-zinc-400 mt-0.5">
                      From:{" "}
                      <span class="text-zinc-700 dark:text-zinc-300">
                        {msg().sender}
                      </span>
                    </p>
                    <p class="text-sm text-zinc-500 dark:text-zinc-400">
                      To:{" "}
                      <span class="text-zinc-700 dark:text-zinc-300">
                        {recipients().join(", ")}
                      </span>
                    </p>
                    <p class="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
                      {formatDate(msg().created_at)} · {formatSize(msg().size)}
                    </p>
                  </div>
                  <div class="flex gap-1.5 flex-shrink-0">
                    {(() => {
                      const starred = () =>
                        messages().find((m) => m.id === msg().id)?.is_starred ??
                        false;
                      return (
                        <button
                          onClick={() => api.markStarred(msg().id, !starred())}
                          class="rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800 p-1.5 transition cursor-pointer"
                          classList={{
                            "text-amber-400 hover:text-amber-500": starred(),
                            "text-zinc-400 dark:text-zinc-500 hover:text-amber-400":
                              !starred(),
                          }}
                          title={starred() ? "Unstar" : "Star"}
                        >
                          <svg
                            class="size-4"
                            viewBox="0 0 24 24"
                            stroke="currentColor"
                            stroke-width="2"
                            fill={starred() ? "currentColor" : "none"}
                          >
                            <path
                              stroke-linecap="round"
                              stroke-linejoin="round"
                              d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"
                            />
                          </svg>
                        </button>
                      );
                    })()}
                    <a
                      href={api.exportUrl(msg().id, "eml")}
                      download={`${msg().id}.eml`}
                      class="rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800 p-1.5 text-zinc-500 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200 transition"
                      title="Download .eml"
                    >
                      <svg
                        class="size-4"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                        stroke-width="2"
                      >
                        <path
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5M16.5 12L12 16.5m0 0L7.5 12m4.5 4.5V3"
                        />
                      </svg>
                    </a>
                    <button
                      onClick={async () => {
                        const ok = await confirm({
                          title: "Delete message",
                          message: "This message will be permanently deleted.",
                          confirmLabel: "Delete",
                        });
                        if (ok) {
                          await api.deleteMessage(msg().id);
                          setSelectedId(null);
                        }
                      }}
                      class="btn-destructive rounded-md border p-1.5 transition cursor-pointer"
                      title="Delete"
                    >
                      <svg
                        class="size-4"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                        stroke-width="2"
                      >
                        <path
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0"
                        />
                      </svg>
                    </button>
                  </div>
                </div>
              </div>

              <TagEditor messageId={msg().id} />

              <Show when={downloadableAttachments().length > 0}>
                <div class="flex-shrink-0 border-b border-zinc-200 dark:border-zinc-800 px-4 py-2 flex flex-wrap gap-2">
                  <For each={downloadableAttachments()}>
                    {(att: Attachment) => (
                      <a
                        href={api.attachmentUrl(msg().id, att.id)}
                        download={att.filename || "attachment"}
                        class="inline-flex items-center gap-1.5 rounded-md border border-zinc-300 dark:border-zinc-700 bg-zinc-100 dark:bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition"
                      >
                        <svg
                          class="size-3.5"
                          fill="none"
                          viewBox="0 0 24 24"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            d="M18.375 12.739l-7.693 7.693a4.5 4.5 0 01-6.364-6.364l10.94-10.94A3 3 0 1119.5 7.372L8.552 18.32m.009-.01l-.01.01m5.699-9.941l-7.81 7.81a1.5 1.5 0 002.112 2.13"
                          />
                        </svg>
                        {att.filename || "attachment"}
                        <Show when={att.size}>
                          <span class="text-zinc-400 dark:text-zinc-500">
                            ({formatSize(att.size!)})
                          </span>
                        </Show>
                      </a>
                    )}
                  </For>
                </div>
              </Show>

              <div class="flex-shrink-0 border-b border-zinc-200 dark:border-zinc-800 px-4 flex gap-0">
                {(Object.entries(TAB_LABELS) as [Tab, string][]).map(
                  ([t, label]) => (
                    <button
                      onClick={() => setTab(t)}
                      class={`px-3 py-2 text-xs font-medium border-b-2 transition cursor-pointer ${
                        tab() === t
                          ? "border-orange-500 text-zinc-900 dark:text-zinc-100"
                          : "border-transparent text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
                      }`}
                    >
                      {label}
                    </button>
                  ),
                )}
              </div>

              <div class="flex-1 overflow-auto max-w-4xl mx-auto w-full">
                <Switch>
                  <Match when={tab() === "html"}>
                    <HtmlPreview
                      html={msg().html_body}
                      text={msg().text_body}
                      messageId={msg().id}
                    />
                  </Match>
                  <Match when={tab() === "text"}>
                    <pre class="p-4 text-sm text-zinc-700 dark:text-zinc-300 whitespace-pre-wrap font-mono">
                      {msg().text_body || "(no text body)"}
                    </pre>
                  </Match>
                  <Match when={tab() === "headers"}>
                    <HeadersView raw={rawSource()} />
                  </Match>
                  <Match when={tab() === "auth"}>
                    <AuthView results={authResults()} />
                  </Match>
                  <Match when={tab() === "raw"}>
                    <RawView raw={rawSource()} />
                  </Match>
                </Switch>
              </div>
            </div>
          );
        }}
      </Show>
    </Show>
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
  return `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src ${origin} data:;">`;
}

function HtmlPreview(props: {
  html: string | null;
  text: string | null;
  messageId: string;
}) {
  const safeHtml = () => {
    if (!props.html) return null;
    const rewritten = rewriteCidUrls(props.html, props.messageId);
    return cspMeta() + rewriteToAbsoluteUrls(rewritten);
  };

  return (
    <Show
      when={safeHtml()}
      fallback={
        <pre class="p-4 text-sm text-zinc-700 dark:text-zinc-300 whitespace-pre-wrap">
          {props.text || "(no content)"}
        </pre>
      }
    >
      {(html) => (
        <iframe
          sandbox=""
          srcdoc={html()}
          class="w-full h-full border-0 bg-white"
          title="Email HTML preview"
        />
      )}
    </Show>
  );
}

function parseHeaders(raw: string): { name: string; value: string }[] {
  const headerSection = raw.split(/\r?\n\r?\n/)[0] || "";
  const headers: { name: string; value: string }[] = [];

  for (const line of headerSection.split(/\r?\n/)) {
    if (line.startsWith(" ") || line.startsWith("\t")) {
      if (headers.length > 0) {
        headers[headers.length - 1].value += " " + line.trim();
      }
    } else {
      const colonIdx = line.indexOf(":");
      if (colonIdx > 0) {
        headers.push({
          name: line.substring(0, colonIdx).trim(),
          value: line.substring(colonIdx + 1).trim(),
        });
      }
    }
  }

  return headers;
}

function HeadersView(props: { raw: string | null | undefined }) {
  return (
    <Show
      when={props.raw}
      fallback={<div class="p-4 text-sm text-zinc-500">Loading...</div>}
    >
      {(raw) => {
        const headers = () => parseHeaders(raw());
        return (
          <div class="p-4">
            <table class="w-full text-sm">
              <tbody>
                <For each={headers()}>
                  {(h) => (
                    <tr class="border-b border-zinc-200/50 dark:border-zinc-800/50">
                      <td class="py-1.5 pr-4 text-zinc-500 dark:text-zinc-400 font-mono text-xs whitespace-nowrap align-top font-medium">
                        {h.name}
                      </td>
                      <td class="py-1.5 text-zinc-700 dark:text-zinc-300 font-mono text-xs break-all">
                        {h.value}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        );
      }}
    </Show>
  );
}

function StatusBadge(props: { status: string }) {
  const normalized = () => props.status.toLowerCase().replace(/^arc:/, "");
  const color = () => {
    switch (normalized()) {
      case "pass":
        return "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300";
      case "fail":
      case "hardfail":
        return "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300";
      case "softfail":
        return "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300";
      case "neutral":
      case "temperror":
      case "permerror":
        return "bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300";
      case "none":
        return "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400";
      case "info":
        return "bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300";
      default:
        return "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400";
    }
  };

  return (
    <span
      class={`inline-block px-2 py-0.5 rounded text-xs font-semibold uppercase ${color()}`}
    >
      {props.status}
    </span>
  );
}

function AuthSection(props: { title: string; checks: AuthCheck[] }) {
  return (
    <div class="mb-6">
      <h3 class="text-sm font-semibold text-zinc-700 dark:text-zinc-300 mb-2">
        {props.title}
      </h3>
      <Show
        when={props.checks.length > 0}
        fallback={
          <p class="text-xs text-zinc-400 dark:text-zinc-600 italic">
            No {props.title.toLowerCase()} headers found
          </p>
        }
      >
        <div class="space-y-2">
          <For each={props.checks}>
            {(check) => (
              <div class="flex items-start gap-3 rounded-md border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-900/50 px-3 py-2">
                <StatusBadge status={check.status} />
                <span class="text-xs text-zinc-600 dark:text-zinc-400 font-mono break-all leading-relaxed">
                  {check.details}
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

function AuthView(props: { results: AuthResults | null | undefined }) {
  return (
    <Show
      when={props.results}
      fallback={<div class="p-4 text-sm text-zinc-500">Loading...</div>}
    >
      {(r) => {
        const isEmpty = () =>
          r().dkim.length === 0 &&
          r().spf.length === 0 &&
          r().dmarc.length === 0 &&
          r().arc.length === 0;

        return (
          <Show
            when={!isEmpty()}
            fallback={
              <div class="flex flex-col items-center justify-center h-48 text-zinc-400 dark:text-zinc-600">
                <svg
                  class="size-8 mb-2 opacity-40"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  stroke-width="1.5"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z"
                  />
                </svg>
                <p class="text-sm">No authentication headers found</p>
                <p class="text-xs mt-1">
                  DKIM, SPF, and DMARC headers are typically added by receiving
                  mail servers
                </p>
              </div>
            }
          >
            <div class="p-4">
              <AuthSection title="DKIM" checks={r().dkim} />
              <AuthSection title="SPF" checks={r().spf} />
              <AuthSection title="DMARC" checks={r().dmarc} />
              <Show when={r().arc.length > 0}>
                <AuthSection title="ARC" checks={r().arc} />
              </Show>
            </div>
          </Show>
        );
      }}
    </Show>
  );
}

function TagEditor(props: { messageId: string }) {
  const [input, setInput] = createSignal("");
  const tags = () =>
    messages().find((m) => m.id === props.messageId)?.tags ?? [];

  async function addTag(value: string) {
    const tag = value.trim().toLowerCase();
    if (!tag || tags().includes(tag)) return;
    try {
      await api.setTags(props.messageId, [...tags(), tag]);
      setInput("");
    } catch {
      // WS event won't arrive; UI stays unchanged
    }
  }

  async function removeTag(tag: string) {
    try {
      await api.setTags(
        props.messageId,
        tags().filter((t) => t !== tag),
      );
    } catch {
      // WS event won't arrive; UI stays unchanged
    }
  }

  return (
    <div class="flex-shrink-0 border-b border-zinc-200 dark:border-zinc-800 px-4 py-2 flex items-center gap-2 flex-wrap">
      <svg
        class="size-3.5 text-zinc-400 dark:text-zinc-600 flex-shrink-0"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
        stroke-width="2"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M9.568 3H5.25A2.25 2.25 0 003 5.25v4.318c0 .597.237 1.17.659 1.591l9.581 9.581c.699.699 1.78.872 2.607.33a18.095 18.095 0 005.223-5.223c.542-.827.369-1.908-.33-2.607L11.16 3.66A2.25 2.25 0 009.568 3z"
        />
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M6 6h.008v.008H6V6z"
        />
      </svg>
      <For each={tags()}>
        {(tag) => (
          <span class="inline-flex items-center gap-1 rounded-md bg-orange-100 dark:bg-orange-900/40 px-2 py-0.5 text-xs font-medium text-orange-700 dark:text-orange-300">
            {tag}
            <button
              onClick={() => removeTag(tag)}
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
        class="bg-transparent text-xs text-zinc-700 dark:text-zinc-300 placeholder-zinc-400 dark:placeholder-zinc-600 outline-none min-w-[80px] flex-1"
      />
    </div>
  );
}

function RawView(props: { raw: string | null | undefined }) {
  return (
    <Show
      when={props.raw}
      fallback={<div class="p-4 text-sm text-zinc-500">Loading...</div>}
    >
      {(raw) => (
        <pre class="p-4 text-xs text-zinc-600 dark:text-zinc-400 whitespace-pre-wrap font-mono leading-relaxed">
          {raw()}
        </pre>
      )}
    </Show>
  );
}
