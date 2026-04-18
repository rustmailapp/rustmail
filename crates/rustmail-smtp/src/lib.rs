//! Lightweight ESMTP server for capturing outbound emails.
//!
//! This crate provides a tokio-based SMTP server that accepts incoming mail,
//! parses ESMTP commands (EHLO, MAIL FROM, RCPT TO, DATA, AUTH PLAIN/LOGIN),
//! and emits [`ReceivedMessage`] values through a [`tokio::sync::mpsc`] channel.
//!
//! Designed for development and testing environments — all authentication
//! attempts are accepted and no mail is actually delivered.
//!
//! # Architecture
//!
//! Each inbound TCP connection spawns an async session task managed by
//! [`SmtpServer`]. A semaphore caps concurrent sessions at 100 to prevent
//! resource exhaustion.
//!
//! # Example
//!
//! ```no_run
//! use rustmail_smtp::{SmtpServer, SmtpServerConfig};
//! use tokio::sync::mpsc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let (tx, mut rx) = mpsc::channel(256);
//! let server = SmtpServer::new(SmtpServerConfig::default(), tx);
//!
//! // Spawn the server
//! tokio::spawn(async move { server.run().await });
//!
//! // Receive captured emails
//! let msg = rx.recv().await.unwrap();
//! println!("From: {}", msg.sender);
//! # Ok(())
//! # }
//! ```

mod message;
mod server;
mod session;

pub use message::ReceivedMessage;
pub use server::{SmtpServer, SmtpServerConfig, TlsConfig};
#[cfg(feature = "test-util")]
pub use session::{Session, SessionError};
