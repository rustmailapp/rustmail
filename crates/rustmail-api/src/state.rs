use rustmail_storage::MessageRepository;
use std::sync::Arc;
use tokio::sync::{Semaphore, broadcast};

const MAX_WS_CONNECTIONS: usize = 50;

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

/// Shared application state passed to all axum handlers.
#[derive(Clone)]
pub struct AppState {
  /// Message storage repository.
  pub repo: MessageRepository,
  /// Broadcast sender for WebSocket events.
  pub ws_tx: Arc<broadcast::Sender<WsEvent>>,
  /// Allowed SMTP host for email release (if configured).
  pub release_host: Option<String>,
  /// Allowed SMTP port for email release.
  pub release_port: Option<u16>,
  /// Semaphore limiting concurrent WebSocket connections.
  pub ws_semaphore: Arc<Semaphore>,
}

impl AppState {
  /// Creates a new application state.
  pub fn new(
    repo: MessageRepository,
    ws_tx: broadcast::Sender<WsEvent>,
    release_host: Option<String>,
    release_port: Option<u16>,
  ) -> Self {
    Self {
      repo,
      ws_tx: Arc::new(ws_tx),
      release_host,
      release_port,
      ws_semaphore: Arc::new(Semaphore::new(MAX_WS_CONNECTIONS)),
    }
  }

  /// Sends an event to all connected WebSocket clients.
  pub fn broadcast(&self, event: WsEvent) {
    let _ = self.ws_tx.send(event);
  }
}
