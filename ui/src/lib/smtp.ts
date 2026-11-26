/**
 * The host an SMTP client should be given for the page's own `hostname`.
 *
 * `location.hostname` keeps the brackets around an IPv6 literal, which SMTP
 * client settings take bare.
 */
export function smtpHost(hostname: string): string {
  return hostname.startsWith("[") && hostname.endsWith("]")
    ? hostname.slice(1, -1)
    : hostname;
}

/** `host:port`, with an IPv6 host bracketed so the port stays unambiguous. */
export function smtpAddress(host: string, port: number): string {
  return host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`;
}

/** The stacks the onboarding has a config snippet for. */
export type SnippetStack = "node" | "django" | "laravel" | "rails";

/** A config snippet for one stack, pointed at a given SMTP host and port. */
export interface SmtpSnippet {
  stack: SnippetStack;
  label: string;
  file: string;
  code: string;
}

/**
 * Plain SMTP settings for common stacks, pointed at `host` and `port`.
 *
 * They carry no TLS or credential settings: the server does not report
 * whether STARTTLS is on, and it accepts mail without authentication.
 */
export function smtpSnippets(host: string, port: number): SmtpSnippet[] {
  const quoted = JSON.stringify(host);
  return [
    {
      stack: "node",
      label: "Node.js",
      file: "nodemailer",
      code: [
        "const transporter = nodemailer.createTransport({",
        `  host: ${quoted},`,
        `  port: ${port},`,
        "});",
      ].join("\n"),
    },
    {
      stack: "django",
      label: "Django",
      file: "settings.py",
      code: [`EMAIL_HOST = ${quoted}`, `EMAIL_PORT = ${port}`].join("\n"),
    },
    {
      stack: "laravel",
      label: "Laravel",
      file: ".env",
      code: ["MAIL_MAILER=smtp", `MAIL_HOST=${host}`, `MAIL_PORT=${port}`].join(
        "\n",
      ),
    },
    {
      stack: "rails",
      label: "Rails",
      file: "config/environments/development.rb",
      code: [
        "config.action_mailer.delivery_method = :smtp",
        "config.action_mailer.smtp_settings = {",
        `  address: ${quoted},`,
        `  port: ${port},`,
        "}",
      ].join("\n"),
    },
  ];
}
