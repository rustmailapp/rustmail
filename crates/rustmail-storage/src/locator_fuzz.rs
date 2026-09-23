//! Randomized check of attachment locators against the in-context parse.
//!
//! Base64 eligibility verifies only the predicted decoded length (D8), so a
//! locator window shifted by an offset bug while keeping its length would
//! pass ingest and serve the wrong bytes. These tests generate many varied
//! MIME messages from a fixed seed, run them through
//! [`PreparedMessage::parse`], and require every located part to decode from
//! its raw window to exactly the bytes mail-parser produced for it.

use mail_parser::{Message, MessageParser, MessagePart, PartType};

use crate::locator::{Locator, TransferEncoding, locate};
use crate::prepared::{AttachmentStorage, PreparedMessage, part_contents, stored_part_refs};

const SEED: u64 = 0x5eed_da7a_4f11_c0de;
const MESSAGES: usize = 4000;
const MAX_DEPTH: usize = 3;
const MAX_PAYLOAD: usize = 1500;
const WRAP_WIDTHS: &[usize] = &[0, 1, 3, 57, 60, 64, 76, 77, 100];
const EOLS: &[&str] = &["\r\n", "\n"];
const BASE64_ALPHABET: &[u8; 64] =
  b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const MIN_LOCATED_PER_KIND: usize = 200;

struct Prng(u64);

impl Prng {
  fn next(&mut self) -> u64 {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    self.0
  }

  fn below(&mut self, bound: usize) -> usize {
    (self.next() % bound as u64) as usize
  }

  fn chance(&mut self, percent: usize) -> bool {
    self.below(100) < percent
  }

  fn pick<T: Copy>(&mut self, items: &[T]) -> T {
    items[self.below(items.len())]
  }

  fn bytes(&mut self, len: usize) -> Vec<u8> {
    (0..len).map(|_| self.next().to_le_bytes()[0]).collect()
  }

  fn payload(&mut self) -> Vec<u8> {
    let len = match self.below(4) {
      0 => self.below(8),
      _ => self.below(MAX_PAYLOAD),
    };
    self.bytes(len)
  }
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

fn wrap(encoded: &str, width: usize, eol: &str, line_tail: &str) -> String {
  if width == 0 || encoded.is_empty() {
    return format!("{encoded}{line_tail}");
  }
  encoded
    .as_bytes()
    .chunks(width)
    .map(|line| format!("{}{line_tail}", String::from_utf8_lossy(line)))
    .collect::<Vec<_>>()
    .join(eol)
}

fn random_base64_body(rng: &mut Prng, payload: &[u8], eol: &str) -> String {
  let line_tail = rng.pick(&["", "", " ", "\t", " \t "]);
  let mut body = wrap(&base64(payload), rng.pick(WRAP_WIDTHS), eol, line_tail);
  if rng.chance(15) {
    body.push_str(eol);
  }
  if rng.chance(10) {
    body.push_str(&format!("{eol}{eol}"));
  }
  body
}

fn quoted_printable(rng: &mut Prng, payload: &[u8], eol: &str) -> String {
  let width = 4 + rng.below(80);
  let mut out = String::new();
  let mut line_len = 0;
  for (i, &byte) in payload.iter().enumerate() {
    let encoded = match byte {
      b'\n' if rng.chance(50) => eol.to_string(),
      b' ' | b'\t' if rng.chance(50) && i + 1 < payload.len() => char::from(byte).to_string(),
      b'!'..=b'~' if byte != b'=' => char::from(byte).to_string(),
      _ => format!("={byte:02X}"),
    };
    if line_len + encoded.len() > width {
      out.push('=');
      out.push_str(rng.pick(&["\r\n", "\n", eol, eol]));
      line_len = 0;
    }
    line_len += encoded.len();
    out.push_str(&encoded);
  }
  if rng.chance(10) {
    out.push('=');
    out.push_str(eol);
  }
  out
}

fn text_like(rng: &mut Prng, eol: &str) -> Vec<u8> {
  let words = ["alpha", "beta", "--", "=", " ", "caf\u{e9}", "line", "\t"];
  let mut text = String::new();
  for _ in 0..rng.below(40) {
    text.push_str(rng.pick(&words));
    if rng.chance(15) {
      text.push_str(eol);
    }
  }
  text.into_bytes()
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

fn disposition(rng: &mut Prng, index: usize, headers: &mut Vec<String>) {
  match rng.below(3) {
    0 => headers.push(format!(
      "Content-Disposition: attachment; filename=\"part-{index}.bin\""
    )),
    1 => headers.push(format!(
      "Content-Disposition: inline; filename=\"part-{index}.bin\""
    )),
    _ => {}
  }
  if rng.chance(30) {
    headers.push(format!("Content-ID: <cid-{index}@fuzz>"));
  }
}

fn binary_type(rng: &mut Prng) -> &'static str {
  rng.pick(&[
    "application/octet-stream",
    "image/png",
    "application/pdf",
    "audio/ogg",
  ])
}

struct Generator {
  rng: Prng,
  next_index: usize,
}

impl Generator {
  fn index(&mut self) -> usize {
    self.next_index += 1;
    self.next_index
  }

  fn message(&mut self, depth: usize, eol: &str) -> Vec<u8> {
    let index = self.index();
    let mut raw = format!(
      "From: s{index}@fuzz.test{eol}To: r@fuzz.test{eol}Subject: Fuzz {index}{eol}MIME-Version: 1.0{eol}"
    )
    .into_bytes();
    raw.extend(self.part(depth, eol, true, false));
    if self.rng.chance(50) {
      raw.extend_from_slice(eol.as_bytes());
    }
    raw
  }

  /// One MIME entity, headers and body, without the line break that
  /// precedes the next boundary.
  ///
  /// Two shapes are generated differently, because mail-parser 0.11.2
  /// reaches a `debug_assert!(false)` on them, which aborts a debug build and
  /// is compiled out of a release one: a digest item without a
  /// `Content-Type` always carries a message, and a message's root entity is
  /// never itself an unencoded `message/rfc822` (it is base64 instead).
  fn part(&mut self, depth: usize, eol: &str, is_root: bool, in_digest: bool) -> Vec<u8> {
    let kinds = if depth == 0 { 7 } else { 9 };
    let kind = if is_root && depth > 0 && self.rng.chance(80) {
      8
    } else {
      match self.rng.below(kinds) {
        5 if is_root => 6,
        kind => kind,
      }
    };
    let index = self.index();
    let mut headers = Vec::new();
    let body: Vec<u8> = match kind {
      0 => {
        headers.push(format!("Content-Type: {}", binary_type(&mut self.rng)));
        headers.push(
          self
            .rng
            .pick(&[
              "Content-Transfer-Encoding: base64",
              "Content-Transfer-Encoding: BASE64",
            ])
            .to_string(),
        );
        disposition(&mut self.rng, index, &mut headers);
        let payload = self.rng.payload();
        random_base64_body(&mut self.rng, &payload, eol).into_bytes()
      }
      1 => {
        headers.push(format!("Content-Type: {}", binary_type(&mut self.rng)));
        headers.push("Content-Transfer-Encoding: quoted-printable".to_string());
        disposition(&mut self.rng, index, &mut headers);
        let payload = self.rng.payload();
        quoted_printable(&mut self.rng, &payload, eol).into_bytes()
      }
      2 => {
        headers.push(format!("Content-Type: {}", binary_type(&mut self.rng)));
        if let Some(encoding) = self
          .rng
          .pick(&[Some("binary"), Some("8bit"), Some("7bit"), None])
        {
          headers.push(format!("Content-Transfer-Encoding: {encoding}"));
        }
        disposition(&mut self.rng, index, &mut headers);
        let mut payload = self.rng.payload();
        if self.rng.chance(20) {
          payload.extend_from_slice(eol.as_bytes());
        }
        payload
      }
      3 => {
        headers.push(
          self
            .rng
            .pick(&[
              "Content-Type: text/plain; charset=utf-8",
              "Content-Type: text/html; charset=utf-8",
              "Content-Type: text/csv; charset=iso-8859-1",
            ])
            .to_string(),
        );
        disposition(&mut self.rng, index, &mut headers);
        text_like(&mut self.rng, eol)
      }
      4 => {
        disposition(&mut self.rng, index, &mut headers);
        if in_digest {
          self.message(depth.saturating_sub(1), eol)
        } else {
          text_like(&mut self.rng, eol)
        }
      }
      5 | 6 => {
        headers.push("Content-Type: message/rfc822".to_string());
        disposition(&mut self.rng, index, &mut headers);
        let inner = self.message(depth.saturating_sub(1), eol);
        if kind == 5 {
          inner
        } else {
          headers.push("Content-Transfer-Encoding: base64".to_string());
          random_base64_body(&mut self.rng, &inner, eol).into_bytes()
        }
      }
      _ => return self.multipart(depth, eol),
    };
    entity(&headers, &body, eol)
  }

  fn multipart(&mut self, depth: usize, eol: &str) -> Vec<u8> {
    let subtype = self
      .rng
      .pick(&["mixed", "related", "alternative", "digest"]);
    let boundary = format!("=_b{}_{:x}", depth, self.rng.next() & 0xffff_ffff);
    let headers = [format!(
      "Content-Type: multipart/{subtype}; boundary=\"{boundary}\""
    )];
    let mut body = Vec::new();
    if self.rng.chance(30) {
      body.extend(text_like(&mut self.rng, eol));
      body.extend_from_slice(eol.as_bytes());
    }
    for _ in 0..1 + self.rng.below(4) {
      body.extend_from_slice(format!("--{boundary}{eol}").as_bytes());
      body.extend(self.part(depth - 1, eol, false, subtype == "digest"));
      body.extend_from_slice(eol.as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--").as_bytes());
    match self.rng.below(3) {
      0 => {}
      1 => body.extend_from_slice(eol.as_bytes()),
      _ => {
        body.extend_from_slice(eol.as_bytes());
        body.extend(text_like(&mut self.rng, eol));
      }
    }
    entity(&headers, &body, eol)
  }
}

#[derive(Default)]
struct Coverage {
  located: [usize; 3],
  nested_messages: usize,
  checked_in_nested: usize,
}

fn assert_window(raw: &[u8], locator: Locator, part: &MessagePart<'_>, context: &str) {
  let window = raw
    .get(locator.offset..locator.offset + locator.len)
    .unwrap_or_else(|| panic!("{context}: locator {locator:?} lies outside the raw message"));
  let decoded = locator.encoding.decode(window);
  assert!(
    decoded.as_deref() == Some(part_contents(part)),
    "{context}: locator {locator:?} decodes to {:?} bytes, the parser produced {} ({:?})",
    decoded.as_ref().map(Vec::len),
    part_contents(part).len(),
    String::from_utf8_lossy(window),
  );
}

fn assert_every_locate_holds(
  raw: &[u8],
  message: &Message<'_>,
  depth: usize,
  context: &str,
  coverage: &mut Coverage,
) {
  for part in &message.parts {
    if let Some(locator) = locate(raw, part) {
      assert_window(raw, locator, part, context);
      if depth > 0 {
        coverage.checked_in_nested += 1;
      }
    }
    if let PartType::Message(nested) = &part.body {
      coverage.nested_messages += 1;
      assert_every_locate_holds(&nested.raw_message, nested, depth + 1, context, coverage);
    }
  }
}

fn check_message(raw: &[u8], context: &str, coverage: &mut Coverage) {
  let prepared = PreparedMessage::parse("s@fuzz.test".to_string(), &[], raw.to_vec());
  let Some(parsed) = MessageParser::default().parse(raw) else {
    assert!(prepared.attachments.is_empty(), "{context}");
    return;
  };
  let stored: Vec<&MessagePart<'_>> = stored_part_refs(&parsed).collect();
  assert_eq!(stored.len(), prepared.attachments.len(), "{context}");
  for (part, attachment) in stored.iter().zip(&prepared.attachments) {
    assert_eq!(attachment.size, part_contents(part).len(), "{context}");
    match &attachment.storage {
      AttachmentStorage::Located(locator) => {
        assert_window(raw, *locator, part, context);
        let slot = usize::try_from(locator.encoding.code()).unwrap();
        coverage.located[slot] += 1;
      }
      AttachmentStorage::Inline(content) => {
        assert_eq!(content.as_slice(), part_contents(part), "{context}");
      }
    }
  }
  assert_every_locate_holds(raw, &parsed, 0, context, coverage);
}

#[test]
fn every_located_part_decodes_to_the_in_context_contents() {
  let mut generator = Generator {
    rng: Prng(SEED),
    next_index: 0,
  };
  let mut coverage = Coverage::default();
  for iteration in 0..MESSAGES {
    let eol = generator.rng.pick(EOLS);
    let depth = generator.rng.below(MAX_DEPTH + 1);
    let raw = generator.message(depth, eol);
    check_message(
      &raw,
      &format!("seed {SEED:#x} iteration {iteration}"),
      &mut coverage,
    );
  }
  println!(
    "{MESSAGES} messages: located identity/qp/base64 {:?}, nested messages {}, located parts checked inside them {}",
    coverage.located, coverage.nested_messages, coverage.checked_in_nested
  );
  for (encoding, located) in coverage.located.iter().enumerate() {
    assert!(
      *located >= MIN_LOCATED_PER_KIND,
      "only {located} parts located with transfer encoding {encoding}; the generator no longer covers it"
    );
  }
  assert!(coverage.nested_messages > 0 && coverage.checked_in_nested > 0);
}

fn single_attachment(headers: &str, body: &[u8], eol: &str, closing: &str) -> Vec<u8> {
  let mut raw = format!(
    "From: a@test{eol}Subject: Edge{eol}MIME-Version: 1.0{eol}\
     Content-Type: multipart/mixed; boundary=\"EDGE\"{eol}{eol}\
     --EDGE{eol}Content-Type: text/plain{eol}{eol}Body{eol}\
     --EDGE{eol}{headers}{eol}{eol}"
  )
  .into_bytes();
  raw.extend_from_slice(body);
  raw.extend_from_slice(format!("{eol}--EDGE--{closing}").as_bytes());
  raw
}

fn only_attachment<'x>(parsed: &'x Message<'x>) -> &'x MessagePart<'x> {
  let stored: Vec<_> = stored_part_refs(parsed).collect();
  assert_eq!(stored.len(), 1);
  stored[0]
}

#[test]
fn base64_windows_stop_before_the_boundary_for_every_wrap_width() {
  let mut rng = Prng(SEED);
  let mut located = 0;
  for &width in WRAP_WIDTHS {
    for len in (1..=10).chain([57, 100, 301]) {
      let payload = rng.bytes(len);
      for &eol in EOLS {
        for tail in ["", " ", "\t ", eol, "\r\n\r\n", " \n"] {
          for closing in ["", eol] {
            let body = format!("{}{tail}", wrap(&base64(&payload), width, eol, ""));
            let raw = single_attachment(
              "Content-Type: application/octet-stream\nContent-Transfer-Encoding: base64"
                .replace('\n', eol)
                .as_str(),
              body.as_bytes(),
              eol,
              closing,
            );
            let context = format!("width {width} len {len} eol {eol:?} tail {tail:?}");
            let parsed = MessageParser::default().parse(&raw).unwrap();
            let part = only_attachment(&parsed);
            assert_eq!(part.contents(), payload.as_slice(), "{context}");
            let locator = locate(&raw, part).unwrap_or_else(|| panic!("{context}: not located"));
            assert_eq!(locator.encoding, TransferEncoding::Base64, "{context}");
            assert_window(&raw, locator, part, &context);
            let window = &raw[locator.offset..locator.offset + locator.len];
            assert!(
              !window.contains(&b'-'),
              "{context}: window reaches the boundary"
            );
            assert!(
              window.starts_with(&base64(&payload).as_bytes()[..1]),
              "{context}: window starts off the body"
            );
            located += 1;
          }
        }
      }
    }
  }
  assert!(located > 0);
}

#[test]
fn quoted_printable_windows_hold_for_every_soft_break_offset() {
  let payload = b"ab=c d\x00\xffe\tf==g h".to_vec();
  let encoded = {
    let mut rng = Prng(1);
    let mut out = String::new();
    for &byte in &payload {
      match byte {
        b'!'..=b'~' if byte != b'=' => out.push(char::from(byte)),
        b' ' if rng.chance(50) => out.push(' '),
        _ => out.push_str(&format!("={byte:02X}")),
      }
    }
    out
  };
  let mut located = 0;
  for &eol in EOLS {
    for soft_break in ["=\r\n", "=\n"] {
      for offset in 0..=encoded.len() {
        for closing in ["", eol] {
          let body = format!("{}{soft_break}{}", &encoded[..offset], &encoded[offset..]);
          let raw = single_attachment(
            &"Content-Type: application/octet-stream\nContent-Transfer-Encoding: quoted-printable"
              .replace('\n', eol),
            body.as_bytes(),
            eol,
            closing,
          );
          let context = format!("eol {eol:?} soft break {soft_break:?} at {offset}");
          let parsed = MessageParser::default().parse(&raw).unwrap();
          let part = only_attachment(&parsed);
          if let Some(locator) = locate(&raw, part) {
            assert_eq!(locator.encoding, TransferEncoding::QuotedPrintable);
            assert_window(&raw, locator, part, &context);
            located += 1;
          }
        }
      }
    }
  }
  assert!(located > 0, "no soft-break variant was located");
}
