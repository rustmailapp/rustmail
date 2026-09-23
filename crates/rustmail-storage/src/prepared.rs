use std::collections::HashSet;

use mail_parser::{ContentType, MessageParser, MessagePart, MimeHeaders, PartType};

use crate::locator::{Locator, locate};

/// A captured message parsed into the rows it will be stored as.
///
/// Parsing is pure and, for large MIME bodies, the most expensive CPU work
/// on the ingest path. Doing it once, ahead of the write, means a contended
/// insert retries only the write, and the single writer never waits on MIME
/// decoding.
#[derive(Debug, Clone)]
pub struct PreparedMessage {
  pub(crate) sender: String,
  pub(crate) recipients_json: String,
  pub(crate) subject: Option<String>,
  pub(crate) text_body: Option<String>,
  pub(crate) html_body: Option<String>,
  pub(crate) raw: Vec<u8>,
  pub(crate) has_attachments: bool,
  pub(crate) attachments: Vec<PreparedAttachment>,
}

/// A MIME part stored as a row of its own: a declared attachment or an inline
/// binary part.
#[derive(Debug, Clone)]
pub(crate) struct PreparedAttachment {
  pub(crate) filename: Option<String>,
  pub(crate) content_type: Option<String>,
  pub(crate) content_id: Option<String>,
  /// Length of the decoded contents, whichever way they are stored.
  pub(crate) size: usize,
  pub(crate) storage: AttachmentStorage,
}

/// How a part's contents are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttachmentStorage {
  /// Decoded on request from the raw message, which already holds them.
  Located(Locator),
  /// Stored decoded, because serving them from the raw message could not be
  /// proven to reproduce the parser's output.
  Inline(Vec<u8>),
}

impl PreparedMessage {
  /// Parses `raw` and extracts everything the store keeps besides the bytes.
  ///
  /// Never fails: mail the parser cannot read is still stored, with no
  /// subject, bodies or attachments, so its raw source stays inspectable.
  pub fn parse(sender: String, recipients: &[String], raw: Vec<u8>) -> Self {
    let recipients_json = serde_json::to_string(recipients).unwrap_or_default();
    let (subject, text_body, html_body, has_attachments, attachments) =
      match MessageParser::default().parse(&raw) {
        Some(parsed) => (
          parsed.subject().map(String::from),
          parsed.body_text(0).map(|body| body.into_owned()),
          parsed.body_html(0).map(|body| body.into_owned()),
          parsed.attachment_count() > 0,
          stored_parts(&parsed, &raw),
        ),
        None => (None, None, None, false, Vec::new()),
      };
    Self {
      sender,
      recipients_json,
      subject,
      text_body,
      html_body,
      raw,
      has_attachments,
      attachments,
    }
  }
}

fn stored_parts(parsed: &mail_parser::Message<'_>, raw: &[u8]) -> Vec<PreparedAttachment> {
  stored_part_refs(parsed)
    .map(|part| PreparedAttachment {
      filename: part.attachment_name().map(String::from),
      content_type: part.content_type().map(mime_type),
      content_id: part.content_id().map(String::from),
      size: part_contents(part).len(),
      storage: locate(raw, part).map_or_else(
        || AttachmentStorage::Inline(part_contents(part).to_vec()),
        AttachmentStorage::Located,
      ),
    })
    .collect()
}

/// The parts of `parsed` stored as attachment rows, in message order: each
/// declared attachment and inline binary part with non-empty contents.
pub(crate) fn stored_part_refs<'p, 'x>(
  parsed: &'p mail_parser::Message<'x>,
) -> impl Iterator<Item = &'p MessagePart<'x>> {
  let attachment_ids: HashSet<u32> = parsed.attachments.iter().copied().collect();
  parsed
    .parts
    .iter()
    .enumerate()
    .filter(move |(idx, part)| {
      let is_attachment = u32::try_from(*idx).is_ok_and(|idx| attachment_ids.contains(&idx));
      is_attachment || matches!(part.body, PartType::InlineBinary(_))
    })
    .map(|(_, part)| part)
    .filter(|part| !part_contents(part).is_empty())
}

/// The part's decoded contents, as [`MessagePart::contents`] returns them.
///
/// A nested message the parser recovered with no parts at all counts as
/// empty: `contents` would index its missing root part and panic.
pub(crate) fn part_contents<'p>(part: &'p MessagePart<'_>) -> &'p [u8] {
  match &part.body {
    PartType::Message(nested) if nested.parts.is_empty() => b"",
    _ => part.contents(),
  }
}

fn mime_type(content_type: &ContentType<'_>) -> String {
  match content_type.subtype() {
    Some(subtype) => format!("{}/{}", content_type.ctype(), subtype),
    None => content_type.ctype().to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const MULTIPART: &str = concat!(
    "From: sender@test.com\r\n",
    "To: rcpt@test.com\r\n",
    "Subject: Quarterly\r\n",
    "MIME-Version: 1.0\r\n",
    "Content-Type: multipart/mixed; boundary=\"BOUNDARY\"\r\n",
    "\r\n",
    "--BOUNDARY\r\n",
    "Content-Type: text/plain\r\n",
    "\r\n",
    "Body text\r\n",
    "--BOUNDARY\r\n",
    "Content-Type: application/pdf\r\n",
    "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
    "\r\n",
    "fake-pdf-content\r\n",
    "--BOUNDARY--\r\n",
  );

  #[test]
  fn extracts_headers_bodies_and_attachments() {
    let prepared = PreparedMessage::parse(
      "sender@test.com".to_string(),
      &["rcpt@test.com".to_string()],
      MULTIPART.as_bytes().to_vec(),
    );

    assert_eq!(prepared.subject.as_deref(), Some("Quarterly"));
    assert_eq!(prepared.text_body.as_deref(), Some("Body text"));
    assert_eq!(prepared.recipients_json, r#"["rcpt@test.com"]"#);
    assert!(prepared.has_attachments);
    assert_eq!(prepared.attachments.len(), 1);
    let attachment = &prepared.attachments[0];
    assert_eq!(attachment.filename.as_deref(), Some("report.pdf"));
    assert_eq!(attachment.content_type.as_deref(), Some("application/pdf"));
    assert_eq!(attachment.size, b"fake-pdf-content".len());
    let AttachmentStorage::Located(locator) = attachment.storage else {
      panic!("an unencoded binary part should be located");
    };
    assert_eq!(
      &MULTIPART.as_bytes()[locator.offset..locator.offset + locator.len],
      b"fake-pdf-content"
    );
    assert_eq!(prepared.raw, MULTIPART.as_bytes());
  }

  #[test]
  fn a_digest_item_that_is_not_a_message_is_skipped() {
    let raw = concat!(
      "From: sender@test.com\r\n",
      "Subject: Digest\r\n",
      "MIME-Version: 1.0\r\n",
      "Content-Type: multipart/digest; boundary=\"D\"\r\n",
      "\r\n",
      "--D\r\n",
      "Content-Disposition: attachment; filename=\"item.bin\"\r\n",
      "\r\n",
      "caf\u{e9}\r\n",
      "line  =\t--caf\u{e9} beta\r\n",
      "=line= beta\r\n",
      "--D--\r\n",
    );
    let prepared =
      PreparedMessage::parse("sender@test.com".to_string(), &[], raw.as_bytes().to_vec());

    assert!(prepared.attachments.is_empty());
    assert_eq!(prepared.raw, raw.as_bytes());
  }

  #[test]
  fn keeps_mail_the_parser_cannot_read() {
    let raw = Vec::new();
    let prepared = PreparedMessage::parse("a@test.com".to_string(), &[], raw);

    assert_eq!(prepared.subject, None);
    assert!(prepared.attachments.is_empty());
    assert!(!prepared.has_attachments);
  }
}
