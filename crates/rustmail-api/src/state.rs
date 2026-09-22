use axum::extract::ws::Utf8Bytes;
use rustmail_storage::MessageRepository;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, broadcast};

use crate::host::Hostname;
use crate::origin::Origin;
use crate::ws::WsTimings;

const MAX_WS_CONNECTIONS: usize = 50;
/// Longest an API request may run before it is answered `503`.
///
/// Well past any read or bulk delete on a 100k mailbox, so it only cuts off a
/// request stuck behind a busy writer, and past the 15 s the UI waits, so the
/// server stops working for a client that has already given up.
const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Events sent to WebSocket clients in real time.
///
/// Serialized as JSON with `{"type": "message:new", "data": ...}` format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsEvent {
  /// A new email was received and stored.
  #[serde(rename = "message:new")]
  MessageNew(rustmail_storage::MessageSummary),
  /// A message was deleted.
  #[serde(rename = "message:delete")]
  MessageDelete { id: String },
  /// A message's read state changed.
  #[serde(rename = "message:read")]
  MessageRead { id: String, is_read: bool },
  /// A message's starred state changed.
  #[serde(rename = "message:starred")]
  MessageStarred { id: String, is_starred: bool },
  /// A message's tags were updated.
  #[serde(rename = "message:tags")]
  MessageTags { id: String, tags: Vec<String> },
  /// All messages were cleared.
  #[serde(rename = "messages:clear")]
  MessagesClear,
}

/// A [`WsEvent`] serialized once, as every subscriber receives it.
///
/// The broadcast channel carries frames rather than events so that each event
/// is encoded a single time however many clients are connected; cloning a
/// frame only bumps a reference count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsFrame(Utf8Bytes);

/// Why a [`WsFrame`] could not be encoded or decoded.
#[derive(Debug, thiserror::Error)]
pub enum WsFrameError {
  #[error("WebSocket event could not be encoded: {0}")]
  Encode(#[source] serde_json::Error),
  #[error("WebSocket frame could not be decoded: {0}")]
  Decode(#[source] serde_json::Error),
}

impl WsFrame {
  /// Encodes `event` in its wire format.
  pub fn encode(event: &WsEvent) -> Result<Self, WsFrameError> {
    serde_json::to_string(event)
      .map(|json| Self(json.into()))
      .map_err(WsFrameError::Encode)
  }

  /// Decodes the event this frame carries.
  pub fn decode(&self) -> Result<WsEvent, WsFrameError> {
    serde_json::from_str(self.0.as_str()).map_err(WsFrameError::Decode)
  }

  pub(crate) fn text(&self) -> Utf8Bytes {
    self.0.clone()
  }
}

/// Shared application state passed to all axum handlers.
#[derive(Clone)]
pub struct AppState {
  /// Message storage repository.
  pub repo: MessageRepository,
  /// Broadcast sender for serialized WebSocket events.
  pub ws_tx: Arc<broadcast::Sender<WsFrame>>,
  /// Allowed SMTP host for email release (if configured).
  pub release_host: Option<String>,
  /// Allowed SMTP port for email release.
  pub release_port: Option<u16>,
  /// Semaphore limiting concurrent WebSocket connections.
  pub ws_semaphore: Arc<Semaphore>,
  /// Origins allowed to open the WebSocket on top of the server's own.
  pub allowed_origins: Arc<[Origin]>,
  /// Host names RustMail answers a browser on, besides addresses and `localhost`.
  pub allowed_hosts: Arc<[Hostname]>,
  pub(crate) ws_timings: WsTimings,
  pub(crate) api_timeout: Duration,
}

impl AppState {
  /// Creates a new application state.
  pub fn new(
    repo: MessageRepository,
    ws_tx: broadcast::Sender<WsFrame>,
    release_host: Option<String>,
    release_port: Option<u16>,
  ) -> Self {
    Self {
      repo,
      ws_tx: Arc::new(ws_tx),
      release_host,
      release_port,
      ws_semaphore: Arc::new(Semaphore::new(MAX_WS_CONNECTIONS)),
      allowed_origins: Arc::from([]),
      allowed_hosts: Arc::from([]),
      ws_timings: WsTimings::default(),
      api_timeout: API_REQUEST_TIMEOUT,
    }
  }

  /// Lets browser pages on `origins` open the WebSocket.
  ///
  /// The origin the server itself is reached at always may, so this is only
  /// needed when the UI reaches RustMail through a reverse proxy that does not
  /// forward the public `Host`, or from a separate front-end origin.
  pub fn with_allowed_origins(mut self, origins: Vec<Origin>) -> Self {
    self.allowed_origins = Arc::from(origins);
    self
  }

  /// Lets browsers reach RustMail on `hosts`.
  ///
  /// Addresses and `localhost` are always answered, so this is only needed for
  /// a name — a reverse proxy's public name, a Docker service name, a `.local`
  /// name — which is exactly what a DNS rebinding attack has to supply.
  pub fn with_allowed_hosts(mut self, hosts: Vec<Hostname>) -> Self {
    self.allowed_hosts = Arc::from(hosts);
    self
  }

  /// Sends an event to all connected WebSocket clients.
  pub fn broadcast(&self, event: WsEvent) {
    let frame = match WsFrame::encode(&event) {
      Ok(frame) => frame,
      Err(error) => {
        tracing::warn!(%error, event = ?event, "WebSocket event could not be serialized, not sent");
        return;
      }
    };
    if self.ws_tx.send(frame).is_err() {
      tracing::debug!(event = ?event, "No active WebSocket subscribers");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{WsEvent, WsFrame, WsFrameError};

  fn wire(event: &WsEvent) -> String {
    WsFrame::encode(event).unwrap().text().as_str().to_owned()
  }

  #[test]
  fn frames_keep_the_tagged_wire_format() {
    let cases = [
      (
        WsEvent::MessageDelete { id: "a".into() },
        r#"{"type":"message:delete","data":{"id":"a"}}"#,
      ),
      (
        WsEvent::MessageRead {
          id: "a".into(),
          is_read: true,
        },
        r#"{"type":"message:read","data":{"id":"a","is_read":true}}"#,
      ),
      (
        WsEvent::MessageStarred {
          id: "a".into(),
          is_starred: false,
        },
        r#"{"type":"message:starred","data":{"id":"a","is_starred":false}}"#,
      ),
      (
        WsEvent::MessageTags {
          id: "a".into(),
          tags: vec!["x".into(), "y".into()],
        },
        r#"{"type":"message:tags","data":{"id":"a","tags":["x","y"]}}"#,
      ),
      (WsEvent::MessagesClear, r#"{"type":"messages:clear"}"#),
    ];

    for (event, expected) in cases {
      assert_eq!(wire(&event), expected, "wire format changed for {event:?}");
    }
  }

  #[test]
  fn a_frame_decodes_back_to_its_event() {
    let event = WsEvent::MessageTags {
      id: "a".into(),
      tags: vec!["x".into()],
    };
    let decoded = WsFrame::encode(&event).unwrap().decode().unwrap();

    assert_eq!(wire(&decoded), wire(&event));
  }

  #[test]
  fn a_malformed_frame_decodes_to_a_ws_frame_error() {
    let frame = WsFrame(r#"{"type":"not-an-event"}"#.into());

    let error = frame.decode().unwrap_err();

    assert!(matches!(error, WsFrameError::Decode(_)));
  }
}
