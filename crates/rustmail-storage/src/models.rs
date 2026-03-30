use serde::{Deserialize, Serialize, Serializer};

/// A fully-loaded email message including parsed bodies and raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
  /// ULID identifier (time-sortable).
  pub id: String,
  /// MAIL FROM address.
  pub sender: String,
  /// JSON-encoded array of RCPT TO addresses.
  #[serde(serialize_with = "serialize_json_string_as_array")]
  pub recipients: String,
  /// Parsed Subject header, if present.
  pub subject: Option<String>,
  /// Extracted plain-text body, if present.
  pub text_body: Option<String>,
  /// Extracted HTML body, if present.
  pub html_body: Option<String>,
  /// Raw RFC 5322 bytes (excluded from JSON serialization).
  #[serde(skip_serializing)]
  pub raw: Vec<u8>,
  /// Size of the raw message in bytes.
  pub size: i64,
  /// Whether the message contains attachments.
  pub has_attachments: bool,
  /// Whether the message has been marked as read.
  pub is_read: bool,
  /// Whether the message has been starred.
  pub is_starred: bool,
  /// JSON-encoded array of user-assigned tags.
  #[serde(serialize_with = "serialize_json_string_as_array")]
  pub tags: String,
  /// ISO 8601 timestamp of when the message was received.
  pub created_at: String,
}

/// Lightweight message representation for list/search results.
///
/// Omits bodies and raw bytes to reduce payload size.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MessageSummary {
  pub id: String,
  pub sender: String,
  #[serde(serialize_with = "serialize_json_string_as_array")]
  pub recipients: String,
  pub subject: Option<String>,
  pub size: i64,
  pub has_attachments: bool,
  pub is_read: bool,
  pub is_starred: bool,
  #[serde(serialize_with = "serialize_json_string_as_array")]
  pub tags: String,
  pub created_at: String,
}

/// Lightweight attachment metadata for list responses (no binary content).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AttachmentSummary {
  pub id: String,
  pub message_id: String,
  pub filename: Option<String>,
  pub content_type: Option<String>,
  pub content_id: Option<String>,
  pub size: Option<i64>,
}

/// An email attachment extracted from a parsed message.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Attachment {
  /// ULID identifier.
  pub id: String,
  /// Parent message ID (foreign key).
  pub message_id: String,
  /// Original filename from Content-Disposition, if present.
  pub filename: Option<String>,
  /// MIME type (e.g., `image/png`).
  pub content_type: Option<String>,
  /// Content-ID for inline parts (e.g., `image001@01D7AB12`).
  pub content_id: Option<String>,
  /// Size in bytes.
  pub size: Option<i64>,
  /// Raw attachment bytes (excluded from JSON serialization).
  #[serde(skip_serializing)]
  pub content: Vec<u8>,
}

fn serialize_json_string_as_array<S: Serializer>(
  json_str: &str,
  serializer: S,
) -> Result<S::Ok, S::Error> {
  let tags: Vec<String> = serde_json::from_str(json_str).unwrap_or_else(|e| {
    tracing::warn!(error = %e, raw = %json_str, "Failed to deserialize JSON array field");
    Vec::new()
  });
  tags.serialize(serializer)
}
