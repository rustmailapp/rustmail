#!/usr/bin/env python3
"""Send test emails to the local RustMail SMTP server.

Default mode sends a batch of realistic filler emails plus the branded
RustMail welcome email (sent last so it appears first in the inbox).

Use --single to send only the branded welcome email.
"""

import argparse
import json
import smtplib
import time
import urllib.request
from email.mime.application import MIMEApplication
from email.mime.image import MIMEImage
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from pathlib import Path
from string import Template

LOGO_PATH = Path(__file__).resolve().parent.parent / "ui" / "public" / "logo.webp"

# ---------------------------------------------------------------------------
# Filler emails — sent first (oldest) so they stack below the hero email
# ---------------------------------------------------------------------------

BATCH_EMAILS = [
    {
        "sender": "GitHub <notifications@github.com>",
        "to": "davide@example.com",
        "subject": "[rustmail/rustmail] New issue: Add ARM64 Docker image (#47)",
        "body": "A new issue was opened by @contributor in rustmail/rustmail.\n\n#47 Add ARM64 Docker image\n\nIt would be great to have multi-arch Docker images for ARM64 runners.",
        "tags": ["github"],
        "is_starred": False,
        "is_read": True,
    },
    {
        "sender": "Amazon Web Services <no-reply@aws.amazon.com>",
        "to": "davide@example.com",
        "subject": "Your AWS billing summary for March 2026",
        "body": "Your AWS account charges for March 2026 are $12.47.\n\nPlease see the attached billing summary.",
        "tags": ["billing"],
        "is_starred": False,
        "is_read": True,
        "attachment": {"filename": "invoice-march-2026.pdf", "content_type": "application/pdf"},
    },
    {
        "sender": "Stripe <receipts@stripe.com>",
        "to": "davide@example.com",
        "subject": "Receipt for your payment of $49.00",
        "body": "Hi Davide,\n\nWe received your payment of $49.00 for your Stripe subscription.\n\nInvoice ID: INV-2026-0319\nDate: March 19, 2026\nAmount: $49.00\n\nThank you for your business.",
        "tags": ["receipt"],
        "is_starred": False,
        "is_read": True,
    },
    {
        "sender": "Cloudflare <noreply@cloudflare.com>",
        "to": "davide@example.com",
        "subject": "SSL certificate expiring: rustmail.app",
        "body": "Your SSL/TLS certificate for rustmail.app is expiring soon.\n\nDomain: rustmail.app\nExpires: April 2, 2026\nIssuer: Cloudflare Inc\n\nRenew or replace the certificate to avoid service disruption.",
        "tags": ["alert"],
        "is_starred": True,
        "is_read": False,
    },
    {
        "sender": "Grafana <alerts@grafana.internal>",
        "to": "davide@example.com",
        "subject": "[FIRING] High API latency p99 > 500ms",
        "body": "Alert: High API latency p99 > 500ms\nStatus: FIRING\n\nLabels:\n  service = rustmail-api\n  severity = warning\n  instance = prod-01\n\nValue: 623ms\nDashboard: API Overview\nSilence: https://grafana.internal/alerting/silence/new",
        "tags": ["grafana", "alert"],
        "is_starred": False,
        "is_read": True,
    },
    {
        "sender": "Docker Hub <noreply@docker.com>",
        "to": "davide@example.com",
        "subject": "Image push succeeded: rustmail/rustmail:latest",
        "body": "Your Docker image has been successfully pushed.\n\nRepository: rustmail/rustmail\nTag: latest\nDigest: sha256:a1b2c3d4e5f6...\nSize: 24.3 MB\nPushed at: 2026-03-24T10:32:00Z",
        "tags": ["ci"],
        "is_starred": False,
        "is_read": True,
    },
    {
        "sender": "RadonForge <noreply@radonforge.com>",
        "to": "davide@example.com",
        "subject": "Something hot's coming...",
        "body": "Stay tuned. We're cooking something big.\n\n-- The RadonForge Team",
        "tags": [],
        "is_starred": False,
        "is_read": False,
    },
    {
        "sender": "RustMail <noreply@rustmail.app>",
        "to": "you@example.com",
        "subject": "Welcome to RustMail",
        "body": None,
        "tags": [],
        "is_starred": True,
        "is_read": False,
        "welcome": True,
    },
]

# ---------------------------------------------------------------------------
# Branded welcome email (HTML + inline logo)
# ---------------------------------------------------------------------------

WELCOME_HTML = """\
<!DOCTYPE html>
<html lang="en" xmlns:v="urn:schemas-microsoft-com:vml">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Welcome to RustMail</title>
  <style>
    * { margin: 0; padding: 0; }
    body { -webkit-text-size-adjust: 100%; -ms-text-size-adjust: 100%; }
    table { border-collapse: collapse; }
    img { border: 0; display: block; }
    @media screen and (max-width: 480px) {
      .email-inner { padding: 32px 24px !important; }
    }
  </style>
</head>
<body style="margin: 0; padding: 0; word-spacing: normal; background-color: #0c0c0c;">
  <div role="article" aria-roledescription="email" aria-label="Welcome to RustMail" lang="en"
       style="font-size: 16px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;">
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background-color: #0c0c0c;">
      <tr>
        <td align="center" style="padding: 48px 16px;">
          <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width: 480px;">

            <!-- Logo -->
            <tr>
              <td align="center" style="padding-bottom: 36px;">
                <img src="cid:rustmail-logo" alt="RustMail" width="64" height="64" style="width: 64px; height: 64px; border-radius: 16px;" />
              </td>
            </tr>

            <!-- Card -->
            <tr>
              <td style="padding: 0;">
                <div style="border-radius: 20px; border: 1px solid #262626; overflow: hidden; padding: 40px 40px 36px 40px;">

                  <!-- Heading -->
                  <h1 style="margin: 0 0 10px 0; font-size: 22px; font-weight: 600; line-height: 1.3; color: #ededed; letter-spacing: -0.02em;">
                    Welcome to RustMail
                  </h1>

                  <!-- Body -->
                  <p style="margin: 0 0 24px 0; font-size: 15px; line-height: 1.6; color: #a3a3a3;">
                    Your local dev mail server is up and running. This is a test email to confirm
                    everything works — SMTP capture, the REST API, and your Neovim plugin.
                  </p>

                  <!-- Feature list -->
                  <table role="presentation" cellpadding="0" cellspacing="0" style="margin: 0 0 28px 0; width: 100%;">
                    <tr>
                      <td style="padding: 12px 16px; border-radius: 10px; background-color: #1a1a1a;">
                        <table role="presentation" cellpadding="0" cellspacing="0" width="100%">
                          <tr>
                            <td style="padding: 0 0 8px 0; font-size: 13px; color: #737373;">SMTP</td>
                            <td align="right" style="padding: 0 0 8px 0; font-size: 13px; color: #ededed; font-family: 'SF Mono', Menlo, monospace;">:$smtp_port</td>
                          </tr>
                          <tr>
                            <td style="padding: 0 0 8px 0; font-size: 13px; color: #737373;">HTTP</td>
                            <td align="right" style="padding: 0 0 8px 0; font-size: 13px; color: #ededed; font-family: 'SF Mono', Menlo, monospace;">:$http_port</td>
                          </tr>
                          <tr>
                            <td style="padding: 0; font-size: 13px; color: #737373;">Status</td>
                            <td align="right" style="padding: 0; font-size: 13px; color: #34d399;">&#10004; Capturing</td>
                          </tr>
                        </table>
                      </td>
                    </tr>
                  </table>

                  <!-- CTA -->
                  <table role="presentation" cellpadding="0" cellspacing="0" style="margin: 0 0 28px 0;">
                    <tr>
                      <td style="padding: 0;">
                        <div style="border-radius: 10px; background-color: #c45a2d; overflow: hidden; display: inline-block;">
                          <a href="http://127.0.0.1:$http_port" target="_blank"
                             style="display: inline-block; padding: 12px 28px; font-size: 14px; font-weight: 600; color: #ffffff; text-decoration: none; line-height: 1.2;">
                            Open Web UI
                          </a>
                        </div>
                      </td>
                    </tr>
                  </table>

                  <!-- Divider -->
                  <hr style="border: none; border-top: 1px solid #262626; margin: 0 0 20px 0;">

                  <!-- Tip -->
                  <p style="margin: 0; font-size: 13px; line-height: 1.5; color: #525252;">
                    Open <span style="color: #a3a3a3; font-family: 'SF Mono', Menlo, monospace;">:RustMail</span> in Neovim to browse captured emails without leaving your editor.
                  </p>

                </div>
              </td>
            </tr>

            <!-- Footer -->
            <tr>
              <td align="center" style="padding-top: 28px;">
                <p style="margin: 0; font-size: 12px; line-height: 1.5; color: #404040;">
                  This email was sent by a local RustMail instance for development purposes.
                </p>
                <p style="margin: 10px 0 0 0; font-size: 11px; line-height: 1.5; color: #404040;">
                  RustMail &mdash; dev email server, built with Rust
                </p>
              </td>
            </tr>

          </table>
        </td>
      </tr>
    </table>
  </div>
</body>
</html>
"""

WELCOME_PLAIN = """\
Welcome to RustMail

Your local dev mail server is up and running.
This is a test email to confirm everything works.

  SMTP:   :$smtp_port
  HTTP:   :$http_port
  Status: Capturing

Open :RustMail in Neovim to browse captured emails.
"""


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def api_get(base_url: str, path: str):
    req = urllib.request.Request(f"{base_url}{path}")
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read())


def api_patch(base_url: str, path: str, body: dict):
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        f"{base_url}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method="PATCH",
    )
    with urllib.request.urlopen(req) as resp:
        return resp.status


def send_plain(host: str, port: int, email: dict) -> None:
    msg = MIMEMultipart("mixed")
    msg.attach(MIMEText(email["body"], "plain"))
    msg["From"] = email["sender"]
    msg["To"] = email["to"]
    msg["Subject"] = email["subject"]

    if "attachment" in email:
        att = email["attachment"]
        fake = MIMEApplication(b"%PDF-1.4 fake attachment placeholder", _subtype="pdf")
        fake.add_header("Content-Disposition", "attachment", filename=att["filename"])
        msg.attach(fake)

    with smtplib.SMTP(host, port) as smtp:
        smtp.send_message(msg)


def send_welcome(host: str, port: int, http_port: int) -> None:
    html = Template(WELCOME_HTML).substitute(smtp_port=port, http_port=http_port)
    plain = Template(WELCOME_PLAIN).substitute(smtp_port=port, http_port=http_port)

    msg = MIMEMultipart("related")
    msg["From"] = "RustMail <noreply@rustmail.app>"
    msg["To"] = "you@example.com"
    msg["Subject"] = "Welcome to RustMail"

    body = MIMEMultipart("alternative")
    body.attach(MIMEText(plain, "plain"))
    body.attach(MIMEText(html, "html"))
    msg.attach(body)

    if not LOGO_PATH.is_file():
        raise SystemExit(f"logo not found: {LOGO_PATH}")

    logo = MIMEImage(LOGO_PATH.read_bytes(), _subtype="webp")
    logo.add_header("Content-ID", "<rustmail-logo>")
    logo.add_header("Content-Disposition", "inline", filename="logo.webp")
    msg.attach(logo)

    with smtplib.SMTP(host, port) as smtp:
        smtp.send_message(msg)


def apply_metadata(api_url: str) -> None:
    result = api_get(api_url, f"/messages?limit=50")
    messages = result.get("messages", [])

    subject_to_id = {}
    for m in messages:
        subj = m.get("subject", "") or ""
        if subj not in subject_to_id:
            subject_to_id[subj] = m["id"]

    for email in BATCH_EMAILS:
        msg_id = subject_to_id.get(email["subject"])
        if not msg_id:
            continue
        patch = {}
        if email.get("tags"):
            patch["tags"] = email["tags"]
        if email.get("is_read") is not None:
            patch["is_read"] = email["is_read"]
        if email.get("is_starred") is not None:
            patch["is_starred"] = email["is_starred"]
        if patch:
            api_patch(api_url, f"/messages/{msg_id}", patch)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def send_batch(host: str, port: int, http_port: int, api_url: str) -> None:
    total = len(BATCH_EMAILS)
    for i, email in enumerate(BATCH_EMAILS):
        if email.get("welcome"):
            send_welcome(host, port, http_port)
        else:
            send_plain(host, port, email)
        print(f"  [{i + 1}/{total}] {email['subject']}")
        time.sleep(0.15)

    time.sleep(0.3)
    print("applying tags, read/starred state...")
    try:
        apply_metadata(api_url)
    except Exception as exc:
        raise SystemExit(f"failed to apply metadata — is rustmail running? ({exc})") from exc
    print(f"done — {total} emails sent to {host}:{port}")


def send_single(host: str, port: int, http_port: int) -> None:
    send_welcome(host, port, http_port)
    print(f"sent to {host}:{port}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Send test emails to RustMail")
    parser.add_argument("--host", default="127.0.0.1", help="SMTP host (default: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=1025, help="SMTP port (default: 1025)")
    parser.add_argument("--http-port", type=int, default=8025, help="HTTP port shown in email (default: 8025)")
    parser.add_argument("--api-url", default=None, help="API base URL (default: http://127.0.0.1:<http-port>/api/v1)")
    parser.add_argument("--single", action="store_true", help="Send only the branded welcome email")
    args = parser.parse_args()

    api_url = args.api_url or f"http://127.0.0.1:{args.http_port}/api/v1"

    if args.single:
        send_single(args.host, args.port, args.http_port)
    else:
        send_batch(args.host, args.port, args.http_port, api_url)
