use std::net::SocketAddr;
use std::sync::Arc;

use rustls::ServerConfig as RustlsServerConfig;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, mpsc};
use tracing::{error, info, warn};

use crate::message::Delivery;
use crate::session::Session;

const MAX_CONCURRENT_SESSIONS: usize = 100;

#[derive(Debug, Clone)]
pub struct TlsConfig {
  pub server_config: Arc<RustlsServerConfig>,
}

/// Configuration for the SMTP capture server.
#[derive(Debug, Clone)]
pub struct SmtpServerConfig {
  /// IP address to bind to (IPv4 or IPv6).
  pub host: std::net::IpAddr,
  /// TCP port to listen on (default: 1025).
  pub port: u16,
  /// Maximum accepted message size in bytes (default: 10 MiB).
  pub max_message_size: usize,
  /// Optional TLS configuration for STARTTLS support.
  pub tls: Option<TlsConfig>,
}

impl Default for SmtpServerConfig {
  fn default() -> Self {
    Self {
      host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
      port: 1025,
      max_message_size: 10 * 1024 * 1024,
      tls: None,
    }
  }
}

/// Async SMTP server that captures inbound mail and broadcasts it.
///
/// Listens for TCP connections and spawns a session task for each.
/// Concurrent sessions are capped at 100 via a semaphore.
///
/// A session is not capped in total duration. Every await inside one already
/// carries its own deadline — commands and responses through the per-line I/O
/// timeout, message bodies through the DATA phase timeout, STARTTLS through the
/// handshake timeout — so a stalled peer is cut off without a blanket limit,
/// and one that keeps delivering is not disconnected mid-send.
pub struct SmtpServer {
  config: SmtpServerConfig,
  sender: mpsc::Sender<Delivery>,
}

impl SmtpServer {
  /// Creates a new server with the given configuration and broadcast sender.
  pub fn new(config: SmtpServerConfig, sender: mpsc::Sender<Delivery>) -> Self {
    Self { config, sender }
  }

  /// Runs the SMTP server, accepting connections until the future is dropped.
  ///
  /// # Errors
  ///
  /// Returns an error if the TCP listener cannot bind to the configured address.
  pub async fn run(&self) -> Result<(), std::io::Error> {
    let addr = SocketAddr::new(self.config.host, self.config.port);
    let listener = TcpListener::bind(addr).await?;
    info!(addr = %addr, "SMTP server listening");

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_SESSIONS));

    loop {
      match listener.accept().await {
        Ok((mut stream, peer)) => {
          let permit = match semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
              warn!(peer = %peer, "SMTP connection rejected: max concurrent sessions reached");
              let _ = stream
                .write_all(b"421 Service not available, too many connections\r\n")
                .await;
              continue;
            }
          };
          let sender = self.sender.clone();
          let max_size = self.config.max_message_size;
          let tls = self.config.tls.clone();
          tokio::spawn(async move {
            let mut session = Session::new(stream, peer, sender, max_size, tls);
            if let Err(e) = session.handle().await {
              error!(peer = %peer, error = %e, "SMTP session error");
            }
            drop(permit);
          });
        }
        Err(e) => {
          error!(error = %e, "Failed to accept TCP connection");
        }
      }
    }
  }
}
