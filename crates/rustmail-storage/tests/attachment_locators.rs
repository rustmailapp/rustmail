//! The attachment correctness matrix: which parts are served from the raw
//! message and which stay stored decoded, and that either way the bytes
//! served are the bytes the parser produced in context.

use mail_parser::{MessageParser, MimeHeaders};
use rustmail_storage::{MessageRepository, StorageError, initialize_database};
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

const CRLF: &str = "\r\n";
const LF: &str = "\n";
const BASE64_LINE: usize = 76;
const QP_LINE: usize = 72;
const BASE64_ALPHABET: &[u8; 64] =
  b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const LARGE_ATTACHMENT_BYTES: usize = 7 * 1000 * 1000;
const LARGE_FILLER_BYTES: usize = 300 * 1000;

#[derive(Debug, PartialEq, Eq)]
enum Stored {
  Located,
  Inline,
}

struct Harness {
  pool: SqlitePool,
  repo: MessageRepository,
}

impl Harness {
  async fn new() -> Self {
    let pool = SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool.clone());
    Self { pool, repo }
  }

  async fn store(&self, raw: &[u8]) -> String {
    self
      .repo
      .insert("sender@test.com", &["rcpt@test.com".to_string()], raw)
      .await
      .unwrap()
      .id
  }

  async fn attachment_id(&self, message_id: &str, filename: &str) -> String {
    self
      .repo
      .get_attachments(message_id)
      .await
      .unwrap()
      .into_iter()
      .find(|attachment| attachment.filename.as_deref() == Some(filename))
      .unwrap_or_else(|| panic!("no attachment named {filename}"))
      .id
  }

  async fn stored(&self, attachment_id: &str) -> Stored {
    let located: bool =
      sqlx::query_scalar("SELECT raw_offset IS NOT NULL FROM attachments WHERE id = ?1")
        .bind(attachment_id)
        .fetch_one(&self.pool)
        .await
        .unwrap();
    if located {
      Stored::Located
    } else {
      Stored::Inline
    }
  }

  async fn served(&self, message_id: &str, attachment_id: &str) -> Vec<u8> {
    self
      .repo
      .get_attachment(message_id, attachment_id)
      .await
      .unwrap()
      .content
  }

  /// Stores `raw` and asserts how the part named `filename` is kept, that it
  /// is served as the parser decodes it in context, and returns what was
  /// served.
  async fn check(&self, raw: &[u8], filename: &str, expected: Stored) -> Vec<u8> {
    let message_id = self.store(raw).await;
    let attachment_id = self.attachment_id(&message_id, filename).await;
    assert_eq!(
      self.stored(&attachment_id).await,
      expected,
      "{filename} stored the wrong way"
    );
    let served = self.served(&message_id, &attachment_id).await;
    assert_eq!(
      served,
      parser_contents(raw, filename),
      "{filename} served differently from the in-context parse"
    );
    served
  }
}

fn parser_contents(raw: &[u8], filename: &str) -> Vec<u8> {
  let parsed = MessageParser::default().parse(raw).unwrap();
  parsed
    .parts
    .iter()
    .find(|part| part.attachment_name() == Some(filename))
    .unwrap_or_else(|| panic!("the parser found no part named {filename}"))
    .contents()
    .to_vec()
}

struct Prng(u64);

impl Prng {
  fn seeded(label: &str) -> Self {
    Self(
      label
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
          (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
        .max(1),
    )
  }

  fn bytes(&mut self, len: usize) -> Vec<u8> {
    (0..len)
      .map(|_| {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.to_le_bytes()[0]
      })
      .collect()
  }
}

fn random(label: &str, len: usize) -> Vec<u8> {
  Prng::seeded(label).bytes(len)
}

fn base64(bytes: &[u8]) -> String {
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let word = chunk.iter().enumerate().fold(0u32, |word, (i, &byte)| {
      word | u32::from(byte) << (16 - 8 * i)
    });
    for i in 0..4 {
      if i <= chunk.len() {
        out.push(char::from(
          BASE64_ALPHABET[(word >> (18 - 6 * i) & 0x3f) as usize],
        ));
      } else {
        out.push('=');
      }
    }
  }
  out
}

fn wrap(text: &str, width: usize, eol: &str) -> String {
  text
    .as_bytes()
    .chunks(width)
    .map(|line| std::str::from_utf8(line).unwrap())
    .collect::<Vec<_>>()
    .join(eol)
}

fn quoted_printable(bytes: &[u8], eol: &str) -> String {
  let mut out = String::new();
  let mut line_len = 0;
  for &byte in bytes {
    let encoded = if (b'!'..=b'~').contains(&byte) && byte != b'=' {
      char::from(byte).to_string()
    } else {
      format!("={byte:02X}")
    };
    if line_len + encoded.len() > QP_LINE {
      out.push('=');
      out.push_str(eol);
      line_len = 0;
    }
    line_len += encoded.len();
    out.push_str(&encoded);
  }
  out
}

fn part(headers: &[&str], body: impl AsRef<[u8]>, eol: &str) -> Vec<u8> {
  let mut out = Vec::new();
  for header in headers {
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(eol.as_bytes());
  }
  out.extend_from_slice(eol.as_bytes());
  out.extend_from_slice(body.as_ref());
  out
}

fn multipart(content_type: &str, boundary: &str, parts: &[Vec<u8>], eol: &str) -> Vec<u8> {
  let mut out =
    format!("Content-Type: {content_type}; boundary=\"{boundary}\"{eol}{eol}").into_bytes();
  for part in parts {
    out.extend_from_slice(format!("--{boundary}{eol}").as_bytes());
    out.extend_from_slice(part);
    out.extend_from_slice(eol.as_bytes());
  }
  out.extend_from_slice(format!("--{boundary}--{eol}").as_bytes());
  out
}

fn message(body: Vec<u8>, eol: &str) -> Vec<u8> {
  let mut raw = format!(
    "From: sender@test.com{eol}To: rcpt@test.com{eol}Subject: Matrix{eol}MIME-Version: 1.0{eol}"
  )
  .into_bytes();
  raw.extend(body);
  raw
}

fn mixed(attachments: Vec<Vec<u8>>, eol: &str) -> Vec<u8> {
  let mut parts = vec![part(
    &["Content-Type: text/plain; charset=utf-8"],
    "See attached.",
    eol,
  )];
  parts.extend(attachments);
  message(multipart("multipart/mixed", "MIX", &parts, eol), eol)
}

fn binary_attachment(filename: &str, encoding: &str, body: impl AsRef<[u8]>, eol: &str) -> Vec<u8> {
  let disposition = format!("Content-Disposition: attachment; filename=\"{filename}\"");
  let transfer = format!("Content-Transfer-Encoding: {encoding}");
  part(
    &[
      "Content-Type: application/octet-stream",
      &disposition,
      &transfer,
    ],
    body,
    eol,
  )
}

#[tokio::test]
async fn base64_with_76_column_crlf_lines_is_located() {
  let harness = Harness::new().await;
  let payload = random("crlf76", 1500);
  let raw = mixed(
    vec![binary_attachment(
      "crlf76.bin",
      "base64",
      wrap(&base64(&payload), BASE64_LINE, CRLF),
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "crlf76.bin", Stored::Located).await,
    payload
  );
}

#[tokio::test]
async fn base64_on_one_unwrapped_line_is_located() {
  let harness = Harness::new().await;
  let payload = random("unwrapped", 1200);
  let raw = mixed(
    vec![binary_attachment(
      "unwrapped.bin",
      "base64",
      base64(&payload),
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "unwrapped.bin", Stored::Located).await,
    payload
  );
}

#[tokio::test]
async fn base64_in_an_lf_only_message_is_located() {
  let harness = Harness::new().await;
  let payload = random("lf-only", 1000);
  let raw = mixed(
    vec![binary_attachment(
      "lf.bin",
      "base64",
      wrap(&base64(&payload), BASE64_LINE, LF),
      LF,
    )],
    LF,
  );
  assert_eq!(
    harness.check(&raw, "lf.bin", Stored::Located).await,
    payload
  );
}

#[tokio::test]
async fn base64_with_trailing_whitespace_is_located() {
  let harness = Harness::new().await;
  let payload = random("trailing-ws", 1000);
  let body = wrap(&base64(&payload), BASE64_LINE, " \t\r\n") + " \t ";
  let raw = mixed(
    vec![binary_attachment("ws.bin", "base64", body, CRLF)],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "ws.bin", Stored::Located).await,
    payload
  );
}

#[tokio::test]
async fn base64_with_a_stray_dash_stays_inline() {
  let harness = Harness::new().await;
  let payload = random("stray-dash", 900);
  let mut encoded = base64(&payload);
  encoded.insert(40, '-');
  let raw = mixed(
    vec![binary_attachment(
      "dash.bin",
      "base64",
      wrap(&encoded, BASE64_LINE, CRLF),
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "dash.bin", Stored::Inline).await,
    payload
  );
}

#[tokio::test]
async fn base64_with_another_non_alphabet_byte_stays_inline() {
  let harness = Harness::new().await;
  let mut encoded = base64(&random("stray-star", 900));
  encoded.insert(150, '*');
  let raw = mixed(
    vec![binary_attachment(
      "star.bin",
      "base64",
      wrap(&encoded, BASE64_LINE, CRLF),
      CRLF,
    )],
    CRLF,
  );
  harness.check(&raw, "star.bin", Stored::Inline).await;
}

#[tokio::test]
async fn quoted_printable_octet_stream_is_located() {
  let harness = Harness::new().await;
  let payload = random("qp", 700);
  let raw = mixed(
    vec![binary_attachment(
      "quoted.bin",
      "quoted-printable",
      quoted_printable(&payload, CRLF),
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "quoted.bin", Stored::Located).await,
    payload
  );
}

#[tokio::test]
async fn binary_and_8bit_parts_are_located_without_the_trailing_line_break() {
  let harness = Harness::new().await;
  let mut binary: Vec<u8> = (0..=255u8).collect();
  binary.extend_from_slice(b"\r\n\r\nline breaks inside\r\n");
  binary.extend(random("binary", 600));
  let eight_bit = "Z\u{fc}rich, M\u{fc}nchen\r\nsecond line \u{2014} still 8bit"
    .as_bytes()
    .to_vec();
  let raw = mixed(
    vec![
      binary_attachment("blob.bin", "binary", &binary, CRLF),
      binary_attachment("eight.bin", "8bit", &eight_bit, CRLF),
    ],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "blob.bin", Stored::Located).await,
    binary
  );
  assert_eq!(
    harness.check(&raw, "eight.bin", Stored::Located).await,
    eight_bit
  );
}

#[tokio::test]
async fn text_attachments_stay_inline_and_charset_converted() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![
      part(
        &[
          "Content-Type: text/csv; charset=utf-8",
          "Content-Disposition: attachment; filename=\"rows.csv\"",
        ],
        "id,city\r\n1,Z\u{fc}rich",
        CRLF,
      ),
      part(
        &[
          "Content-Type: text/calendar; charset=utf-8",
          "Content-Disposition: attachment; filename=\"invite.ics\"",
          "Content-Transfer-Encoding: base64",
        ],
        base64(b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n"),
        CRLF,
      ),
      part(
        &[
          "Content-Type: text/html; charset=utf-8",
          "Content-Disposition: attachment; filename=\"page.html\"",
        ],
        "<p>Hello</p>",
        CRLF,
      ),
      part(
        &[
          "Content-Type: text/plain; charset=iso-8859-1",
          "Content-Disposition: attachment; filename=\"latin1.txt\"",
        ],
        b"caf\xe9",
        CRLF,
      ),
      part(
        &[
          "Content-Type: text/plain; charset=utf-8",
          "Content-Disposition: attachment; filename=\"invalid.txt\"",
        ],
        b"bad \xff\xfe bytes",
        CRLF,
      ),
    ],
    CRLF,
  );
  for filename in [
    "rows.csv",
    "invite.ics",
    "page.html",
    "latin1.txt",
    "invalid.txt",
  ] {
    harness.check(&raw, filename, Stored::Inline).await;
  }
  assert_eq!(
    harness.check(&raw, "latin1.txt", Stored::Inline).await,
    "caf\u{e9}".as_bytes()
  );
}

#[tokio::test]
async fn nested_related_images_are_located_at_absolute_offsets() {
  let harness = Harness::new().await;
  let logo = random("logo", 800);
  let banner = random("banner", 600);
  let duplicate = random("duplicate", 300);
  let image = |filename: &str, cid: &str, payload: &[u8]| {
    let disposition = format!("Content-Disposition: inline; filename=\"{filename}\"");
    let content_id = format!("Content-ID: <{cid}>");
    part(
      &[
        "Content-Type: image/png",
        &disposition,
        &content_id,
        "Content-Transfer-Encoding: base64",
      ],
      wrap(&base64(payload), BASE64_LINE, CRLF),
      CRLF,
    )
  };
  let alternative = multipart(
    "multipart/alternative",
    "ALT",
    &[
      part(&["Content-Type: text/plain"], "Plain", CRLF),
      part(
        &["Content-Type: text/html"],
        "<img src=\"cid:logo@test\"><img src=\"cid:banner@test\">",
        CRLF,
      ),
    ],
    CRLF,
  );
  let related = multipart(
    "multipart/related",
    "REL",
    &[
      alternative,
      image("logo.png", "logo@test", &logo),
      image("banner.png", "banner@test", &banner),
      image("again.png", "logo@test", &duplicate),
    ],
    CRLF,
  );
  let raw = message(
    multipart(
      "multipart/mixed",
      "MIX",
      &[
        related,
        binary_attachment("tail.bin", "base64", base64(&random("tail", 200)), CRLF),
      ],
      CRLF,
    ),
    CRLF,
  );
  assert_eq!(harness.check(&raw, "logo.png", Stored::Located).await, logo);
  assert_eq!(
    harness.check(&raw, "banner.png", Stored::Located).await,
    banner
  );
  harness.check(&raw, "tail.bin", Stored::Located).await;

  let message_id = harness.store(&raw).await;
  let by_cid = harness
    .repo
    .get_attachment_by_content_id(&message_id, "logo@test")
    .await
    .unwrap();
  assert_eq!(by_cid.filename.as_deref(), Some("logo.png"));
  assert_eq!(by_cid.content, logo);

  let banner_id = harness.attachment_id(&message_id, "banner.png").await;
  let (offset, len): (i64, i64) =
    sqlx::query_as("SELECT raw_offset, raw_len FROM attachments WHERE id = ?1")
      .bind(&banner_id)
      .fetch_one(&harness.pool)
      .await
      .unwrap();
  let slice = &raw[offset as usize..(offset + len) as usize];
  assert_eq!(slice, wrap(&base64(&banner), BASE64_LINE, CRLF).as_bytes());
}

fn inner_message(subject: &str) -> Vec<u8> {
  format!(
    "From: inner@test.com\r\nTo: rcpt@test.com\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe forwarded body line."
  )
  .into_bytes()
}

#[tokio::test]
async fn a_plain_forwarded_message_is_located() {
  let harness = Harness::new().await;
  let inner = inner_message("Plain forward");
  let raw = mixed(
    vec![part(
      &[
        "Content-Type: message/rfc822",
        "Content-Disposition: attachment; filename=\"forwarded.eml\"",
      ],
      &inner,
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "forwarded.eml", Stored::Located).await,
    inner
  );
}

#[tokio::test]
async fn a_base64_forwarded_message_is_located() {
  let harness = Harness::new().await;
  let inner = inner_message("Encoded forward");
  let raw = mixed(
    vec![part(
      &[
        "Content-Type: message/rfc822",
        "Content-Disposition: attachment; filename=\"encoded.eml\"",
        "Content-Transfer-Encoding: base64",
      ],
      wrap(&base64(&inner), BASE64_LINE, CRLF),
      CRLF,
    )],
    CRLF,
  );
  assert_eq!(
    harness.check(&raw, "encoded.eml", Stored::Located).await,
    inner
  );
}

#[tokio::test]
async fn digest_parts_without_a_content_type_stay_inline() {
  let harness = Harness::new().await;
  let first = b"From: one@test.com\r\nSubject: Digest one\r\n\r\nFirst digested body.".to_vec();
  let raw = message(
    multipart(
      "multipart/digest",
      "DIGEST",
      &[part(&[], &first, CRLF)],
      CRLF,
    ),
    CRLF,
  );
  let message_id = harness.store(&raw).await;
  let attachments = harness.repo.get_attachments(&message_id).await.unwrap();
  assert!(!attachments.is_empty(), "the digest item should be stored");
  for attachment in attachments {
    assert_eq!(harness.stored(&attachment.id).await, Stored::Inline);
  }
}

#[tokio::test]
async fn malformed_base64_stays_inline() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![binary_attachment(
      "malformed.bin",
      "base64",
      "SGVsbG8sIG1hbGZvcm1lZA\r\n=QmFz=ZTY0\r\n#@!\r\nIHdvcmxk=",
      CRLF,
    )],
    CRLF,
  );
  harness.check(&raw, "malformed.bin", Stored::Inline).await;
}

#[tokio::test]
async fn a_large_attachment_in_a_ten_mebibyte_message_is_located_and_served() {
  let harness = Harness::new().await;
  let big = random("large", LARGE_ATTACHMENT_BYTES);
  let raw = mixed(
    vec![
      binary_attachment(
        "big.bin",
        "base64",
        wrap(&base64(&big), BASE64_LINE, CRLF),
        CRLF,
      ),
      binary_attachment(
        "filler.bin",
        "base64",
        wrap(
          &base64(&random("filler", LARGE_FILLER_BYTES)),
          BASE64_LINE,
          CRLF,
        ),
        CRLF,
      ),
    ],
    CRLF,
  );
  assert!(raw.len() > 9 * 1024 * 1024);
  let message_id = harness.store(&raw).await;
  let attachment_id = harness.attachment_id(&message_id, "big.bin").await;
  assert_eq!(harness.stored(&attachment_id).await, Stored::Located);
  assert!(harness.served(&message_id, &attachment_id).await == big);
}

async fn located_fixture(harness: &Harness, raw: &[u8], filename: &str) -> (String, String) {
  let message_id = harness.store(raw).await;
  let attachment_id = harness.attachment_id(&message_id, filename).await;
  assert_eq!(harness.stored(&attachment_id).await, Stored::Located);
  (message_id, attachment_id)
}

async fn tamper(harness: &Harness, attachment_id: &str, assignment: &str) {
  sqlx::query(&format!(
    "UPDATE attachments SET {assignment} WHERE id = ?1"
  ))
  .bind(attachment_id)
  .execute(&harness.pool)
  .await
  .unwrap();
}

fn assert_corrupt(result: Result<rustmail_storage::Attachment, StorageError>, attachment_id: &str) {
  match result {
    Err(StorageError::AttachmentCorrupt {
      attachment_id: reported,
      ..
    }) => assert_eq!(reported, attachment_id),
    Err(other) => panic!("expected AttachmentCorrupt, got {other}"),
    Ok(_) => panic!("a corrupt locator was served"),
  }
}

#[tokio::test]
async fn an_identity_slice_forged_to_carry_the_trailing_line_break_is_rejected() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![binary_attachment(
      "blob.bin",
      "binary",
      random("forged", 64),
      CRLF,
    )],
    CRLF,
  );
  let (message_id, attachment_id) = located_fixture(&harness, &raw, "blob.bin").await;
  tamper(&harness, &attachment_id, "raw_len = raw_len + 2").await;
  assert_corrupt(
    harness
      .repo
      .get_attachment(&message_id, &attachment_id)
      .await,
    &attachment_id,
  );
}

#[tokio::test]
async fn a_base64_locator_cut_short_is_rejected() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![binary_attachment(
      "short.bin",
      "base64",
      base64(&random("short", 300)),
      CRLF,
    )],
    CRLF,
  );
  let (message_id, attachment_id) = located_fixture(&harness, &raw, "short.bin").await;
  tamper(&harness, &attachment_id, "raw_len = raw_len - 4").await;
  assert_corrupt(
    harness
      .repo
      .get_attachment(&message_id, &attachment_id)
      .await,
    &attachment_id,
  );
}

#[tokio::test]
async fn a_locator_past_the_end_of_the_raw_message_is_rejected() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![binary_attachment(
      "end.bin",
      "binary",
      random("end", 64),
      CRLF,
    )],
    CRLF,
  );
  let (message_id, attachment_id) = located_fixture(&harness, &raw, "end.bin").await;
  tamper(
    &harness,
    &attachment_id,
    "raw_offset = raw_offset + 1000000",
  )
  .await;
  assert_corrupt(
    harness
      .repo
      .get_attachment(&message_id, &attachment_id)
      .await,
    &attachment_id,
  );
}

#[tokio::test]
async fn a_locator_whose_bytes_no_longer_decode_is_rejected_by_content_id() {
  let harness = Harness::new().await;
  let raw = mixed(
    vec![part(
      &[
        "Content-Type: image/png",
        "Content-ID: <img@test>",
        "Content-Disposition: inline; filename=\"img.png\"",
        "Content-Transfer-Encoding: base64",
      ],
      base64(&random("img", 90)),
      CRLF,
    )],
    CRLF,
  );
  let (message_id, attachment_id) = located_fixture(&harness, &raw, "img.png").await;
  tamper(&harness, &attachment_id, "raw_offset = raw_offset - 2").await;
  assert_corrupt(
    harness
      .repo
      .get_attachment_by_content_id(&message_id, "img@test")
      .await,
    &attachment_id,
  );
}
