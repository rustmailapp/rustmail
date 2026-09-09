use rustmail_storage::MessageRepository;
use std::sync::Arc;
use tokio::sync::{Semaphore, broadcast};

use crate::host::Hostname;
use crate::origin::Origin;

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
  /// Origins allowed to open the WebSocket on top of the server's own.
  pub allowed_origins: Arc<[Origin]>,
  /// Host names RustMail answers a browser on, besides addresses and `localhost`.
  pub allowed_hosts: Arc<[Hostname]>,
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
      allowed_origins: Arc::from([]),
      allowed_hosts: Arc::from([]),
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
    if let Err(e) = self.ws_tx.send(event) {
      tracing::debug!(event = ?e.0, "No active WebSocket subscribers");
    }
  }
}
