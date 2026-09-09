use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// A raw email captured by the SMTP server.
///
/// Carried inside a [`Delivery`] after a successful DATA command. Contains the
/// envelope information (sender, recipients) and the complete RFC 5322 message
/// bytes for downstream parsing and storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedMessage {
  /// MAIL FROM address from the SMTP envelope.
  pub sender: String,
  /// RCPT TO addresses from the SMTP envelope.
  pub recipients: Vec<String>,
  /// Raw RFC 5322 message bytes (headers + body).
  pub raw: Vec<u8>,
}

/// What became of a captured message once its consumer tried to store it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
  /// The message reached storage, so the session may answer `250`.
  Stored,
  /// The message did not reach storage, so the session must not claim it did.
  Rejected,
}

/// A captured message, together with the channel its verdict travels back on.
///
/// The SMTP session waits for that verdict before replying, which is what
/// makes a `250` mean the message was stored rather than merely handed over.
/// A catcher that answered on the hand-off would lose a message whenever the
/// write failed, having already told the sender there was nothing to retry.
#[derive(Debug)]
pub struct Delivery {
  message: ReceivedMessage,
  ack: oneshot::Sender<DeliveryOutcome>,
}

impl Delivery {
  /// Pairs `message` with a fresh acknowledgement channel.
  pub fn new(message: ReceivedMessage) -> (Self, oneshot::Receiver<DeliveryOutcome>) {
    let (ack, verdict) = oneshot::channel();
    (Self { message, ack }, verdict)
  }

  /// Splits the delivery into the message and the handle that answers for it.
  pub fn into_parts(self) -> (ReceivedMessage, DeliveryAck) {
    (self.message, DeliveryAck(self.ack))
  }
}

/// The consumer's one chance to say whether a captured message was stored.
///
/// Dropping it without answering reads as [`DeliveryOutcome::Rejected`]: a
/// consumer that went away mid-write cannot vouch for the message, and a
/// sender told to retry is better served than one told a lie.
#[derive(Debug)]
pub struct DeliveryAck(oneshot::Sender<DeliveryOutcome>);

impl DeliveryAck {
  /// Reports that the message reached storage.
  pub fn stored(self) {
    let _ = self.0.send(DeliveryOutcome::Stored);
  }

  /// Reports that the message did not reach storage.
  pub fn rejected(self) {
    let _ = self.0.send(DeliveryOutcome::Rejected);
  }
}
