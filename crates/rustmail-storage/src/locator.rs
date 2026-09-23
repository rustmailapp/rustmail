use std::ops::Range;

use mail_parser::decoders::base64::base64_decode;
use mail_parser::decoders::quoted_printable::quoted_printable_decode;
use mail_parser::{Encoding, MessagePart, MimeHeaders, PartType};

/// Base64 characters that make up one quantum.
const BASE64_QUANTUM_CHARS: u8 = 4;
/// Bytes one complete base64 quantum decodes to.
const BASE64_QUANTUM_BYTES: usize = 3;

/// How a located part's body is encoded in the raw message.
///
/// Stored in `attachments.transfer_encoding`; the codes mirror
/// [`mail_parser::Encoding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferEncoding {
  /// 7bit, 8bit or binary: the body is served as it sits in the raw message.
  Identity,
  /// `quoted-printable`.
  QuotedPrintable,
  /// `base64`.
  Base64,
}

impl TransferEncoding {
  /// The code stored in `attachments.transfer_encoding`.
  pub(crate) const fn code(self) -> i64 {
    match self {
      Self::Identity => 0,
      Self::QuotedPrintable => 1,
      Self::Base64 => 2,
    }
  }

  /// The encoding a stored code names, or `None` for a code no rustmail writes.
  pub(crate) const fn from_code(code: i64) -> Option<Self> {
    match code {
      0 => Some(Self::Identity),
      1 => Some(Self::QuotedPrintable),
      2 => Some(Self::Base64),
      _ => None,
    }
  }

  const fn of(encoding: Encoding) -> Self {
    match encoding {
      Encoding::None => Self::Identity,
      Encoding::QuotedPrintable => Self::QuotedPrintable,
      Encoding::Base64 => Self::Base64,
    }
  }

  /// Decodes a located body with mail-parser's own slice decoders.
  ///
  /// Returns `None` when the body does not decode, which for a located part
  /// means the locator or the raw message no longer matches what was
  /// verified at ingest.
  pub(crate) fn decode(self, body: &[u8]) -> Option<Vec<u8>> {
    match self {
      Self::Identity => Some(body.to_vec()),
      Self::QuotedPrintable => quoted_printable_decode(body),
      Self::Base64 => base64_decode(body),
    }
  }
}

/// Where a part's body sits in the raw message, and how to decode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Locator {
  /// Byte offset of the body in the raw message.
  pub(crate) offset: usize,
  /// Length of the encoded body in the raw message.
  pub(crate) len: usize,
  /// How the body is encoded there.
  pub(crate) encoding: TransferEncoding,
}

/// Returns a locator for `part` when serving it from `raw` provably yields
/// the bytes the parser produced for it in context, and `None` when the part
/// has to be stored decoded.
///
/// A part is located only if it is binary or a nested message (text parts are
/// charset-converted by the parser), declares a `Content-Type`, parsed
/// without an encoding problem, and passes the guard for its encoding:
///
/// - identity: the raw slice equals the parsed contents;
/// - quoted-printable: the slice decoder's output equals the parsed contents;
/// - base64: every byte of the slice is base64 alphabet, `=` or whitespace,
///   and the decoded length this predicts equals the parsed length. The
///   slice decoder rejects bytes such as a lone `-` that the in-context
///   decoder skips, so the alphabet check is what keeps both in step.
pub(crate) fn locate(raw: &[u8], part: &MessagePart<'_>) -> Option<Locator> {
  let is_binary_or_message = matches!(
    part.body,
    PartType::Binary(_) | PartType::InlineBinary(_) | PartType::Message(_)
  );
  if !is_binary_or_message || part.content_type().is_none() || part.is_encoding_problem {
    return None;
  }
  let encoding = TransferEncoding::of(part.encoding);
  let range = body_range(part, encoding)?;
  let body = raw.get(range.clone())?;
  let contents = part.contents();
  let decodes_to_contents = match encoding {
    TransferEncoding::Identity => body == contents,
    TransferEncoding::QuotedPrintable => {
      quoted_printable_decode(body).is_some_and(|decoded| decoded == contents)
    }
    TransferEncoding::Base64 => predicted_base64_len(body) == Some(contents.len()),
  };
  decodes_to_contents.then_some(Locator {
    offset: range.start,
    len: range.len(),
    encoding,
  })
}

/// The span of `raw` a part's contents come from.
///
/// An unencoded nested message is parsed in place, and its contents are its
/// own root part, headers included, rather than the outer part's body.
fn body_range(part: &MessagePart<'_>, encoding: TransferEncoding) -> Option<Range<usize>> {
  let (start, end) = match (&part.body, encoding) {
    (PartType::Message(nested), TransferEncoding::Identity) => {
      let root = nested.parts.first()?;
      (root.offset_header, root.offset_end)
    }
    _ => (part.offset_body, part.offset_end),
  };
  let start = usize::try_from(start).ok()?;
  let end = usize::try_from(end).ok()?;
  (start <= end).then_some(start..end)
}

/// The length mail-parser's base64 decoders produce for `body`, or `None` if
/// `body` holds a byte other than base64 alphabet, `=` or whitespace.
///
/// Mirrors the decoders' state machine: a complete quantum yields three
/// bytes, `=` flushes a partial one and restarts, and a trailing partial
/// quantum is dropped.
fn predicted_base64_len(body: &[u8]) -> Option<usize> {
  let mut decoded_len = 0;
  let mut pending_chars = 0;
  for &byte in body {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
        pending_chars = (pending_chars + 1) % BASE64_QUANTUM_CHARS;
        if pending_chars == 0 {
          decoded_len += BASE64_QUANTUM_BYTES;
        }
      }
      b'=' => {
        decoded_len += match pending_chars {
          1 | 2 => 1,
          3 => 2,
          _ => 0,
        };
        pending_chars = 0;
      }
      b' ' | b'\t' | b'\r' | b'\n' => {}
      _ => return None,
    }
  }
  Some(decoded_len)
}

#[cfg(test)]
mod tests {
  use super::*;
  use mail_parser::MessageParser;

  const BASE64_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

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
  }

  fn attachment_message(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
      "From: a@test.com\r\nTo: b@test.com\r\nSubject: Locate\r\nMIME-Version: 1.0\r\n\
       Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
       --B\r\nContent-Type: text/plain\r\n\r\nBody\r\n\
       --B\r\n{headers}\r\n\r\n"
    )
    .into_bytes();
    raw.extend_from_slice(body);
    raw.extend_from_slice(b"\r\n--B--\r\n");
    raw
  }

  fn attachment_locator(raw: &[u8]) -> (Option<Locator>, Vec<u8>) {
    let parsed = MessageParser::default().parse(raw).unwrap();
    let index = *parsed.attachments.first().unwrap() as usize;
    let part = &parsed.parts[index];
    (locate(raw, part), part.contents().to_vec())
  }

  #[test]
  fn the_predicted_length_matches_the_slice_decoder() {
    let mut rng = Prng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..2000 {
      let len = rng.below(64);
      let body: Vec<u8> = (0..len)
        .map(|_| match rng.below(10) {
          0 => b'=',
          1 => b'\r',
          2 => b'\n',
          _ => BASE64_ALPHABET[rng.below(BASE64_ALPHABET.len())],
        })
        .collect();
      assert_eq!(
        predicted_base64_len(&body),
        base64_decode(&body).map(|decoded| decoded.len()),
        "prediction diverged on {:?}",
        String::from_utf8_lossy(&body)
      );
    }
  }

  #[test]
  fn a_lone_dash_is_outside_the_alphabet() {
    assert_eq!(predicted_base64_len(b"QUJD-REVG"), None);
    assert_eq!(base64_decode(b"QUJD-REVG"), None);
  }

  #[test]
  fn a_base64_part_with_a_stray_dash_is_not_located() {
    let raw = attachment_message(
      "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64",
      b"QUJDREVG-R0hJ",
    );
    let (locator, contents) = attachment_locator(&raw);
    assert_eq!(contents, b"ABCDEFGHI");
    assert_eq!(locator, None);
  }

  #[test]
  fn a_clean_base64_part_is_located_on_its_encoded_body() {
    let raw = attachment_message(
      "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64",
      b"QUJDREVG\r\nR0hJ",
    );
    let (locator, contents) = attachment_locator(&raw);
    let locator = locator.unwrap();
    assert_eq!(locator.encoding, TransferEncoding::Base64);
    let body = &raw[locator.offset..locator.offset + locator.len];
    assert_eq!(body, b"QUJDREVG\r\nR0hJ");
    assert_eq!(TransferEncoding::Base64.decode(body).unwrap(), contents);
  }

  #[test]
  fn an_identity_slice_stops_before_the_boundary_line_break() {
    let raw = attachment_message("Content-Type: application/octet-stream", b"\x00\x01payload");
    let (locator, contents) = attachment_locator(&raw);
    let locator = locator.unwrap();
    assert_eq!(&raw[locator.offset..locator.offset + locator.len], contents);
    assert_eq!(contents, b"\x00\x01payload");
  }

  #[test]
  fn a_forged_slice_carrying_the_trailing_line_break_fails_the_guard() {
    let raw = attachment_message("Content-Type: application/octet-stream", b"payload");
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let index = *parsed.attachments.first().unwrap() as usize;
    let mut forged = parsed.parts[index].clone();
    forged.offset_end += 2;
    assert_eq!(
      &raw[forged.offset_end as usize - 2..forged.offset_end as usize],
      b"\r\n"
    );
    assert_eq!(locate(&raw, &forged), None);
  }

  #[test]
  fn a_part_without_a_content_type_is_not_located() {
    let raw = attachment_message(
      "Content-Disposition: attachment; filename=\"notes.bin\"",
      b"notes",
    );
    let (locator, _) = attachment_locator(&raw);
    assert_eq!(locator, None);
  }

  #[test]
  fn stored_codes_round_trip() {
    for encoding in [
      TransferEncoding::Identity,
      TransferEncoding::QuotedPrintable,
      TransferEncoding::Base64,
    ] {
      assert_eq!(TransferEncoding::from_code(encoding.code()), Some(encoding));
    }
    assert_eq!(TransferEncoding::from_code(3), None);
  }
}
