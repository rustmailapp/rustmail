import { describe, expect, it } from "vitest";
import { smtpAddress, smtpHost, smtpSnippets } from "./smtp";

describe("smtpHost", () => {
  it("keeps a name or IPv4 address as it is", () => {
    expect(smtpHost("localhost")).toBe("localhost");
    expect(smtpHost("127.0.0.1")).toBe("127.0.0.1");
  });

  it("strips the brackets from an IPv6 literal", () => {
    expect(smtpHost("[::1]")).toBe("::1");
    expect(smtpHost("[fe80::1]")).toBe("fe80::1");
  });
});

describe("smtpAddress", () => {
  it("joins a name or IPv4 host and the port", () => {
    expect(smtpAddress("127.0.0.1", 1025)).toBe("127.0.0.1:1025");
    expect(smtpAddress("mail.test", 2525)).toBe("mail.test:2525");
  });

  it("brackets an IPv6 host so the port stays apart", () => {
    expect(smtpAddress("::1", 1025)).toBe("[::1]:1025");
  });
});

describe("smtpSnippets", () => {
  it("fills every snippet with the host and port", () => {
    for (const snippet of smtpSnippets("::1", 2525)) {
      expect(snippet.code).toContain("::1");
      expect(snippet.code).not.toContain("[::1]");
      expect(snippet.code).toContain("2525");
    }
  });

  it("sets no TLS or credential options", () => {
    for (const snippet of smtpSnippets("127.0.0.1", 1025)) {
      expect(snippet.code).not.toMatch(/tls|ssl|secure|user|pass|auth/i);
    }
  });
});
