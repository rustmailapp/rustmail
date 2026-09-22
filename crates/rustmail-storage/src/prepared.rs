use std::collections::HashSet;

use mail_parser::{ContentType, MessageParser, MimeHeaders, PartType};

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
  pub(crate) content: Vec<u8>,
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
          stored_parts(&parsed),
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

fn stored_parts(parsed: &mail_parser::Message<'_>) -> Vec<PreparedAttachment> {
  let attachment_ids: HashSet<u32> = parsed.attachments.iter().copied().collect();
  parsed
    .parts
    .iter()
    .enumerate()
    .filter(|(idx, part)| {
      let is_attachment = u32::try_from(*idx).is_ok_and(|idx| attachment_ids.contains(&idx));
      is_attachment || matches!(part.body, PartType::InlineBinary(_))
    })
    .filter(|(_, part)| !part.contents().is_empty())
    .map(|(_, part)| PreparedAttachment {
      filename: part.attachment_name().map(String::from),
      content_type: part.content_type().map(mime_type),
      content_id: part.content_id().map(String::from),
      content: part.contents().to_vec(),
    })
    .collect()
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
    assert_eq!(attachment.content, b"fake-pdf-content");
    assert_eq!(prepared.raw, MULTIPART.as_bytes());
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
