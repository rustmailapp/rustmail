//! Deterministic byte sources and MIME transfer encoders for the corpus.
//!
//! Written here rather than taken from a crate so the corpus depends on
//! nothing that could change its bytes under it.

const BASE64_ALPHABET: &[u8; 64] =
  b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_PAD: u8 = b'=';
const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
const SPLITMIX_MUL_1: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX_MUL_2: u64 = 0x94d0_49bb_1331_11eb;
/// Longest encoded line RFC 2045 allows, before the soft line break.
const QP_MAX_LINE: usize = 76;

/// The 64-bit FNV-1a digest of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
  bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
    (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
  })
}

/// A SplitMix64 generator seeded from a label, so every payload is
/// reproducible from its name alone.
pub struct Prng(u64);

impl Prng {
  /// A generator whose stream depends only on `label`.
  pub fn seeded(label: &str) -> Self {
    Self(fnv1a64(label.as_bytes()))
  }

  fn next_u64(&mut self) -> u64 {
    self.0 = self.0.wrapping_add(SPLITMIX_INCREMENT);
    let mut z = self.0;
    z = (z ^ (z >> 30)).wrapping_mul(SPLITMIX_MUL_1);
    z = (z ^ (z >> 27)).wrapping_mul(SPLITMIX_MUL_2);
    z ^ (z >> 31)
  }

  /// `len` pseudo-random bytes.
  pub fn bytes(&mut self, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
      out.extend_from_slice(&self.next_u64().to_le_bytes());
    }
    out.truncate(len);
    out
  }
}

/// Standard padded base64 on one line.
pub fn base64(bytes: &[u8]) -> String {
  let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let b0 = chunk[0];
    let b1 = chunk.get(1).copied().unwrap_or(0);
    let b2 = chunk.get(2).copied().unwrap_or(0);
    let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
    let sextet = |shift: u32| BASE64_ALPHABET[((triple >> shift) & 0x3f) as usize];
    out.push(sextet(18));
    out.push(sextet(12));
    out.push(if chunk.len() > 1 {
      sextet(6)
    } else {
      BASE64_PAD
    });
    out.push(if chunk.len() > 2 {
      sextet(0)
    } else {
      BASE64_PAD
    });
  }
  String::from_utf8(out).expect("base64 output is ASCII")
}

/// Splits `encoded` into lines of `width` characters joined by `eol`.
///
/// The last line has no `eol`: in a MIME part the line break before the
/// next boundary belongs to the delimiter, not to the content.
pub fn wrap(encoded: &str, width: usize, eol: &str) -> String {
  encoded
    .as_bytes()
    .chunks(width)
    .map(|line| std::str::from_utf8(line).expect("wrapped input is ASCII"))
    .collect::<Vec<_>>()
    .join(eol)
}

/// Quoted-printable for binary content: every byte outside printable ASCII,
/// `=`, and all whitespace are escaped, so the encoding carries no line
/// structure of its own and soft breaks keep lines within 76 columns. Like
/// [`wrap`], the last line has no `eol`.
pub fn quoted_printable_binary(bytes: &[u8], eol: &str) -> String {
  let mut out = String::new();
  let mut line_len = 0;
  for byte in bytes {
    let literal = byte.is_ascii_graphic() && *byte != b'=';
    let token_len = if literal { 1 } else { 3 };
    if line_len + token_len > QP_MAX_LINE - 1 {
      out.push('=');
      out.push_str(eol);
      line_len = 0;
    }
    if literal {
      out.push(char::from(*byte));
    } else {
      out.push('=');
      out.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
      out.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    line_len += token_len;
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn base64_matches_the_rfc_4648_vectors() {
    let vectors: [(&[u8], &str); 7] = [
      (b"", ""),
      (b"f", "Zg=="),
      (b"fo", "Zm8="),
      (b"foo", "Zm9v"),
      (b"foob", "Zm9vYg=="),
      (b"fooba", "Zm9vYmE="),
      (b"foobar", "Zm9vYmFy"),
    ];
    for (input, expected) in vectors {
      assert_eq!(base64(input), expected);
    }
  }

  #[test]
  fn quoted_printable_escapes_whitespace_and_equals() {
    assert_eq!(
      quoted_printable_binary(b"a= \r\n\xff", "\r\n"),
      "a=3D=20=0D=0A=FF"
    );
  }

  #[test]
  fn quoted_printable_lines_stay_within_76_columns() {
    let encoded = quoted_printable_binary(&[0u8; 200], "\r\n");
    assert!(encoded.split("\r\n").all(|line| line.len() <= QP_MAX_LINE));
  }

  #[test]
  fn the_prng_is_a_pure_function_of_its_label() {
    assert_eq!(Prng::seeded("x").bytes(33), Prng::seeded("x").bytes(33));
    assert_ne!(Prng::seeded("x").bytes(33), Prng::seeded("y").bytes(33));
  }
}
