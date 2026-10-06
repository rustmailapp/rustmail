/** A link a message points at, once per distinct target. */
export interface MessageLink {
  /** The normalised target, which is also what makes two links the same. */
  href: string;
  /** The URL scheme without its colon: `https`, `mailto`, `tel`... */
  scheme: string;
  /** The host with its port, or the address for `mailto:` and `tel:`. */
  host: string;
  /** What follows the host, shortened for display; empty for a bare host. */
  path: string;
  /** The text of the first anchor that has some; empty for text-body links. */
  text: string;
  /** How often the link appears in whichever body holds it most. */
  count: number;
  /** Plain http, which a mail client may warn about or refuse. */
  insecure: boolean;
  /** Points at loopback or a `.local`, `.test` or `.localhost` name. */
  local: boolean;
}

/** An `a[href]` as read from the HTML: its raw attribute and its text. */
export interface RawAnchor {
  href: string;
  text: string;
}

/**
 * The schemes worth listing. Anything else in an untrusted email may hand the
 * click to a script, the file system or an external protocol handler.
 */
const LISTED_SCHEMES = new Set(["http", "https", "mailto", "tel"]);
const TEXT_URL =
  /\bhttps?:\/\/(?:\[[\da-f:.]+\]|[^\s<>"'`[\]{}])[^\s<>"'`[\]{}]*/gi;
const TRAILING_PUNCTUATION = /[.,;:!?]$/;
const LOOPBACK_V4 = /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/;
const LOCAL_HOSTS = new Set(["localhost", "[::1]"]);
const LOCAL_SUFFIXES = [".localhost", ".local", ".test"];
const MAX_PATH_CHARS = 48;

/**
 * The links in a message, unique, in the order they first appear.
 *
 * The HTML is parsed into an inert document that is only read: `DOMParser`
 * neither runs its scripts nor loads its resources, and it is never attached
 * to the page.
 */
export function extractLinks(
  html: string | null,
  text: string | null,
): MessageLink[] {
  return linksFrom(html ? anchorsIn(html) : [], text);
}

function anchorsIn(html: string): RawAnchor[] {
  const doc = new DOMParser().parseFromString(html, "text/html");
  return Array.from(doc.querySelectorAll("a[href]"), (anchor) => ({
    href: anchor.getAttribute("href") ?? "",
    text: anchor.textContent ?? "",
  }));
}

/**
 * The links among `anchors` and the bare http(s) URLs in `text`.
 *
 * The text body is usually the same message as the HTML, so a link's count is
 * the most times it appears in either body rather than their sum.
 */
export function linksFrom(
  anchors: readonly RawAnchor[],
  text: string | null,
): MessageLink[] {
  const found = new Map<string, MessageLink>();
  const tally = (sources: readonly RawAnchor[]): Map<string, number> => {
    const counts = new Map<string, number>();
    for (const source of sources) {
      const url = parseTarget(source.href);
      if (!url) continue;
      counts.set(url.href, (counts.get(url.href) ?? 0) + 1);
      const label = collapseWhitespace(source.text);
      const known = found.get(url.href);
      if (!known) found.set(url.href, describe(url, label));
      else if (!known.text) known.text = label;
    }
    return counts;
  };
  const inHtml = tally(anchors);
  const inText = tally(urlsIn(text).map((href) => ({ href, text: "" })));
  return Array.from(found.values(), (link) => ({
    ...link,
    count: Math.max(inHtml.get(link.href) ?? 0, inText.get(link.href) ?? 0),
  }));
}

function urlsIn(text: string | null): string[] {
  if (!text) return [];
  return Array.from(text.matchAll(TEXT_URL), ([url]) => trimTrailing(url));
}

/** Drops the punctuation and unbalanced `)` that close the sentence around a URL. */
function trimTrailing(url: string): string {
  let end = url.length;
  while (end > 0) {
    const last = url[end - 1];
    const unbalanced =
      last === ")" && countOf(url, "(", end) < countOf(url, ")", end);
    if (!TRAILING_PUNCTUATION.test(last) && !unbalanced) break;
    end -= 1;
  }
  return url.slice(0, end);
}

function countOf(text: string, char: string, end: number): number {
  let count = 0;
  for (let i = 0; i < end; i += 1) if (text[i] === char) count += 1;
  return count;
}

/** The absolute URL `href` points at, or `null` for one not worth listing. */
function parseTarget(href: string): URL | null {
  const trimmed = href.trim();
  if (!trimmed || trimmed.startsWith("#") || !URL.canParse(trimmed)) {
    return null;
  }
  const url = new URL(trimmed);
  return LISTED_SCHEMES.has(schemeOf(url)) ? url : null;
}

function schemeOf(url: URL): string {
  return url.protocol.slice(0, -1);
}

function describe(url: URL, text: string): MessageLink {
  const hasHost = url.host !== "";
  const rest = hasHost
    ? (url.pathname === "/" ? "" : url.pathname) + url.search + url.hash
    : url.search;
  return {
    href: url.href,
    scheme: schemeOf(url),
    host: hasHost ? url.host : url.pathname,
    path: shorten(rest),
    text,
    count: 0,
    insecure: url.protocol === "http:",
    local: hasHost && isLocalHost(url.hostname),
  };
}

function isLocalHost(hostname: string): boolean {
  return (
    LOCAL_HOSTS.has(hostname) ||
    LOOPBACK_V4.test(hostname) ||
    LOCAL_SUFFIXES.some((suffix) => hostname.endsWith(suffix))
  );
}

function shorten(path: string): string {
  return path.length > MAX_PATH_CHARS
    ? `${path.slice(0, MAX_PATH_CHARS - 1)}…`
    : path;
}

function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}
