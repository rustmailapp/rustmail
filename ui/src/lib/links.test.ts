import { describe, expect, it } from "vitest";
import { extractLinks, linksFrom, type RawAnchor } from "./links";

function anchor(href: string, text = ""): RawAnchor {
  return { href, text };
}

function hrefs(anchors: RawAnchor[], text: string | null = null): string[] {
  return linksFrom(anchors, text).map((link) => link.href);
}

describe("linksFrom", () => {
  it("keeps each link once, in first-seen order, with its count", () => {
    const links = linksFrom(
      [
        anchor("https://example.com/b"),
        anchor("https://example.com/a"),
        anchor("https://EXAMPLE.com/b"),
      ],
      null,
    );

    expect(links.map((link) => [link.href, link.count])).toEqual([
      ["https://example.com/b", 2],
      ["https://example.com/a", 1],
    ]);
  });

  it("splits a link into host, short path and collapsed anchor text", () => {
    const [link] = linksFrom(
      [
        anchor(
          "https://example.com:8443/reset?token=abc#top",
          "  Reset\n   your\tpassword ",
        ),
      ],
      null,
    );

    expect(link).toMatchObject({
      scheme: "https",
      host: "example.com:8443",
      path: "/reset?token=abc#top",
      text: "Reset your password",
    });
  });

  it("leaves the path empty for a bare host and shortens a long one", () => {
    const long = `/${"a".repeat(80)}`;
    const [bare, shortened] = linksFrom(
      [anchor("https://example.com"), anchor(`https://example.com${long}`)],
      null,
    );

    expect(bare.path).toBe("");
    expect(shortened.path.length).toBeLessThan(long.length);
    expect(shortened.path.endsWith("…")).toBe(true);
  });

  it("finds bare http(s) URLs in the text body without trailing punctuation", () => {
    const links = linksFrom(
      [],
      "Confirm at https://example.com/confirm. Or see <http://docs.example.com/faq>, then ftp://example.com/x",
    );

    expect(links.map((link) => [link.href, link.text])).toEqual([
      ["https://example.com/confirm", ""],
      ["http://docs.example.com/faq", ""],
    ]);
  });

  it("keeps balanced parentheses and drops the one closing a sentence", () => {
    const links = linksFrom(
      [],
      "See https://en.wikipedia.org/wiki/Foo_(bar). (Or https://example.com/docs) and [https://example.com/x].",
    );

    expect(links.map((link) => link.href)).toEqual([
      "https://en.wikipedia.org/wiki/Foo_(bar)",
      "https://example.com/docs",
      "https://example.com/x",
    ]);
  });

  it("counts a link in both bodies by the body that holds it most", () => {
    const [link] = linksFrom(
      [anchor("https://example.com/", "Home"), anchor("https://example.com/")],
      "Home: https://example.com/",
    );

    expect(link.count).toBe(2);
    expect(link.text).toBe("Home");
  });

  it("takes the anchor text from the first anchor that has some", () => {
    const [link] = linksFrom(
      [anchor("https://example.com/"), anchor("https://example.com/", "Home")],
      null,
    );

    expect(link.text).toBe("Home");
  });

  it("skips fragment-only and script hrefs", () => {
    expect(
      hrefs([
        anchor("#top"),
        anchor("javascript:alert(1)"),
        anchor(" JavaScript:alert(1)"),
        anchor("java\nscript:alert(1)"),
        anchor("vbscript:msgbox(1)"),
        anchor("data:text/html,<p>hi</p>"),
      ]),
    ).toEqual([]);
  });

  it("skips schemes that hand the click to something other than a page", () => {
    expect(
      hrefs([
        anchor("file:///etc/passwd"),
        anchor("blob:https://example.com/uuid"),
        anchor("ms-msdt:/id PCWDiagnostic"),
        anchor("search-ms:query=x"),
        anchor("ftp://example.com/file"),
      ]),
    ).toEqual([]);
  });

  it("keeps mailto and tel links under their own scheme", () => {
    const links = linksFrom(
      [
        anchor("mailto:support@example.com?subject=Hi"),
        anchor("tel:+15551234567"),
      ],
      null,
    );

    expect(
      links.map(({ scheme, host, path }) => ({ scheme, host, path })),
    ).toEqual([
      { scheme: "mailto", host: "support@example.com", path: "?subject=Hi" },
      { scheme: "tel", host: "+15551234567", path: "" },
    ]);
    expect(links.every((link) => !link.insecure && !link.local)).toBe(true);
  });

  it("flags plain http as insecure", () => {
    const [plain, secure] = linksFrom(
      [anchor("http://example.com/"), anchor("https://example.com/")],
      null,
    );

    expect(plain.insecure).toBe(true);
    expect(secure.insecure).toBe(false);
  });

  it.each([
    "http://localhost:3000/",
    "https://127.0.0.1/",
    "http://127.10.0.2:8080/x",
    "http://[::1]:5173/",
    "https://printer.local/",
    "https://app.test/",
    "https://api.localhost/",
  ])("flags %s as local", (href) => {
    expect(linksFrom([anchor(href)], null)[0].local).toBe(true);
  });

  it.each([
    "https://example.com/",
    "https://testing.example.com/",
    "https://localhost.example.com/",
    "https://128.0.0.1/",
  ])("does not flag %s as local", (href) => {
    expect(linksFrom([anchor(href)], null)[0].local).toBe(false);
  });

  it("drops malformed and relative hrefs without throwing", () => {
    expect(
      hrefs([
        anchor(""),
        anchor("   "),
        anchor("http://"),
        anchor("https://exa mple.com/"),
        anchor("/relative/path"),
        anchor("%%%"),
        anchor("https://example.com/ok"),
      ]),
    ).toEqual(["https://example.com/ok"]);
  });
});

describe("extractLinks", () => {
  it("reads the text body alone when there is no HTML", () => {
    expect(
      extractLinks(null, "See https://example.com/a").map((l) => l.href),
    ).toEqual(["https://example.com/a"]);
  });

  it("finds nothing in empty bodies", () => {
    expect(extractLinks(null, null)).toEqual([]);
    expect(extractLinks("", "")).toEqual([]);
  });
});
