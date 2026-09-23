//! The deterministic MIME corpus every golden runs over.
//!
//! Each message records the decoded bytes it was built from, so a download
//! can be compared against exactly what the generator encoded. Payloads marked
//! [`Served::Exact`] must come back byte for byte; the others pin whatever the
//! parser makes of them today (charset conversion, lenient base64), and the
//! golden says whether that still equals the source.

use std::sync::OnceLock;

use crate::mime::{Prng, base64, quoted_printable_binary, wrap};

const CRLF: &str = "\r\n";
const LF: &str = "\n";
const BASE64_LINE: usize = 76;
const DATE: &str = "Date: Wed, 23 Sep 2026 09:00:00 +0000";
const MIME_VERSION: &str = "MIME-Version: 1.0";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const GIF_SIGNATURE: &[u8] = b"GIF89a";
const PDF_SIGNATURE: &[u8] = b"%PDF-1.7\n";
const BOB: &str = "bob@example.test";
const LARGE_ATTACHMENT_BYTES: usize = 7_000_000;
const LARGE_FILLER_BYTES: usize = 650_000;
const INVALID_UTF8: &[u8] = b"caf\xe9 \xff\xfe bytes \xc3\x28 end";

/// How a download of a payload must relate to its source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
  /// Served byte for byte as the generator encoded it.
  Exact,
  /// Served as the parser decodes it today, which the golden pins.
  Pinned,
}

/// Decoded bytes the generator put into one MIME part.
pub struct Payload {
  pub label: &'static str,
  pub bytes: Vec<u8>,
  pub served: Served,
}

/// The read, starred and tag state a message is given after it is stored.
#[derive(Default)]
pub struct MailState {
  pub read: bool,
  pub starred: bool,
  pub tags: &'static [&'static str],
}

/// One captured message: its envelope, its exact wire bytes, and the
/// payloads encoded inside them.
pub struct CorpusMessage {
  pub name: &'static str,
  pub sender: String,
  pub recipients: Vec<String>,
  pub raw: Vec<u8>,
  pub payloads: Vec<Payload>,
  pub state: MailState,
}

/// Every corpus message, in the order it is stored.
pub fn corpus() -> &'static [CorpusMessage] {
  static CORPUS: OnceLock<Vec<CorpusMessage>> = OnceLock::new();
  CORPUS.get_or_init(build)
}

fn build() -> Vec<CorpusMessage> {
  vec![
    welcome(),
    newsletter_html(),
    receipt_alternative(),
    latin1_subject(),
    invoice_reminder(),
    auth_headers(),
    folded_headers(),
    no_subject_no_content_type(),
    like_wildcards_subject(),
    headers_only(),
    base64_crlf76(),
    base64_unwrapped(),
    base64_lf_only(),
    base64_trailing_whitespace(),
    base64_stray_bytes(),
    quoted_printable_octet_stream(),
    binary_and_8bit(),
    text_attachments(),
    nested_related(),
    rfc822_plain(),
    rfc822_base64(),
    digest_without_content_types(),
    part_without_content_type(),
    base64_malformed(),
    large_10mib(),
  ]
}

struct Draft {
  name: &'static str,
  sender: &'static str,
  recipients: Vec<&'static str>,
  headers: Vec<String>,
  body: Vec<u8>,
  eol: &'static str,
  payloads: Vec<Payload>,
  state: MailState,
}

impl Draft {
  fn new(name: &'static str, sender: &'static str, subject: Option<&str>) -> Self {
    let mut headers = vec![format!("From: {sender}"), format!("To: {BOB}")];
    if let Some(subject) = subject {
      headers.push(format!("Subject: {subject}"));
    }
    headers.push(DATE.to_string());
    headers.push(format!("Message-ID: <{name}@corpus.rustmail.test>"));
    Self {
      name,
      sender,
      recipients: vec![BOB],
      headers,
      body: Vec::new(),
      eol: CRLF,
      payloads: Vec::new(),
      state: MailState::default(),
    }
  }

  fn header(mut self, line: impl Into<String>) -> Self {
    self.headers.push(line.into());
    self
  }

  fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
    self.body = body.into();
    self
  }

  fn line_endings(mut self, eol: &'static str) -> Self {
    self.eol = eol;
    self
  }

  fn recipients(mut self, recipients: Vec<&'static str>) -> Self {
    self.recipients = recipients;
    self
  }

  fn state(mut self, state: MailState) -> Self {
    self.state = state;
    self
  }

  fn payload(mut self, label: &'static str, bytes: Vec<u8>, served: Served) -> Self {
    self.payloads.push(Payload {
      label,
      bytes,
      served,
    });
    self
  }

  fn multipart(self, content_type: &str, boundary: &str, parts: Vec<Vec<u8>>) -> Self {
    let eol = self.eol;
    self
      .header(MIME_VERSION)
      .header(format!(
        "Content-Type: {content_type}; boundary=\"{boundary}\""
      ))
      .body(multipart_body(boundary, &parts, eol))
  }

  fn finish(self) -> CorpusMessage {
    CorpusMessage {
      name: self.name,
      sender: self.sender.to_string(),
      recipients: self.recipients.iter().map(ToString::to_string).collect(),
      raw: entity(&self.headers, &self.body, self.eol),
      payloads: self.payloads,
      state: self.state,
    }
  }
}

fn entity(headers: &[String], body: &[u8], eol: &str) -> Vec<u8> {
  let mut out = Vec::new();
  for header in headers {
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(eol.as_bytes());
  }
  out.extend_from_slice(eol.as_bytes());
  out.extend_from_slice(body);
  out
}

fn part(headers: &[&str], body: impl AsRef<[u8]>, eol: &str) -> Vec<u8> {
  let headers: Vec<String> = headers.iter().map(ToString::to_string).collect();
  entity(&headers, body.as_ref(), eol)
}

fn multipart_body(boundary: &str, parts: &[Vec<u8>], eol: &str) -> Vec<u8> {
  let mut out = Vec::new();
  for part in parts {
    out.extend_from_slice(format!("--{boundary}{eol}").as_bytes());
    out.extend_from_slice(part);
    out.extend_from_slice(eol.as_bytes());
  }
  out.extend_from_slice(format!("--{boundary}--{eol}").as_bytes());
  out
}

fn random(label: &str, len: usize) -> Vec<u8> {
  Prng::seeded(label).bytes(len)
}

fn with_signature(signature: &[u8], label: &str, len: usize) -> Vec<u8> {
  let mut bytes = signature.to_vec();
  bytes.extend(random(label, len));
  bytes
}

fn base64_lines(payload: &[u8], eol: &str) -> String {
  wrap(&base64(payload), BASE64_LINE, eol)
}

fn text_part(text: &str, eol: &str) -> Vec<u8> {
  part(&["Content-Type: text/plain; charset=utf-8"], text, eol)
}

fn welcome() -> CorpusMessage {
  Draft::new("welcome", "alice@example.com", Some("Welcome aboard"))
    .header("Content-Type: text/plain; charset=utf-8")
    .body("Hello Bob,\r\n\r\nwelcome to the quarterly planning group.\r\n\r\nRegards,\r\nAlice\r\n")
    .state(MailState {
      read: true,
      ..MailState::default()
    })
    .finish()
}

fn newsletter_html() -> CorpusMessage {
  Draft::new("newsletter_html", "news@example.org", Some("Weekly digest: Rust tips"))
    .header(MIME_VERSION)
    .header("Content-Type: text/html; charset=utf-8")
    .body("<html><body><h1>Weekly digest</h1><p>Rust tips and <b>tricks</b> for planning.</p></body></html>\r\n")
    .state(MailState {
      tags: &["newsletter"],
      ..MailState::default()
    })
    .finish()
}

fn receipt_alternative() -> CorpusMessage {
  Draft::new(
    "receipt_alternative",
    "shop@example.net",
    Some("Your receipt #4471"),
  )
  .recipients(vec!["carol@example.test", "dave@example.test"])
  .multipart(
    "multipart/alternative",
    "ALT-4471",
    vec![
      text_part("Receipt 4471: total 12.50 EUR.", CRLF),
      part(
        &["Content-Type: text/html; charset=utf-8"],
        "<p>Receipt <strong>4471</strong>: total 12.50 EUR.</p>",
        CRLF,
      ),
    ],
  )
  .state(MailState {
    read: true,
    starred: true,
    tags: &["work", "finance"],
  })
  .finish()
}

fn latin1_subject() -> CorpusMessage {
  Draft::new(
    "latin1_subject",
    "chef@example.fr",
    Some("=?ISO-8859-1?Q?Caf=E9_cr=E8me?="),
  )
  .header(MIME_VERSION)
  .header("Content-Type: text/plain; charset=iso-8859-1")
  .header("Content-Transfer-Encoding: quoted-printable")
  .body("Un caf=E9 cr=E8me pour la r=E9union.\r\n")
  .finish()
}

fn invoice_reminder() -> CorpusMessage {
  Draft::new(
    "invoice_reminder",
    "billing@example.com",
    Some("Invoice 2026-091 reminder"),
  )
  .header("Content-Type: text/plain; charset=utf-8")
  .body("Invoice 2026-091 is due. Planning budget attached separately.\r\n")
  .state(MailState {
    starred: true,
    tags: &["work", "urgent"],
    ..MailState::default()
  })
  .finish()
}

fn auth_headers() -> CorpusMessage {
  Draft::new("auth_headers", "sender@example.com", Some("Signed and sealed"))
    .header("Authentication-Results: mx.example.com; dkim=pass header.d=example.com header.s=sel1; spf=pass smtp.mailfrom=sender@example.com; dmarc=pass action=none header.from=example.com")
    .header("ARC-Authentication-Results: i=1; mx.example.com; dkim=pass header.d=example.com; spf=softfail smtp.mailfrom=relay@example.net")
    .header("DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=sel1; h=from:to:subject; bh=YWJj; b=ZGVm")
    .header("Received-SPF: Pass (mx.example.com: domain of sender@example.com designates 192.0.2.1 as permitted sender)")
    .header("Content-Type: text/plain")
    .body("Authenticated body.\r\n")
    .finish()
}

fn folded_headers() -> CorpusMessage {
  Draft::new("folded_headers", "relay@example.net", None)
    .header("Subject: A subject folded\r\n  across two lines")
    .header("Received: from a.example.net by b.example.net;\r\n\tWed, 23 Sep 2026 08:59:58 +0000")
    .header("Received: from c.example.net by a.example.net;\r\n\tWed, 23 Sep 2026 08:59:57 +0000")
    .header("X-Long-Header: first segment\r\n second segment\r\n\tthird segment")
    .header("Content-Type: text/plain")
    .body("Folded headers body.\r\n")
    .finish()
}

fn no_subject_no_content_type() -> CorpusMessage {
  Draft::new("no_subject_no_content_type", "quiet@example.com", None)
    .body("A message with neither a subject nor a content type.\r\n")
    .state(MailState {
      read: true,
      tags: &["personal"],
      ..MailState::default()
    })
    .finish()
}

fn like_wildcards_subject() -> CorpusMessage {
  Draft::new(
    "like_wildcards_subject",
    "promo@example.org",
    Some("Discount 100% off_now"),
  )
  .recipients(vec!["Carol@Example.Test"])
  .header("Content-Type: text/plain")
  .body("Percent and underscore in the subject.\r\n")
  .finish()
}

fn headers_only() -> CorpusMessage {
  Draft::new("headers_only", "empty@example.com", Some("Headers only")).finish()
}

fn attachment_draft(name: &'static str, subject: &str) -> Draft {
  Draft::new(name, "files@example.com", Some(subject))
}

fn base64_crlf76() -> CorpusMessage {
  let payload = with_signature(PDF_SIGNATURE, "base64_crlf76", 2_000);
  attachment_draft("base64_crlf76", "Base64 wrapped at 76 with CRLF")
    .multipart(
      "multipart/mixed",
      "B64-CRLF76",
      vec![
        text_part("Report attached.", CRLF),
        part(
          &[
            "Content-Type: application/pdf; name=\"report.pdf\"",
            "Content-Disposition: attachment; filename=\"report.pdf\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&payload, CRLF),
          CRLF,
        ),
      ],
    )
    .payload("report.pdf", payload, Served::Exact)
    .state(MailState {
      tags: &["work"],
      ..MailState::default()
    })
    .finish()
}

fn base64_unwrapped() -> CorpusMessage {
  let payload = random("base64_unwrapped", 1_500);
  attachment_draft("base64_unwrapped", "Base64 on a single line")
    .multipart(
      "multipart/mixed",
      "B64-ONELINE",
      vec![
        text_part("One long base64 line.", CRLF),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"oneline.bin\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64(&payload),
          CRLF,
        ),
      ],
    )
    .payload("oneline.bin", payload, Served::Exact)
    .finish()
}

fn base64_lf_only() -> CorpusMessage {
  let payload = random("base64_lf_only", 1_200);
  attachment_draft("base64_lf_only", "Base64 with LF-only line endings")
    .line_endings(LF)
    .multipart(
      "multipart/mixed",
      "B64-LF",
      vec![
        text_part("Unix line endings throughout.", LF),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"unix.bin\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&payload, LF),
          LF,
        ),
      ],
    )
    .payload("unix.bin", payload, Served::Exact)
    .state(MailState {
      starred: true,
      ..MailState::default()
    })
    .finish()
}

fn base64_trailing_whitespace() -> CorpusMessage {
  let payload = random("base64_trailing_whitespace", 1_100);
  let encoded = base64(&payload);
  let lines: Vec<String> = encoded
    .as_bytes()
    .chunks(BASE64_LINE)
    .enumerate()
    .map(|(index, line)| {
      let padding = if index % 2 == 0 { "  " } else { "\t" };
      format!("{}{padding}", String::from_utf8_lossy(line))
    })
    .collect();
  attachment_draft(
    "base64_trailing_whitespace",
    "Base64 with trailing whitespace",
  )
  .multipart(
    "multipart/mixed",
    "B64-TRAILING",
    vec![
      text_part("Every base64 line ends in blanks.", CRLF),
      part(
        &[
          "Content-Type: application/octet-stream",
          "Content-Disposition: attachment; filename=\"padded.bin\"",
          "Content-Transfer-Encoding: base64",
        ],
        lines.join(CRLF),
        CRLF,
      ),
    ],
  )
  .payload("padded.bin", payload, Served::Exact)
  .finish()
}

fn base64_stray_bytes() -> CorpusMessage {
  let payload = random("base64_stray_bytes", 900);
  let mut encoded = base64(&payload);
  encoded.insert(40, '-');
  encoded.insert(150, '*');
  attachment_draft("base64_stray_bytes", "Base64 with stray non-alphabet bytes")
    .multipart(
      "multipart/mixed",
      "B64-STRAY",
      vec![
        text_part("A dash and an asterisk sit inside the base64.", CRLF),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"stray.bin\"",
            "Content-Transfer-Encoding: base64",
          ],
          wrap(&encoded, BASE64_LINE, CRLF),
          CRLF,
        ),
      ],
    )
    .payload("stray.bin", payload, Served::Pinned)
    .finish()
}

fn quoted_printable_octet_stream() -> CorpusMessage {
  let payload = random("quoted_printable_octet_stream", 700);
  attachment_draft(
    "quoted_printable_octet_stream",
    "Quoted-printable octet-stream",
  )
  .multipart(
    "multipart/mixed",
    "QP-OCTET",
    vec![
      text_part("Binary content in quoted-printable.", CRLF),
      part(
        &[
          "Content-Type: application/octet-stream",
          "Content-Disposition: attachment; filename=\"quoted.bin\"",
          "Content-Transfer-Encoding: quoted-printable",
        ],
        quoted_printable_binary(&payload, CRLF),
        CRLF,
      ),
    ],
  )
  .payload("quoted.bin", payload, Served::Exact)
  .finish()
}

fn binary_and_8bit() -> CorpusMessage {
  let mut binary: Vec<u8> = (0..=255u8).collect();
  binary.extend_from_slice(b"\r\n\r\nline breaks inside\r\n");
  binary.extend(random("binary_and_8bit/blob", 600));
  let eight_bit = "Z\u{fc}rich, M\u{fc}nchen, S\u{e3}o Paulo\r\nsecond line \u{2014} still 8bit"
    .as_bytes()
    .to_vec();
  attachment_draft("binary_and_8bit", "Binary and 8bit transfer encodings")
    .multipart(
      "multipart/mixed",
      "BIN-8BIT-7f3c9a",
      vec![
        text_part("Raw bytes follow.", CRLF),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"blob.bin\"",
            "Content-Transfer-Encoding: binary",
          ],
          &binary,
          CRLF,
        ),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"eight.bin\"",
            "Content-Transfer-Encoding: 8bit",
          ],
          &eight_bit,
          CRLF,
        ),
      ],
    )
    .payload("blob.bin", binary, Served::Exact)
    .payload("eight.bin", eight_bit, Served::Exact)
    .finish()
}

fn text_attachments() -> CorpusMessage {
  let csv_utf8 = "name,city\r\nZo\u{eb},Z\u{fc}rich\r\nAnn,Oslo"
    .as_bytes()
    .to_vec();
  let ics = concat!(
    "BEGIN:VCALENDAR\r\n",
    "VERSION:2.0\r\n",
    "PRODID:-//RustMail//Corpus//EN\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:planning@corpus.rustmail.test\r\n",
    "DTSTART:20260924T090000Z\r\n",
    "SUMMARY:Planning\r\n",
    "END:VEVENT\r\n",
    "END:VCALENDAR",
  )
  .as_bytes()
  .to_vec();
  let html = "<html><body><p>Attached <em>page</em> \u{2713}</p></body></html>"
    .as_bytes()
    .to_vec();
  let csv_latin1 = b"name,city\r\nRen\xe9,Gen\xe8ve".to_vec();
  let invalid_utf8 = INVALID_UTF8.to_vec();
  attachment_draft("text_attachments", "Text attachments in several charsets")
    .multipart(
      "multipart/mixed",
      "TEXT-ATTACH",
      vec![
        text_part("See the attached exports.", CRLF),
        part(
          &[
            "Content-Type: text/csv; charset=utf-8",
            "Content-Disposition: attachment; filename=\"data.csv\"",
            "Content-Transfer-Encoding: 8bit",
          ],
          &csv_utf8,
          CRLF,
        ),
        part(
          &[
            "Content-Type: text/calendar; charset=utf-8; method=REQUEST",
            "Content-Disposition: attachment; filename=\"invite.ics\"",
            "Content-Transfer-Encoding: 7bit",
          ],
          &ics,
          CRLF,
        ),
        part(
          &[
            "Content-Type: text/html; charset=utf-8",
            "Content-Disposition: attachment; filename=\"page.html\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&html, CRLF),
          CRLF,
        ),
        part(
          &[
            "Content-Type: text/csv; charset=iso-8859-1",
            "Content-Disposition: attachment; filename=\"latin1.csv\"",
            "Content-Transfer-Encoding: quoted-printable",
          ],
          "name,city\r\nRen=E9,Gen=E8ve",
          CRLF,
        ),
        part(
          &[
            "Content-Type: text/plain; charset=utf-8",
            "Content-Disposition: attachment; filename=\"broken.txt\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&invalid_utf8, CRLF),
          CRLF,
        ),
      ],
    )
    .payload("data.csv", csv_utf8, Served::Pinned)
    .payload("invite.ics", ics, Served::Pinned)
    .payload("page.html", html, Served::Pinned)
    .payload("latin1.csv", csv_latin1, Served::Pinned)
    .payload("broken.txt", invalid_utf8, Served::Pinned)
    .state(MailState {
      read: true,
      tags: &["work"],
      ..MailState::default()
    })
    .finish()
}

fn nested_related() -> CorpusMessage {
  let logo = with_signature(PNG_SIGNATURE, "nested_related/logo", 900);
  let chart = with_signature(PNG_SIGNATURE, "nested_related/chart", 700);
  let logo_duplicate = with_signature(GIF_SIGNATURE, "nested_related/logo-dup", 300);
  let spec = with_signature(PDF_SIGNATURE, "nested_related/spec", 400);
  let photo = random("nested_related/photo", 500);
  let report = with_signature(PDF_SIGNATURE, "nested_related/report", 1_300);
  let alternative = multipart_body(
    "ALT-NESTED",
    &[
      text_part("Plain alternative of the planning update.", CRLF),
      part(
        &["Content-Type: text/html; charset=utf-8"],
        "<p>Planning update</p><img src=\"cid:logo@corpus.test\"><img src=\"cid:chart@corpus.test\">",
        CRLF,
      ),
    ],
    CRLF,
  );
  let related = multipart_body(
    "REL-NESTED",
    &[
      part(
        &["Content-Type: multipart/alternative; boundary=\"ALT-NESTED\""],
        alternative,
        CRLF,
      ),
      part(
        &[
          "Content-Type: image/png; name=\"logo.png\"",
          "Content-ID: <logo@corpus.test>",
          "Content-Disposition: inline; filename=\"logo.png\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&logo, CRLF),
        CRLF,
      ),
      part(
        &[
          "Content-Type: image/png",
          "Content-ID: <chart@corpus.test>",
          "Content-Disposition: inline",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&chart, CRLF),
        CRLF,
      ),
      part(
        &[
          "Content-Type: image/gif; name=\"logo-dup.gif\"",
          "Content-ID: <logo@corpus.test>",
          "Content-Disposition: inline; filename=\"logo-dup.gif\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&logo_duplicate, CRLF),
        CRLF,
      ),
      part(
        &[
          "Content-Type: application/pdf; name=\"spec.pdf\"",
          "Content-ID: <spec@corpus.test>",
          "Content-Disposition: inline; filename=\"spec.pdf\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&spec, CRLF),
        CRLF,
      ),
      part(
        &[
          "Content-Type: IMAGE/JPEG",
          "Content-ID: <photo@corpus.test>",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&photo, CRLF),
        CRLF,
      ),
    ],
    CRLF,
  );
  attachment_draft("nested_related", "Nested related with cid images")
    .multipart(
      "multipart/mixed",
      "MIX-NESTED",
      vec![
        part(
          &["Content-Type: multipart/related; boundary=\"REL-NESTED\"; type=\"multipart/alternative\""],
          related,
          CRLF,
        ),
        part(
          &[
            "Content-Type: application/pdf; name=\"report.pdf\"",
            "Content-Disposition: attachment; filename=\"report.pdf\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&report, CRLF),
          CRLF,
        ),
      ],
    )
    .payload("logo.png", logo, Served::Exact)
    .payload("chart.png", chart, Served::Exact)
    .payload("logo-dup.gif", logo_duplicate, Served::Exact)
    .payload("spec.pdf", spec, Served::Exact)
    .payload("photo.jpg", photo, Served::Exact)
    .payload("report.pdf", report, Served::Exact)
    .state(MailState {
      starred: true,
      tags: &["urgent"],
      ..MailState::default()
    })
    .finish()
}

fn inner_message(subject: &str) -> Vec<u8> {
  format!(
    "From: inner@example.org\r\nTo: {BOB}\r\nSubject: {subject}\r\n{DATE}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe forwarded body line."
  )
  .into_bytes()
}

fn rfc822_plain() -> CorpusMessage {
  let inner = inner_message("Inner forwarded note");
  attachment_draft("rfc822_plain", "Forwarded message inline")
    .multipart(
      "multipart/mixed",
      "RFC822-PLAIN",
      vec![
        text_part("Forwarding the note below.", CRLF),
        part(
          &[
            "Content-Type: message/rfc822",
            "Content-Disposition: attachment; filename=\"forwarded.eml\"",
          ],
          &inner,
          CRLF,
        ),
      ],
    )
    .payload("forwarded.eml", inner, Served::Exact)
    .finish()
}

fn rfc822_base64() -> CorpusMessage {
  let inner = inner_message("Inner base64 note");
  attachment_draft("rfc822_base64", "Forwarded message in base64")
    .multipart(
      "multipart/mixed",
      "RFC822-B64",
      vec![
        text_part("Forwarding the encoded note below.", CRLF),
        part(
          &[
            "Content-Type: message/rfc822",
            "Content-Disposition: attachment; filename=\"encoded.eml\"",
            "Content-Transfer-Encoding: base64",
          ],
          base64_lines(&inner, CRLF),
          CRLF,
        ),
      ],
    )
    .payload("encoded.eml", inner, Served::Exact)
    .finish()
}

fn digest_without_content_types() -> CorpusMessage {
  let first =
    b"From: one@example.org\r\nSubject: Digest item one\r\n\r\nFirst digested body.".to_vec();
  let second =
    b"From: two@example.org\r\nSubject: Digest item two\r\n\r\nSecond digested body.".to_vec();
  attachment_draft(
    "digest_without_content_types",
    "Digest whose parts have no Content-Type",
  )
  .multipart(
    "multipart/digest",
    "DIGEST",
    vec![part(&[], &first, CRLF), part(&[], &second, CRLF)],
  )
  .payload("digest item one", first, Served::Pinned)
  .payload("digest item two", second, Served::Pinned)
  .finish()
}

fn part_without_content_type() -> CorpusMessage {
  let notes = b"Notes with no declared type.\r\nSecond line.".to_vec();
  attachment_draft(
    "part_without_content_type",
    "Attachment with no Content-Type",
  )
  .multipart(
    "multipart/mixed",
    "NO-CT",
    vec![
      text_part("The attachment declares no type.", CRLF),
      part(
        &["Content-Disposition: attachment; filename=\"notes.txt\""],
        &notes,
        CRLF,
      ),
    ],
  )
  .payload("notes.txt", notes, Served::Pinned)
  .finish()
}

fn base64_malformed() -> CorpusMessage {
  let intended = b"Hello, malformed base64 world".to_vec();
  attachment_draft("base64_malformed", "Malformed base64")
    .multipart(
      "multipart/mixed",
      "B64-BAD",
      vec![
        text_part("The attachment is not valid base64.", CRLF),
        part(
          &[
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"malformed.bin\"",
            "Content-Transfer-Encoding: base64",
          ],
          "SGVsbG8sIG1hbGZvcm1lZA\r\n=QmFz=ZTY0\r\n#@!\r\nIHdvcmxk=",
          CRLF,
        ),
      ],
    )
    .payload("malformed.bin", intended, Served::Pinned)
    .finish()
}

fn large_10mib() -> CorpusMessage {
  let big = random("large_10mib/big", LARGE_ATTACHMENT_BYTES);
  let filler = random("large_10mib/filler", LARGE_FILLER_BYTES);
  attachment_draft(
    "large_10mib",
    "Ten mebibytes with a seven megabyte attachment",
  )
  .multipart(
    "multipart/mixed",
    "LARGE-10MIB",
    vec![
      text_part("The large archive is attached.", CRLF),
      part(
        &[
          "Content-Type: application/octet-stream",
          "Content-Disposition: attachment; filename=\"big.bin\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&big, CRLF),
        CRLF,
      ),
      part(
        &[
          "Content-Type: application/zip",
          "Content-Disposition: attachment; filename=\"filler.zip\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64_lines(&filler, CRLF),
        CRLF,
      ),
    ],
  )
  .payload("big.bin", big, Served::Exact)
  .payload("filler.zip", filler, Served::Exact)
  .finish()
}
