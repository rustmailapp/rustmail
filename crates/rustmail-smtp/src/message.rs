use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{Context, Poll};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::RecvError;

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

const AWAITING_WRITE: u8 = 0;
const WRITE_BEGUN: u8 = 1;
const WITHDRAWN: u8 = 2;

/// Which side settled a delivery first: the consumer, by beginning its
/// write, or the session, by giving up on it.
///
/// Settling is one compare-and-swap, so exactly one side wins. A session
/// that withdraws a message knows its write will never begin, and a consumer
/// that begins a write knows the session will wait for its outcome.
#[derive(Debug, Default)]
struct Claim(AtomicU8);

impl Claim {
  fn settle(&self, state: u8) -> bool {
    self
      .0
      .compare_exchange(AWAITING_WRITE, state, Ordering::AcqRel, Ordering::Acquire)
      .is_ok()
  }
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
  claim: Arc<Claim>,
}

impl Delivery {
  /// Pairs `message` with a fresh acknowledgement channel.
  pub fn new(message: ReceivedMessage) -> (Self, Verdict) {
    let (ack, outcome) = oneshot::channel();
    let claim = Arc::new(Claim::default());
    let verdict = Verdict {
      outcome,
      claim: Arc::clone(&claim),
    };
    (
      Self {
        message,
        ack,
        claim,
      },
      verdict,
    )
  }

  /// Splits the delivery into the message and the handle that answers for it.
  pub fn into_parts(self) -> (ReceivedMessage, DeliveryAck) {
    (
      self.message,
      DeliveryAck {
        ack: self.ack,
        claim: self.claim,
      },
    )
  }
}

/// The session's side of a [`Delivery`]: resolves to the consumer's outcome.
///
/// Resolves to an error when the consumer drops the delivery unanswered.
#[derive(Debug)]
pub struct Verdict {
  outcome: oneshot::Receiver<DeliveryOutcome>,
  claim: Arc<Claim>,
}

impl Verdict {
  /// Withdraws the message unless its write has already begun.
  ///
  /// Returns `true` when the write will now never begin, so the sender can
  /// be told to retry without the message being captured twice. Returns
  /// `false` when the write is in flight: only its outcome can answer then.
  pub fn withdraw(&self) -> bool {
    self.claim.settle(WITHDRAWN)
  }
}

impl Future for Verdict {
  type Output = Result<DeliveryOutcome, RecvError>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    Pin::new(&mut self.outcome).poll(cx)
  }
}

/// The consumer's one chance to say whether a captured message was stored.
///
/// Dropping it without answering reads as [`DeliveryOutcome::Rejected`]: a
/// consumer that went away mid-write cannot vouch for the message, and a
/// sender told to retry is better served than one told a lie.
#[derive(Debug)]
pub struct DeliveryAck {
  ack: oneshot::Sender<DeliveryOutcome>,
  claim: Arc<Claim>,
}

impl DeliveryAck {
  /// Claims the message for a write about to begin.
  ///
  /// Returns `false` when the session already withdrew it or stopped
  /// waiting: it has told the sender to retry, so storing the message would
  /// capture it twice. Once this returns `true`, the session waits for
  /// [`Self::stored`] or [`Self::rejected`] however long the write takes.
  pub fn begin_write(&self) -> bool {
    !self.ack.is_closed() && self.claim.settle(WRITE_BEGUN)
  }

  /// Reports that the message reached storage.
  pub fn stored(self) {
    let _ = self.ack.send(DeliveryOutcome::Stored);
  }

  /// Reports that the message did not reach storage.
  pub fn rejected(self) {
    let _ = self.ack.send(DeliveryOutcome::Rejected);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample() -> ReceivedMessage {
    ReceivedMessage {
      sender: "alice@test.com".to_string(),
      recipients: vec!["bob@test.com".to_string()],
      raw: b"Subject: hi\r\n\r\nbody\r\n".to_vec(),
    }
  }

  #[test]
  fn a_withdrawn_message_cannot_begin_its_write() {
    let (delivery, verdict) = Delivery::new(sample());
    let (_, ack) = delivery.into_parts();

    assert!(verdict.withdraw());
    assert!(!ack.begin_write());
  }

  #[test]
  fn a_write_in_flight_cannot_be_withdrawn() {
    let (delivery, verdict) = Delivery::new(sample());
    let (_, ack) = delivery.into_parts();

    assert!(ack.begin_write());
    assert!(!verdict.withdraw());
  }

  #[test]
  fn a_session_that_stopped_waiting_leaves_nothing_to_write() {
    let (delivery, verdict) = Delivery::new(sample());
    let (_, ack) = delivery.into_parts();

    drop(verdict);

    assert!(!ack.begin_write());
  }
}
