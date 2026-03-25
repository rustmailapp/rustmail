use serde::{Deserialize, Serialize};

/// A raw email captured by the SMTP server.
///
/// Emitted via broadcast channel after a successful DATA command.
/// Contains the envelope information (sender, recipients) and the
/// complete RFC 5322 message bytes for downstream parsing and storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedMessage {
  /// MAIL FROM address from the SMTP envelope.
  pub sender: String,
  /// RCPT TO addresses from the SMTP envelope.
  pub recipients: Vec<String>,
  /// Raw RFC 5322 message bytes (headers + body).
  pub raw: Vec<u8>,
}
