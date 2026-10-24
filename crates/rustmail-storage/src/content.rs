//! A message's stored content: its raw source as one zstd frame, and the
//! bodies parsed from it.

use std::io::Read;

use mail_parser::{Message, MessageParser};

use crate::StorageError;
use crate::schema::RAW_COMPRESSION_LEVEL;

/// The text and HTML bodies of a message, as the API serves them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Bodies {
  pub(crate) text: Option<String>,
  pub(crate) html: Option<String>,
}

impl Bodies {
  /// The bodies of `parsed`: its first text and first HTML body, each
  /// converted from the other when the message lacks it.
  pub(crate) fn of(parsed: &Message<'_>) -> Self {
    Self {
      text: parsed.body_text(0).map(|body| body.into_owned()),
      html: parsed.body_html(0).map(|body| body.into_owned()),
    }
  }

  /// The bodies of the raw source `raw`, or none if the parser cannot read it.
  pub(crate) fn parse(raw: &[u8]) -> Self {
    MessageParser::default()
      .parse(raw)
      .map_or_else(Self::default, |parsed| Self::of(&parsed))
  }
}

/// Compresses a raw source into the frame `message_content.raw` keeps.
///
/// # Errors
///
/// Returns [`StorageError::Compress`] if zstd refuses the input.
pub(crate) fn compress(raw: &[u8]) -> Result<Vec<u8>, StorageError> {
  zstd::bulk::compress(raw, RAW_COMPRESSION_LEVEL).map_err(StorageError::Compress)
}

/// Decompresses the stored frame of message `message_id`, which must hold
/// exactly `size` bytes.
///
/// The frame header must declare that size before anything is allocated, so
/// a damaged `size` cannot make it reserve more than the frame holds.
///
/// # Errors
///
/// Returns [`StorageError::ContentCorrupt`] if the frame does not declare or
/// decompress to `size` bytes.
pub(crate) fn decompress(
  frame: &[u8],
  size: i64,
  message_id: &str,
) -> Result<Vec<u8>, StorageError> {
  let corrupt = |reason: String| StorageError::ContentCorrupt {
    message_id: message_id.to_string(),
    reason,
  };
  let capacity =
    usize::try_from(size).map_err(|_| corrupt(format!("recorded size {size} is negative")))?;
  match zstd::zstd_safe::get_frame_content_size(frame) {
    Ok(Some(declared)) if declared == capacity as u64 => {}
    Ok(Some(declared)) => {
      return Err(corrupt(format!(
        "frame declares {declared} bytes, recorded size is {size}"
      )));
    }
    Ok(None) => return Err(corrupt("frame declares no content size".to_string())),
    Err(_) => return Err(corrupt("frame header is unreadable".to_string())),
  }
  let raw = zstd::bulk::decompress(frame, capacity).map_err(|error| corrupt(error.to_string()))?;
  if raw.len() != capacity {
    return Err(corrupt(format!(
      "decompressed to {} bytes, recorded size is {size}",
      raw.len()
    )));
  }
  Ok(raw)
}

/// Decompresses at most `max_bytes` from the start of message `message_id`'s
/// stored frame, without decompressing the rest.
///
/// # Errors
///
/// Returns [`StorageError::ContentCorrupt`] if the frame does not decompress.
pub(crate) fn decompress_prefix(
  frame: &[u8],
  max_bytes: u64,
  message_id: &str,
) -> Result<Vec<u8>, StorageError> {
  let mut prefix = Vec::new();
  zstd::stream::read::Decoder::with_buffer(frame)
    .and_then(|decoder| decoder.take(max_bytes).read_to_end(&mut prefix))
    .map_err(|error| StorageError::ContentCorrupt {
      message_id: message_id.to_string(),
      reason: error.to_string(),
    })?;
  Ok(prefix)
}

#[cfg(test)]
mod tests {
  use super::*;

  const RAW: &[u8] =
    b"Subject: hi\r\nContent-Type: text/plain\r\n\r\nHello there, this is the body.";

  #[test]
  fn a_compressed_source_decompresses_to_itself() {
    let frame = compress(RAW).unwrap();

    assert_eq!(decompress(&frame, RAW.len() as i64, "m").unwrap(), RAW);
  }

  #[test]
  fn a_frame_that_does_not_match_its_recorded_size_is_corrupt() {
    let frame = compress(RAW).unwrap();

    let error = decompress(&frame, RAW.len() as i64 - 1, "m").unwrap_err();

    assert!(
      matches!(error, StorageError::ContentCorrupt { .. }),
      "{error:?}"
    );
  }

  #[test]
  fn a_recorded_size_larger_than_the_frame_declares_is_corrupt_before_allocating() {
    let frame = compress(RAW).unwrap();

    let error = decompress(&frame, i64::MAX, "m").unwrap_err();

    assert!(
      matches!(error, StorageError::ContentCorrupt { .. }),
      "{error:?}"
    );
  }

  #[test]
  fn garbage_is_corrupt() {
    let error = decompress(b"not a zstd frame", 16, "m").unwrap_err();

    assert!(
      matches!(error, StorageError::ContentCorrupt { .. }),
      "{error:?}"
    );
  }

  #[test]
  fn a_prefix_stops_at_its_limit() {
    let frame = compress(RAW).unwrap();

    assert_eq!(decompress_prefix(&frame, 11, "m").unwrap(), b"Subject: hi");
    assert_eq!(decompress_prefix(&frame, 10_000, "m").unwrap(), RAW);
  }

  #[test]
  fn bodies_come_from_the_parser() {
    let bodies = Bodies::parse(RAW);

    assert_eq!(
      bodies.text.as_deref(),
      Some("Hello there, this is the body.")
    );
    assert!(
      bodies.html.is_some(),
      "the parser converts a text-only body to HTML"
    );
  }
}
