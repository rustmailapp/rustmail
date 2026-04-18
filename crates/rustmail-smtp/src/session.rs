use std::borrow::Cow;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::ServerConfig as RustlsServerConfig;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tracing::{debug, warn};

use crate::message::ReceivedMessage;
use crate::server::TlsConfig;

const OK: &str = "250 OK\r\n";
const DATA_START: &str = "354 Start mail input; end with <CRLF>.<CRLF>\r\n";
const QUIT_RESPONSE: &str = "221 Bye\r\n";
const RSET_OK: &str = "250 Reset OK\r\n";
const UNKNOWN_CMD: &str = "500 Unknown command\r\n";
const BAD_SEQUENCE: &str = "503 Bad sequence of commands\r\n";
const STARTTLS_READY: &str = "220 Ready to start TLS\r\n";
const MAX_LINE_LENGTH: usize = 4096;
const MAX_RECIPIENTS: usize = 100;
const MAX_COMMANDS: usize = 1000;

enum SmtpStream {
  Plain(TcpStream),
  Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for SmtpStream {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    match self.get_mut() {
      Self::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
      Self::Tls(stream) => Pin::new(stream.as_mut()).poll_read(cx, buf),
    }
  }
}

impl AsyncWrite for SmtpStream {
  fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
    match self.get_mut() {
      Self::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
      Self::Tls(stream) => Pin::new(stream.as_mut()).poll_write(cx, buf),
    }
  }

  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    match self.get_mut() {
      Self::Plain(stream) => Pin::new(stream).poll_flush(cx),
      Self::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(cx),
    }
  }

  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    match self.get_mut() {
      Self::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
      Self::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(cx),
    }
  }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
  #[error("I/O error: {0}")]
  Io(#[from] std::io::Error),
  #[error("Message exceeds maximum size")]
  MessageTooLarge,
  #[error("Line exceeds maximum length")]
  LineTooLong,
  #[error("TLS upgrade failed: {0}")]
  TlsUpgrade(String),
}

pub struct Session {
  stream: Option<BufReader<SmtpStream>>,
  peer: SocketAddr,
  sender: mpsc::Sender<ReceivedMessage>,
  max_message_size: usize,
  tls: Option<Arc<RustlsServerConfig>>,
  greeted: bool,
  mail_from: Option<String>,
  rcpt_to: Vec<String>,
}

impl Session {
  pub fn new(
    stream: TcpStream,
    peer: SocketAddr,
    sender: mpsc::Sender<ReceivedMessage>,
    max_message_size: usize,
    tls: Option<TlsConfig>,
  ) -> Self {
    Self {
      stream: Some(BufReader::new(SmtpStream::Plain(stream))),
      peer,
      sender,
      max_message_size,
      tls: tls.map(|config| config.server_config),
      greeted: false,
      mail_from: None,
      rcpt_to: Vec::new(),
    }
  }

  pub async fn handle(&mut self) -> Result<(), SessionError> {
    debug!(peer = %self.peer, "New SMTP connection");
    self.write("220 rustmail ESMTP ready\r\n").await?;

    let mut line = String::new();
    let mut command_count: usize = 0;
    loop {
      line.clear();
      let bytes_read = self.read_bounded_line(&mut line).await?;
      if bytes_read == 0 {
        debug!(peer = %self.peer, "Client disconnected");
        return Ok(());
      }

      command_count += 1;
      if command_count > MAX_COMMANDS {
        self.write("421 Too many commands\r\n").await?;
        return Ok(());
      }

      let trimmed = line.trim();
      let log_cmd = redact_auth(trimmed);
      debug!(peer = %self.peer, cmd = %log_cmd, "Received");

      let upper = trimmed.to_ascii_uppercase();

      if upper.starts_with("EHLO") || upper.starts_with("HELO") {
        self.greeted = true;
        self.reset_transaction();
        let ehlo = self.ehlo_response();
        self.write(&ehlo).await?;
      } else if upper == "STARTTLS" {
        if self.tls.is_none() {
          self
            .write("454 TLS not available due to temporary reason\r\n")
            .await?;
        } else if self.tls_active() {
          self.write(BAD_SEQUENCE).await?;
        } else {
          self.write(STARTTLS_READY).await?;
          self.upgrade_to_tls().await?;
        }
      } else if upper.starts_with("MAIL FROM:") {
        if !self.greeted {
          self.write(BAD_SEQUENCE).await?;
        } else {
          self.mail_from = Some(extract_address(trimmed));
          self.rcpt_to.clear();
          self.write(OK).await?;
        }
      } else if upper.starts_with("RCPT TO:") {
        if self.rcpt_to.len() >= MAX_RECIPIENTS {
          self.write("452 Too many recipients\r\n").await?;
        } else {
          self.rcpt_to.push(extract_address(trimmed));
          self.write(OK).await?;
        }
      } else if upper == "DATA" {
        if self.mail_from.is_none() || self.rcpt_to.is_empty() {
          self.write(BAD_SEQUENCE).await?;
        } else {
          self.write(DATA_START).await?;
          match self.receive_data().await {
            Ok(()) => {}
            Err(SessionError::MessageTooLarge) => {
              self.write("552 Message exceeds maximum size\r\n").await?;
            }
            Err(e) => return Err(e),
          }
        }
      } else if upper == "QUIT" {
        self.write(QUIT_RESPONSE).await?;
        return Ok(());
      } else if upper == "RSET" {
        self.reset_transaction();
        self.write(RSET_OK).await?;
      } else if upper.starts_with("AUTH PLAIN") {
        self.handle_auth_plain(trimmed).await?;
      } else if upper.starts_with("AUTH LOGIN") {
        self.handle_auth_login().await?;
      } else if upper.starts_with("AUTH") {
        self
          .write("504 Unrecognized authentication type\r\n")
          .await?;
      } else if upper == "NOOP" {
        self.write(OK).await?;
      } else {
        warn!(peer = %self.peer, cmd = trimmed, "Unknown SMTP command");
        self.write(UNKNOWN_CMD).await?;
      }
    }
  }

  fn ehlo_response(&self) -> String {
    let starttls = if self.tls.is_some() && !self.tls_active() {
      "250-STARTTLS\r\n"
    } else {
      ""
    };

    format!(
      "250-rustmail\r\n250-SIZE {}\r\n250-8BITMIME\r\n250-PIPELINING\r\n{}250-AUTH PLAIN LOGIN\r\n250 HELP\r\n",
      self.max_message_size, starttls
    )
  }

  fn reset_transaction(&mut self) {
    self.mail_from = None;
    self.rcpt_to.clear();
  }

  fn reset_after_starttls(&mut self) {
    self.greeted = false;
    self.reset_transaction();
  }

  fn tls_active(&self) -> bool {
    matches!(
      self.stream.as_ref().expect("SMTP stream missing").get_ref(),
      SmtpStream::Tls(_)
    )
  }

  async fn upgrade_to_tls(&mut self) -> Result<(), SessionError> {
    let buffered = self.stream.as_ref().expect("SMTP stream missing").buffer();
    if !buffered.is_empty() {
      warn!(peer = %self.peer, buffered = buffered.len(), "Closing session during STARTTLS because buffered plaintext bytes remain");
      return Err(SessionError::TlsUpgrade(
        "buffered plaintext remained before STARTTLS handshake".to_string(),
      ));
    }

    let Some(server_config) = self.tls.clone() else {
      return Err(SessionError::TlsUpgrade("TLS not configured".to_string()));
    };

    let stream = self.stream.take().expect("SMTP stream missing");
    let plain_stream = match stream.into_inner() {
      SmtpStream::Plain(stream) => stream,
      SmtpStream::Tls(_) => {
        return Err(SessionError::TlsUpgrade(
          "connection is already using TLS".to_string(),
        ));
      }
    };

    let acceptor = TlsAcceptor::from(server_config);
    let tls_stream = acceptor.accept(plain_stream).await.map_err(|error| {
      warn!(peer = %self.peer, %error, "SMTP STARTTLS handshake failed");
      SessionError::TlsUpgrade(error.to_string())
    })?;

    self.stream = Some(BufReader::new(SmtpStream::Tls(Box::new(tls_stream))));
    self.reset_after_starttls();
    Ok(())
  }

  async fn handle_auth_plain(&mut self, line: &str) -> Result<(), SessionError> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() == 3 && !parts[2].is_empty() {
      self
        .write("235 2.7.0 Authentication successful\r\n")
        .await?;
    } else {
      self.write("334\r\n").await?;
      let mut creds = String::new();
      self.read_bounded_line(&mut creds).await?;
      self
        .write("235 2.7.0 Authentication successful\r\n")
        .await?;
    }
    Ok(())
  }

  async fn handle_auth_login(&mut self) -> Result<(), SessionError> {
    self.write("334 VXNlcm5hbWU6\r\n").await?;
    let mut username = String::new();
    self.read_bounded_line(&mut username).await?;

    self.write("334 UGFzc3dvcmQ6\r\n").await?;
    let mut password = String::new();
    self.read_bounded_line(&mut password).await?;

    self
      .write("235 2.7.0 Authentication successful\r\n")
      .await?;
    Ok(())
  }

  async fn receive_data(&mut self) -> Result<(), SessionError> {
    let mut data = Vec::with_capacity(8192);
    let mut line_buf = Vec::new();

    loop {
      line_buf.clear();
      let bytes_read = self.read_bounded_line_raw(&mut line_buf).await?;
      if bytes_read == 0 {
        return Ok(());
      }

      let trimmed = line_buf
        .strip_suffix(b"\r\n")
        .or_else(|| line_buf.strip_suffix(b"\n"))
        .unwrap_or(&line_buf);
      if trimmed == b"." {
        break;
      }

      if data.len() + line_buf.len() > self.max_message_size {
        self.drain_data().await;
        return Err(SessionError::MessageTooLarge);
      }

      let content = if line_buf.starts_with(b"..") {
        &line_buf[1..]
      } else {
        &line_buf
      };
      data.extend_from_slice(content);
    }

    let message = ReceivedMessage {
      sender: self.mail_from.clone().unwrap_or_default(),
      recipients: self.rcpt_to.clone(),
      raw: data,
    };

    if self.sender.send(message).await.is_err() {
      warn!(peer = %self.peer, "Channel closed, message not stored");
      self
        .write("451 Requested action aborted: local error in processing\r\n")
        .await?;
    } else {
      self.write(OK).await?;
    }
    self.reset_transaction();
    Ok(())
  }

  async fn drain_data(&mut self) {
    let mut line = Vec::new();
    loop {
      line.clear();
      match self.read_bounded_line_raw(&mut line).await {
        Ok(0) => return,
        Ok(_) => {
          let trimmed = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(&line);
          if trimmed == b"." {
            return;
          }
        }
        Err(_) => return,
      }
    }
  }

  async fn read_bounded_line_raw(&mut self, buf: &mut Vec<u8>) -> Result<usize, SessionError> {
    loop {
      let available = self
        .stream
        .as_mut()
        .expect("SMTP stream missing")
        .fill_buf()
        .await?;
      if available.is_empty() {
        if buf.is_empty() {
          return Ok(0);
        }
        break;
      }
      if let Some(pos) = available.iter().position(|&b| b == b'\n') {
        buf.extend_from_slice(&available[..=pos]);
        let consumed = pos + 1;
        self
          .stream
          .as_mut()
          .expect("SMTP stream missing")
          .consume(consumed);
        break;
      } else {
        buf.extend_from_slice(available);
        let len = available.len();
        self
          .stream
          .as_mut()
          .expect("SMTP stream missing")
          .consume(len);
      }
      if buf.len() > MAX_LINE_LENGTH {
        return Err(SessionError::LineTooLong);
      }
    }
    Ok(buf.len())
  }

  async fn read_bounded_line(&mut self, buf: &mut String) -> Result<usize, SessionError> {
    let mut raw = Vec::new();
    let bytes_read = self.read_bounded_line_raw(&mut raw).await?;
    if bytes_read == 0 {
      return Ok(0);
    }
    let s = String::from_utf8_lossy(&raw).into_owned();
    let len = s.len();
    buf.push_str(&s);
    Ok(len)
  }

  async fn write(&mut self, response: &str) -> Result<(), SessionError> {
    self
      .stream
      .as_mut()
      .expect("SMTP stream missing")
      .get_mut()
      .write_all(response.as_bytes())
      .await?;
    Ok(())
  }
}

fn redact_auth(cmd: &str) -> Cow<'_, str> {
  if cmd.len() > 11 && cmd.as_bytes()[..10].eq_ignore_ascii_case(b"AUTH PLAIN") {
    "AUTH PLAIN [REDACTED]".into()
  } else {
    cmd.into()
  }
}

fn extract_address(line: &str) -> String {
  if let Some(start) = line.find('<')
    && let Some(end) = line.find('>')
  {
    return line[start + 1..end].to_string();
  }
  line
    .split_once(':')
    .map(|x| x.1)
    .unwrap_or("")
    .trim()
    .to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extracts_bracketed_address() {
    assert_eq!(
      extract_address("MAIL FROM:<user@example.com>"),
      "user@example.com"
    );
    assert_eq!(
      extract_address("RCPT TO:<admin@test.org>"),
      "admin@test.org"
    );
  }

  #[test]
  fn extracts_plain_address() {
    assert_eq!(
      extract_address("MAIL FROM: user@example.com"),
      "user@example.com"
    );
  }
}
