//! Rendering responses into stable text and checking it against the
//! committed snapshots.
//!
//! Only two things are rewritten before comparing: ids minted at insert
//! (message and attachment ULIDs), which map to `{msg:<name>}` and
//! `{att:<name>/<n>}`, and `created_at`-style timestamps, which become
//! `{timestamp}` only when they have exactly the `YYYY-MM-DDTHH:MM:SSZ`
//! shape. Everything else is compared byte for byte: JSON is re-indented
//! only when it is compact, so the indentation is reversible, and any other
//! body is either named as the corpus blob it equals or shown escaped.
//!
//! Set `UPDATE_GOLDENS=1` to rewrite the snapshots from the current code;
//! it is refused when `CI` is set.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::mime::fnv1a64;

const UPDATE_ENV: &str = "UPDATE_GOLDENS";
const CI_ENV: &str = "CI";
const SNAPSHOT_DIR: &str = "tests/goldens/snapshots";
const TIMESTAMP_TOKEN: &str = "{timestamp}";
const TIMESTAMP_SHAPE: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";
const TEXT_RENDER_LIMIT: usize = 64 * 1024;
const HEX_RENDER_LIMIT: usize = 4 * 1024;
const HEX_BYTES_PER_LINE: usize = 32;
const INDENT: &str = "  ";

/// Rewrites the non-deterministic parts of a rendering into stable tokens.
#[derive(Default)]
pub struct Normalizer {
  replacements: Vec<(String, String)>,
}

impl Normalizer {
  /// Renders every occurrence of `id` as `token`.
  pub fn map(&mut self, id: &str, token: String) {
    self.replacements.push((id.to_string(), token));
  }

  /// `text` with every mapped id and every timestamp replaced.
  pub fn apply(&self, text: &str) -> String {
    let mapped = self
      .replacements
      .iter()
      .fold(text.to_string(), |acc, (id, token)| acc.replace(id, token));
    replace_timestamps(&mapped)
  }
}

fn replace_timestamps(text: &str) -> String {
  let bytes = text.as_bytes();
  let mut out = String::with_capacity(text.len());
  let mut start = 0;
  let mut index = 0;
  while index + TIMESTAMP_SHAPE.len() <= bytes.len() {
    if is_timestamp(&bytes[index..index + TIMESTAMP_SHAPE.len()]) {
      out.push_str(&text[start..index]);
      out.push_str(TIMESTAMP_TOKEN);
      index += TIMESTAMP_SHAPE.len();
      start = index;
    } else {
      index += 1;
    }
  }
  out.push_str(&text[start..]);
  out
}

fn is_timestamp(window: &[u8]) -> bool {
  window
    .iter()
    .zip(TIMESTAMP_SHAPE)
    .all(|(byte, shape)| match shape {
      b'd' => byte.is_ascii_digit(),
      literal => byte == literal,
    })
}

/// A named byte string a response body may be identified as.
pub struct KnownBlob<'a> {
  pub label: String,
  pub bytes: &'a [u8],
}

/// Renders `body` so equal bytes always render equally and different bytes
/// never do.
pub fn render_body(
  body: &[u8],
  content_type: Option<&str>,
  known: &[KnownBlob<'_>],
  normalizer: &Normalizer,
) -> String {
  if body.is_empty() {
    return "body: (empty)\n".to_string();
  }
  if let Some(blob) = known.iter().find(|blob| blob.bytes == body) {
    return format!("body: == {} ({} bytes)\n", blob.label, body.len());
  }
  let is_json = content_type.is_some_and(|ct| ct.starts_with("application/json"));
  match std::str::from_utf8(body) {
    Ok(text) if is_json && text.len() <= TEXT_RENDER_LIMIT => match indent_compact_json(text) {
      Some(pretty) => format!("body-json:\n{}\n", normalizer.apply(&pretty)),
      None => format!("body-text:\n{}", escaped_lines(&normalizer.apply(text))),
    },
    Ok(text) if text.len() <= TEXT_RENDER_LIMIT => {
      format!("body-text:\n{}", escaped_lines(&normalizer.apply(text)))
    }
    _ if body.len() <= HEX_RENDER_LIMIT => {
      format!("body-hex ({} bytes):\n{}", body.len(), hex_lines(body))
    }
    _ => format!(
      "body: {} bytes, fnv1a64 {:016x}, matches no corpus blob\n",
      body.len(),
      fnv1a64(body)
    ),
  }
}

/// Indents compact JSON, or `None` if `text` has whitespace outside strings,
/// in which case indenting would no longer be reversible.
fn indent_compact_json(text: &str) -> Option<String> {
  let mut out = String::with_capacity(text.len() * 2);
  let mut depth = 0usize;
  let mut in_string = false;
  let mut escaped = false;
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    if in_string {
      out.push(c);
      match (escaped, c) {
        (true, _) => escaped = false,
        (false, '\\') => escaped = true,
        (false, '"') => in_string = false,
        _ => {}
      }
      continue;
    }
    match c {
      '"' => {
        in_string = true;
        out.push(c);
      }
      '{' | '[' => {
        out.push(c);
        let closes = matches!(chars.peek(), Some('}') | Some(']'));
        if closes {
          continue;
        }
        depth += 1;
        newline(&mut out, depth);
      }
      '}' | ']' => {
        let empty = out.ends_with('{') || out.ends_with('[');
        if !empty {
          depth = depth.checked_sub(1)?;
          newline(&mut out, depth);
        }
        out.push(c);
      }
      ',' => {
        out.push(c);
        newline(&mut out, depth);
      }
      ':' => out.push_str(": "),
      c if c.is_whitespace() => return None,
      c => out.push(c),
    }
  }
  Some(out)
}

fn newline(out: &mut String, depth: usize) {
  out.push('\n');
  for _ in 0..depth {
    out.push_str(INDENT);
  }
}

/// One `| `-prefixed line per source line, with `\r`, `\n`, `\t`, `\\` and
/// other control characters escaped so the snapshot keeps the exact bytes.
pub fn escaped_lines(text: &str) -> String {
  let mut out = String::new();
  for line in text.split_inclusive('\n') {
    out.push_str("| ");
    for c in line.chars() {
      match c {
        '\\' => out.push_str("\\\\"),
        '\r' => out.push_str("\\r"),
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        c if c.is_control() => {
          let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
        }
        c => out.push(c),
      }
    }
    out.push('\n');
  }
  out
}

fn hex_lines(bytes: &[u8]) -> String {
  let mut out = String::new();
  for line in bytes.chunks(HEX_BYTES_PER_LINE) {
    out.push_str("| ");
    for byte in line {
      let _ = write!(out, "{byte:02x}");
    }
    out.push('\n');
  }
  out
}

fn snapshot_path(name: &str) -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join(SNAPSHOT_DIR)
    .join(format!("{name}.txt"))
}

/// Compares `actual` with the committed snapshot `name`, or rewrites the
/// snapshot when `UPDATE_GOLDENS=1`.
pub fn assert_golden(name: &str, actual: &str) {
  let path = snapshot_path(name);
  if std::env::var(UPDATE_ENV).is_ok_and(|value| value == "1") {
    assert!(
      std::env::var_os(CI_ENV).is_none(),
      "{UPDATE_ENV}=1 is refused when {CI_ENV} is set: goldens are regenerated locally and reviewed"
    );
    std::fs::create_dir_all(path.parent().expect("snapshot path has a parent"))
      .expect("create snapshot dir");
    std::fs::write(&path, actual.as_bytes()).expect("write snapshot");
    return;
  }
  let expected = std::fs::read(&path).unwrap_or_else(|error| {
    panic!(
      "golden {name} could not be read from {}: {error}; generate it with {UPDATE_ENV}=1 cargo test -p rustmail-api --test goldens",
      path.display()
    )
  });
  let expected = String::from_utf8(expected).expect("snapshots are UTF-8");
  if expected == actual {
    return;
  }
  let actual_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("golden-{name}.txt"));
  std::fs::write(&actual_path, actual.as_bytes()).expect("write actual rendering");
  let (line, want, got) = first_difference(&expected, actual);
  panic!(
    "golden {name} differs at line {line}\n  expected: {want}\n  actual:   {got}\nfull actual output: {}\ndiff it against {}; if the change is intended, rerun with {UPDATE_ENV}=1 and review the snapshot diff",
    actual_path.display(),
    path.display()
  );
}

fn first_difference<'a>(expected: &'a str, actual: &'a str) -> (usize, &'a str, &'a str) {
  let mut want = expected.lines();
  let mut got = actual.lines();
  let mut line = 1;
  loop {
    match (want.next(), got.next()) {
      (Some(a), Some(b)) if a == b => line += 1,
      (a, b) => {
        return (
          line,
          a.unwrap_or("<end of file>"),
          b.unwrap_or("<end of file>"),
        );
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn timestamps_of_the_exact_shape_are_replaced() {
    assert_eq!(
      replace_timestamps(r#"{"a":"2026-09-23T09:00:00Z","b":"2026-09-23 09:00:00Z"}"#),
      r#"{"a":"{timestamp}","b":"2026-09-23 09:00:00Z"}"#
    );
  }

  #[test]
  fn compact_json_indents_and_keeps_strings_intact() {
    assert_eq!(
      indent_compact_json(r#"{"a":[1,{}],"b":"x, {y}","c":[]}"#).as_deref(),
      Some("{\n  \"a\": [\n    1,\n    {}\n  ],\n  \"b\": \"x, {y}\",\n  \"c\": []\n}")
    );
  }

  #[test]
  fn json_with_insignificant_whitespace_is_not_indented() {
    assert_eq!(indent_compact_json(r#"{"a": 1}"#), None);
  }

  #[test]
  fn escaped_lines_keep_line_endings_visible() {
    assert_eq!(escaped_lines("a\r\nb\\\x01"), "| a\\r\\n\n| b\\\\\\u{1}\n");
  }
}
