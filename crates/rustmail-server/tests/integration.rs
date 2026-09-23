use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{
  AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, Lines,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::ChildStdout;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_rustls::TlsConnector;

use rustmail_api::{AppState, WsEvent, WsFrame, router};
use rustmail_smtp::{Delivery, ReceivedMessage, Session, SmtpServer, SmtpServerConfig, TlsConfig};
use rustmail_storage::{MessageRepository, initialize_database};

#[path = "../../rustmail-storage/tests/common/legacy_v0_7_0.rs"]
mod legacy_v0_7_0;

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
/// Port 0 asks the OS for a free port, one no concurrent test can also be handed.
const ANY_FREE_PORT: &str = "0";
/// Child log filter: warnings, plus the line that reports the SMTP address.
const CHILD_LOG_FILTER: &str = "warn,rustmail_smtp::server=info";
const SMTP_LISTENING_LOG: &str = "SMTP server listening";
const LISTEN_ADDR_FIELD: &str = "addr=";
const LISTEN_REPORT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SMTP_PORT: u16 = 1025;
const STARTTLS_CERT_PATH: &str = concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/tests/fixtures/starttls-cert.pem"
);
const STARTTLS_KEY_PATH: &str = concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/tests/fixtures/starttls-key.pem"
);

fn starttls_cert_path() -> PathBuf {
  PathBuf::from(STARTTLS_CERT_PATH)
}

fn starttls_key_path() -> PathBuf {
  PathBuf::from(STARTTLS_KEY_PATH)
}

fn load_test_tls_config() -> TlsConfig {
  let cert_file = std::fs::File::open(starttls_cert_path()).unwrap();
  let key_file = std::fs::File::open(starttls_key_path()).unwrap();

  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  let certs = CertificateDer::pem_reader_iter(cert_file)
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
  let key = PrivateKeyDer::from_pem_reader(key_file).unwrap();

  let server_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();

  TlsConfig {
    server_config: Arc::new(server_config),
  }
}

fn load_test_cert_der() -> CertificateDer<'static> {
  let cert_file = std::fs::File::open(starttls_cert_path()).unwrap();
  CertificateDer::pem_reader_iter(cert_file)
    .next()
    .unwrap()
    .unwrap()
}

fn test_tls_connector() -> TlsConnector {
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  let mut root_store = rustls::RootCertStore::empty();
  root_store.add(load_test_cert_der()).unwrap();

  let client_config = rustls::ClientConfig::builder()
    .with_root_certificates(root_store)
    .with_no_client_auth();

  TlsConnector::from(Arc::new(client_config))
}

/// Builds a `rustmail` invocation whose log output the tests can parse.
///
/// Colour codes and an inherited `RUST_LOG` would both hide the line that
/// reports the SMTP address, so neither reaches the child.
fn rustmail_command() -> tokio::process::Command {
  let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"));
  command
    .env("NO_COLOR", "1")
    .env_remove("RUST_LOG")
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
  command
}

/// Finds the address in the SMTP server's start-up log line, if this is it.
fn parse_smtp_listen_addr(line: &str) -> Option<SocketAddr> {
  let (_, fields) = line.split_once(SMTP_LISTENING_LOG)?;
  let (_, value) = fields.split_once(LISTEN_ADDR_FIELD)?;
  value.split_whitespace().next()?.parse().ok()
}

struct ChildGuard {
  child: Option<tokio::process::Child>,
  _open_stdout: Option<Lines<BufReader<ChildStdout>>>,
}

impl ChildGuard {
  fn new(child: tokio::process::Child) -> Self {
    Self {
      child: Some(child),
      _open_stdout: None,
    }
  }

  async fn wait_with_timeout(&mut self, secs: u64) -> std::process::ExitStatus {
    let child = self.child.as_mut().expect("child already consumed");
    tokio::time::timeout(Duration::from_secs(secs), child.wait())
      .await
      .expect("child did not exit in time")
      .expect("failed to wait on child")
  }

  /// Kills the child, as a crash or `SIGKILL` would, and waits for it to exit.
  async fn kill(&mut self) {
    let mut child = self.child.take().expect("child already consumed");
    child.kill().await.expect("failed to kill child");
  }

  /// Waits for the child to log the SMTP address it bound.
  ///
  /// Children are started on port 0, so the OS picks a port no other test
  /// holds, and the log line is the only way to learn which one it was. The
  /// line is written after the bind, so the port already accepts connections.
  /// The stdout pipe stays open afterwards so later log writes cannot fail.
  async fn smtp_addr(&mut self) -> SocketAddr {
    let child = self.child.as_mut().expect("child already consumed");
    let mut stdout = BufReader::new(child.stdout.take().expect("child stdout not piped")).lines();
    let mut stderr = child.stderr.take().expect("child stderr not piped");
    let addr = tokio::time::timeout(LISTEN_REPORT_TIMEOUT, async {
      while let Some(line) = stdout
        .next_line()
        .await
        .expect("failed to read child stdout")
      {
        if let Some(addr) = parse_smtp_listen_addr(&line) {
          return addr;
        }
      }
      let mut diagnostics = String::new();
      stderr
        .read_to_string(&mut diagnostics)
        .await
        .expect("failed to read child stderr");
      panic!("rustmail exited before reporting its SMTP address: {diagnostics}");
    })
    .await
    .expect("rustmail did not report its SMTP address in time");
    self._open_stdout = Some(stdout);
    addr
  }
}

impl Drop for ChildGuard {
  fn drop(&mut self) {
    if let Some(ref mut child) = self.child {
      let _ = child.start_kill();
    }
  }
}

async fn test_repo() -> MessageRepository {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  MessageRepository::new(pool)
}

/// Answers every delivery as stored, forwarding the messages for inspection.
///
/// The session now holds its SMTP reply until a verdict comes back, so a test
/// that reads `250` before it touches the channel needs something else
/// answering in the meantime.
fn accept_deliveries(mut rx: mpsc::Receiver<Delivery>) -> mpsc::Receiver<ReceivedMessage> {
  let (tx, out) = mpsc::channel(256);
  tokio::spawn(async move {
    while let Some(delivery) = rx.recv().await {
      let (message, ack) = delivery.into_parts();
      ack.stored();
      if tx.send(message).await.is_err() {
        break;
      }
    }
  });
  out
}

fn spawn_smtp_with_real_session(listener: tokio::net::TcpListener, tx: mpsc::Sender<Delivery>) {
  spawn_smtp_with_real_session_and_tls(listener, tx, None);
}

fn spawn_smtp_with_real_session_and_tls(
  listener: tokio::net::TcpListener,
  tx: mpsc::Sender<Delivery>,
  tls: Option<TlsConfig>,
) {
  tokio::spawn(async move {
    loop {
      let Ok((stream, peer)) = listener.accept().await else {
        break;
      };
      let sender = tx.clone();
      let tls = tls.clone();
      tokio::spawn(async move {
        let mut session = Session::new(stream, peer, sender, MAX_MESSAGE_SIZE, tls);
        if let Err(e) = session.handle().await {
          eprintln!("SMTP session error: {e}");
        }
      });
    }
  });
}

async fn read_smtp_response_line<S>(stream: &mut BufReader<S>) -> String
where
  S: AsyncRead + AsyncWrite + Unpin,
{
  let mut line = String::new();
  let bytes = stream.read_line(&mut line).await.unwrap();
  assert!(bytes > 0, "expected SMTP response line");
  line
}

async fn read_ehlo_response<S>(stream: &mut BufReader<S>) -> String
where
  S: AsyncRead + AsyncWrite + Unpin,
{
  stream.write_all(b"EHLO test\r\n").await.unwrap();

  let mut response = String::new();
  loop {
    let line = read_smtp_response_line(stream).await;
    response.push_str(&line);
    if line.starts_with("250 ") {
      break;
    }
  }

  response
}

async fn read_banner(stream: &mut BufReader<TcpStream>) -> String {
  read_smtp_response_line(stream).await
}

/// Sends one message and quits without requiring the QUIT reply, because
/// `rustmail assert` exits once its condition holds and may reset the socket first.
async fn smtp_send(addr: std::net::SocketAddr, from: &str, to: &str, subject: &str, body: &str) {
  let mut stream = TcpStream::connect(addr).await.unwrap();
  let mut buf = vec![0u8; 4096];

  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"EHLO test\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream
    .write_all(format!("MAIL FROM:<{from}>\r\n").as_bytes())
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream
    .write_all(format!("RCPT TO:<{to}>\r\n").as_bytes())
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"DATA\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  let data = format!(
    "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\n{body}\r\n.\r\n"
  );
  stream.write_all(data.as_bytes()).await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"QUIT\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await;
}

async fn wait_for_count(repo: &MessageRepository, expected: i64) {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
  loop {
    let count = repo.count().await.unwrap();
    if count >= expected {
      return;
    }
    if tokio::time::Instant::now() > deadline {
      panic!("Timed out waiting for {expected} message(s), got {count}");
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
}

#[tokio::test]
async fn smtp_to_api_pipeline() {
  let smtp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let smtp_addr = smtp_listener.local_addr().unwrap();

  let repo = test_repo().await;
  let (smtp_tx, mut smtp_rx) = mpsc::channel(256);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);

  spawn_smtp_with_real_session(smtp_listener, smtp_tx);

  let repo_clone = repo.clone();
  tokio::spawn(async move {
    while let Some(delivery) = smtp_rx.recv().await {
      let (msg, ack) = delivery.into_parts();
      repo_clone
        .insert(&msg.sender, &msg.recipients, &msg.raw)
        .await
        .unwrap();
      ack.stored();
    }
  });

  let state = AppState::new(repo.clone(), ws_tx, None, None);
  let app = router(state);
  let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let http_addr = http_listener.local_addr().unwrap();
  tokio::spawn(async move {
    axum::serve(http_listener, app).await.unwrap();
  });

  smtp_send(
    smtp_addr,
    "alice@test.com",
    "bob@test.com",
    "Integration Test",
    "Hello from SMTP",
  )
  .await;
  wait_for_count(&repo, 1).await;

  let client = reqwest::Client::new();

  let resp = client
    .get(format!("http://{}/api/v1/messages", http_addr))
    .send()
    .await
    .unwrap();
  assert_eq!(resp.status(), 200);
  let body: serde_json::Value = resp.json().await.unwrap();
  assert_eq!(body["total"], 1);
  let messages = body["messages"].as_array().unwrap();
  assert_eq!(messages[0]["sender"], "alice@test.com");
  assert_eq!(messages[0]["subject"], "Integration Test");

  let id = messages[0]["id"].as_str().unwrap();

  let resp = client
    .get(format!("http://{}/api/v1/messages/{}", http_addr, id))
    .send()
    .await
    .unwrap();
  assert_eq!(resp.status(), 200);
  let msg: serde_json::Value = resp.json().await.unwrap();
  assert_eq!(msg["text_body"], "Hello from SMTP\r\n");

  let resp = client
    .get(format!("http://{}/api/v1/messages/{}/raw", http_addr, id))
    .send()
    .await
    .unwrap();
  assert_eq!(resp.status(), 200);
  assert!(
    resp
      .text()
      .await
      .unwrap()
      .contains("Subject: Integration Test")
  );

  let resp = client
    .get(format!(
      "http://{}/api/v1/assert/count?min=1&subject=Integration",
      http_addr
    ))
    .send()
    .await
    .unwrap();
  assert_eq!(resp.status(), 200);
  let body: serde_json::Value = resp.json().await.unwrap();
  assert_eq!(body["ok"], true);

  let resp = client
    .get(format!(
      "http://{}/api/v1/messages?q=Integration",
      http_addr
    ))
    .send()
    .await
    .unwrap();
  let body: serde_json::Value = resp.json().await.unwrap();
  assert_eq!(body["messages"].as_array().unwrap().len(), 1);

  let resp = client
    .delete(format!("http://{}/api/v1/messages/{}", http_addr, id))
    .send()
    .await
    .unwrap();
  assert_eq!(resp.status(), 204);

  let resp = client
    .get(format!("http://{}/api/v1/messages", http_addr))
    .send()
    .await
    .unwrap();
  let body: serde_json::Value = resp.json().await.unwrap();
  assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn smtp_multiple_messages() {
  let smtp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let smtp_addr = smtp_listener.local_addr().unwrap();

  let repo = test_repo().await;
  let (smtp_tx, mut smtp_rx) = mpsc::channel(256);

  spawn_smtp_with_real_session(smtp_listener, smtp_tx);

  let repo_clone = repo.clone();
  tokio::spawn(async move {
    while let Some(delivery) = smtp_rx.recv().await {
      let (msg, ack) = delivery.into_parts();
      repo_clone
        .insert(&msg.sender, &msg.recipients, &msg.raw)
        .await
        .unwrap();
      ack.stored();
    }
  });

  for i in 0..3 {
    smtp_send(
      smtp_addr,
      &format!("sender{}@test.com", i),
      "rcpt@test.com",
      &format!("Message {}", i),
      &format!("Body {}", i),
    )
    .await;
  }

  wait_for_count(&repo, 3).await;

  let messages = repo.list(50, 0).await.unwrap();
  let subjects: Vec<_> = messages
    .iter()
    .map(|m| m.subject.as_deref().unwrap_or(""))
    .collect();
  assert!(subjects.contains(&"Message 0"));
  assert!(subjects.contains(&"Message 1"));
  assert!(subjects.contains(&"Message 2"));
}

#[tokio::test]
async fn smtp_auth_login_accepted() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = TcpStream::connect(addr).await.unwrap();
  let mut buf = vec![0u8; 4096];

  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).starts_with("220"));

  stream.write_all(b"EHLO test\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  let ehlo = String::from_utf8_lossy(&buf[..n]);
  assert!(ehlo.contains("AUTH PLAIN LOGIN"));

  stream.write_all(b"AUTH LOGIN\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).starts_with("334"));

  stream.write_all(b"dXNlcg==\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).starts_with("334"));

  stream.write_all(b"cGFzcw==\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).starts_with("235"));

  stream.write_all(b"QUIT\r\n").await.unwrap();
}

#[tokio::test]
async fn smtp_auth_plain_inline() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = TcpStream::connect(addr).await.unwrap();
  let mut buf = vec![0u8; 4096];

  let _ = stream.read(&mut buf).await.unwrap();
  stream.write_all(b"EHLO test\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream
    .write_all(b"AUTH PLAIN AGFsaWNlAHBhc3N3b3Jk\r\n")
    .await
    .unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).starts_with("235"));

  stream.write_all(b"QUIT\r\n").await.unwrap();
}

#[tokio::test]
async fn smtp_ehlo_omits_starttls_when_tls_not_configured() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session(listener, tx);

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let banner = read_banner(&mut stream).await;
  assert!(banner.starts_with("220"));

  let ehlo = read_ehlo_response(&mut stream).await;
  assert!(!ehlo.contains("STARTTLS"));
  assert!(ehlo.contains("AUTH PLAIN LOGIN"));
}

#[tokio::test]
async fn smtp_ehlo_advertises_starttls_when_tls_configured() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session_and_tls(listener, tx, Some(load_test_tls_config()));

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let banner = read_banner(&mut stream).await;
  assert!(banner.starts_with("220"));

  let ehlo = read_ehlo_response(&mut stream).await;
  assert!(ehlo.contains("STARTTLS"));
  assert!(ehlo.contains("AUTH PLAIN LOGIN"));
  assert!(ehlo.contains("PIPELINING"));
}

#[tokio::test]
async fn smtp_starttls_upgrades_connection_and_accepts_message() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, rx) = mpsc::channel(256);
  let mut rx = accept_deliveries(rx);
  spawn_smtp_with_real_session_and_tls(listener, tx, Some(load_test_tls_config()));

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let banner = read_banner(&mut stream).await;
  assert!(banner.starts_with("220"));

  let ehlo = read_ehlo_response(&mut stream).await;
  assert!(ehlo.contains("STARTTLS"));

  stream.write_all(b"STARTTLS\r\n").await.unwrap();
  let response = read_smtp_response_line(&mut stream).await;
  assert_eq!(response, "220 Ready to start TLS\r\n");

  let connector = test_tls_connector();
  let server_name = ServerName::try_from("localhost").unwrap();
  let tls_stream = connector
    .connect(server_name, stream.into_inner())
    .await
    .unwrap();
  let mut tls_stream = BufReader::new(tls_stream);

  let tls_ehlo = read_ehlo_response(&mut tls_stream).await;
  assert!(!tls_ehlo.contains("STARTTLS"));
  assert!(tls_ehlo.contains("AUTH PLAIN LOGIN"));

  tls_stream
    .write_all(b"MAIL FROM:<alice@test.com>\r\n")
    .await
    .unwrap();
  assert_eq!(read_smtp_response_line(&mut tls_stream).await, "250 OK\r\n");

  tls_stream
    .write_all(b"RCPT TO:<bob@test.com>\r\n")
    .await
    .unwrap();
  assert_eq!(read_smtp_response_line(&mut tls_stream).await, "250 OK\r\n");

  tls_stream.write_all(b"DATA\r\n").await.unwrap();
  assert!(
    read_smtp_response_line(&mut tls_stream)
      .await
      .starts_with("354 ")
  );

  tls_stream
    .write_all(
      b"From: alice@test.com\r\nTo: bob@test.com\r\nSubject: STARTTLS Test\r\n\r\nHello over TLS\r\n.\r\n",
    )
    .await
    .unwrap();
  assert_eq!(read_smtp_response_line(&mut tls_stream).await, "250 OK\r\n");

  tls_stream.write_all(b"QUIT\r\n").await.unwrap();
  assert_eq!(
    read_smtp_response_line(&mut tls_stream).await,
    "221 Bye\r\n"
  );

  let message = tokio::time::timeout(Duration::from_secs(5), rx.recv())
    .await
    .unwrap()
    .unwrap();
  assert_eq!(message.sender, "alice@test.com");
  assert_eq!(message.recipients, vec!["bob@test.com"]);
  assert!(String::from_utf8_lossy(&message.raw).contains("Subject: STARTTLS Test"));
}

#[tokio::test]
async fn smtp_starttls_resets_session_state() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session_and_tls(listener, tx, Some(load_test_tls_config()));

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let _ = read_banner(&mut stream).await;
  let _ = read_ehlo_response(&mut stream).await;

  stream
    .write_all(b"MAIL FROM:<before@test.com>\r\n")
    .await
    .unwrap();
  assert_eq!(read_smtp_response_line(&mut stream).await, "250 OK\r\n");

  stream.write_all(b"STARTTLS\r\n").await.unwrap();
  assert_eq!(
    read_smtp_response_line(&mut stream).await,
    "220 Ready to start TLS\r\n"
  );

  let connector = test_tls_connector();
  let server_name = ServerName::try_from("localhost").unwrap();
  let tls_stream = connector
    .connect(server_name, stream.into_inner())
    .await
    .unwrap();
  let mut tls_stream = BufReader::new(tls_stream);

  tls_stream.write_all(b"DATA\r\n").await.unwrap();
  assert_eq!(
    read_smtp_response_line(&mut tls_stream).await,
    "503 Bad sequence of commands\r\n"
  );
}

#[tokio::test]
async fn smtp_starttls_rejects_second_upgrade() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _) = mpsc::channel(256);
  spawn_smtp_with_real_session_and_tls(listener, tx, Some(load_test_tls_config()));

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let _ = read_banner(&mut stream).await;
  let _ = read_ehlo_response(&mut stream).await;

  stream.write_all(b"STARTTLS\r\n").await.unwrap();
  assert_eq!(
    read_smtp_response_line(&mut stream).await,
    "220 Ready to start TLS\r\n"
  );

  let connector = test_tls_connector();
  let server_name = ServerName::try_from("localhost").unwrap();
  let tls_stream = connector
    .connect(server_name, stream.into_inner())
    .await
    .unwrap();
  let mut tls_stream = BufReader::new(tls_stream);

  let tls_ehlo = read_ehlo_response(&mut tls_stream).await;
  assert!(!tls_ehlo.contains("STARTTLS"));

  tls_stream.write_all(b"STARTTLS\r\n").await.unwrap();
  assert_eq!(
    read_smtp_response_line(&mut tls_stream).await,
    "503 Bad sequence of commands\r\n"
  );
}

#[tokio::test]
async fn smtp_send_and_receive_via_channel() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let (tx, rx) = mpsc::channel(256);
  let mut rx = accept_deliveries(rx);
  spawn_smtp_with_real_session(listener, tx);

  smtp_send(
    addr,
    "test@example.com",
    "dest@example.com",
    "Channel test",
    "body",
  )
  .await;

  let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
    .await
    .expect("Timed out waiting for message")
    .expect("Channel closed");

  assert_eq!(msg.sender, "test@example.com");
  assert_eq!(msg.recipients, vec!["dest@example.com"]);
  assert!(String::from_utf8_lossy(&msg.raw).contains("Subject: Channel test"));
}

#[tokio::test]
async fn smtp_rset_clears_envelope() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let (tx, rx) = mpsc::channel(256);
  let mut rx = accept_deliveries(rx);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = TcpStream::connect(addr).await.unwrap();
  let mut buf = vec![0u8; 4096];

  let _ = stream.read(&mut buf).await.unwrap();
  stream.write_all(b"EHLO test\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream
    .write_all(b"MAIL FROM:<old@test.com>\r\n")
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();
  stream
    .write_all(b"RCPT TO:<old-rcpt@test.com>\r\n")
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"RSET\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  assert!(String::from_utf8_lossy(&buf[..n]).contains("Reset OK"));

  stream
    .write_all(b"MAIL FROM:<new@test.com>\r\n")
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();
  stream
    .write_all(b"RCPT TO:<new-rcpt@test.com>\r\n")
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"DATA\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();
  stream
    .write_all(b"Subject: After RSET\r\n\r\nbody\r\n.\r\n")
    .await
    .unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  stream.write_all(b"QUIT\r\n").await.unwrap();

  let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
    .await
    .expect("timed out")
    .expect("channel closed");

  assert_eq!(msg.sender, "new@test.com");
  assert_eq!(msg.recipients, vec!["new-rcpt@test.com"]);
}

#[tokio::test]
async fn webhook_fires_on_new_message() {
  let received_payloads: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
  let payloads_clone = received_payloads.clone();

  let mock_app = axum::Router::new().route(
    "/hook",
    axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
      let store = payloads_clone.clone();
      async move {
        store.lock().await.push(body);
        axum::http::StatusCode::OK
      }
    }),
  );
  let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let mock_addr = mock_listener.local_addr().unwrap();
  tokio::spawn(async move {
    axum::serve(mock_listener, mock_app).await.unwrap();
  });
  let webhook_url = format!("http://{}/hook", mock_addr);

  let smtp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let smtp_addr = smtp_listener.local_addr().unwrap();

  let repo = test_repo().await;
  let (smtp_tx, mut smtp_rx) = mpsc::channel(256);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);

  spawn_smtp_with_real_session(smtp_listener, smtp_tx);

  let webhook_client = reqwest::Client::new();
  let webhook_url_clone = webhook_url.clone();
  let repo_clone = repo.clone();
  let state = AppState::new(repo.clone(), ws_tx, None, None);
  tokio::spawn(async move {
    while let Some(delivery) = smtp_rx.recv().await {
      let (received, ack) = delivery.into_parts();
      if let Ok(summary) = repo_clone
        .insert(&received.sender, &received.recipients, &received.raw)
        .await
      {
        ack.stored();
        state.broadcast(WsEvent::MessageNew(summary.clone()));

        let client = webhook_client.clone();
        let url = webhook_url_clone.clone();
        tokio::spawn(async move {
          let _ = client
            .post(&url)
            .json(&summary)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        });
      }
    }
  });

  smtp_send(
    smtp_addr,
    "webhook@test.com",
    "dest@test.com",
    "Webhook Test",
    "Hello webhook",
  )
  .await;

  wait_for_count(&repo, 1).await;

  let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
  loop {
    let payloads = received_payloads.lock().await;
    if !payloads.is_empty() {
      break;
    }
    drop(payloads);
    if tokio::time::Instant::now() > deadline {
      panic!("Timed out waiting for webhook delivery");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  let payloads = received_payloads.lock().await;
  assert_eq!(payloads.len(), 1);
  let payload = &payloads[0];
  assert_eq!(payload["sender"], "webhook@test.com");
  assert_eq!(payload["subject"], "Webhook Test");
  assert!(payload["id"].is_string());
  assert!(payload["created_at"].is_string());
  assert_eq!(payload["is_read"], false);
  assert_eq!(payload["is_starred"], false);
  assert!(payload["tags"].is_array());
}

#[tokio::test]
async fn cli_assert_passes_when_email_arrives() {
  let mut guard = ChildGuard::new(
    rustmail_command()
      .args([
        "assert",
        "--smtp-port",
        ANY_FREE_PORT,
        "--min-count",
        "1",
        "--subject",
        "CLI Test",
        "--timeout",
        "10s",
        "--log-level",
        CHILD_LOG_FILTER,
      ])
      .spawn()
      .expect("failed to spawn rustmail assert"),
  );

  let addr = guard.smtp_addr().await;

  smtp_send(addr, "cli@test.com", "dest@test.com", "CLI Test", "body").await;

  let output = guard.wait_with_timeout(15).await;
  assert!(output.success(), "Expected exit code 0, got {:?}", output);
}

#[tokio::test]
async fn cli_assert_fails_on_timeout() {
  let mut guard = ChildGuard::new(
    rustmail_command()
      .args([
        "assert",
        "--smtp-port",
        ANY_FREE_PORT,
        "--min-count",
        "1",
        "--subject",
        "Never Sent",
        "--timeout",
        "2s",
        "--log-level",
        "warn",
      ])
      .spawn()
      .expect("failed to spawn rustmail assert"),
  );

  let output = guard.wait_with_timeout(10).await;
  assert!(
    !output.success(),
    "Expected non-zero exit, got {:?}",
    output
  );
}

#[tokio::test]
async fn smtp_rejects_oversized_message() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();

  let (tx, _) = mpsc::channel(256);
  let small_limit: usize = 256;

  tokio::spawn(async move {
    let (stream, peer) = listener.accept().await.unwrap();
    let mut session = Session::new(stream, peer, tx, small_limit, None);
    let _ = session.handle().await;
  });

  let mut stream = TcpStream::connect(addr).await.unwrap();
  let mut buf = vec![0u8; 4096];

  let _ = stream.read(&mut buf).await.unwrap();
  stream.write_all(b"EHLO test\r\n").await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  let ehlo = String::from_utf8_lossy(&buf[..n]);
  assert!(ehlo.contains("SIZE 256"), "EHLO should advertise SIZE 256");

  stream.write_all(b"MAIL FROM:<a@t.com>\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();
  stream.write_all(b"RCPT TO:<b@t.com>\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();
  stream.write_all(b"DATA\r\n").await.unwrap();
  let _ = stream.read(&mut buf).await.unwrap();

  let big_body = "X".repeat(512);
  let data = format!("Subject: Big\r\nFrom: a@t.com\r\nTo: b@t.com\r\n\r\n{big_body}\r\n.\r\n");
  stream.write_all(data.as_bytes()).await.unwrap();
  let n = stream.read(&mut buf).await.unwrap();
  let resp = String::from_utf8_lossy(&buf[..n]);
  assert!(
    resp.contains("552"),
    "Expected 552 rejection for oversized message, got: {resp}"
  );

  stream.write_all(b"QUIT\r\n").await.unwrap();
}

#[tokio::test]
async fn cli_assert_filters_by_subject() {
  let mut guard = ChildGuard::new(
    rustmail_command()
      .args([
        "assert",
        "--smtp-port",
        ANY_FREE_PORT,
        "--min-count",
        "1",
        "--subject",
        "Target",
        "--timeout",
        "10s",
        "--log-level",
        CHILD_LOG_FILTER,
      ])
      .spawn()
      .expect("failed to spawn"),
  );

  let addr = guard.smtp_addr().await;

  smtp_send(addr, "a@t.com", "b@t.com", "Decoy", "ignored").await;
  smtp_send(addr, "a@t.com", "b@t.com", "Target Email", "match").await;

  let output = guard.wait_with_timeout(15).await;
  assert!(output.success(), "Expected exit 0 after matching 'Target'");
}

#[tokio::test]
async fn smtp_session_limit_rejects_excess() {
  let (tx, _) = mpsc::channel::<Delivery>(256);
  let addr = spawn_smtp_only(tx).await;

  let mut held_connections = Vec::new();
  for _ in 0..100 {
    let stream = TcpStream::connect(addr).await.unwrap();
    held_connections.push(stream);
  }

  tokio::time::sleep(Duration::from_millis(200)).await;

  // The 101st connection should receive a 421 response and then be closed.
  let mut probe = TcpStream::connect(addr).await.unwrap();
  let mut buf = [0u8; 512];
  let result =
    tokio::time::timeout(Duration::from_secs(2), async { probe.read(&mut buf).await }).await;

  match result {
    Ok(Ok(0)) | Err(_) | Ok(Err(_)) => {}
    Ok(Ok(n)) => {
      let response = std::str::from_utf8(&buf[..n]).unwrap_or("");
      assert!(
        response.starts_with("421"),
        "Expected 421 rejection, got: {response}"
      );
    }
  }

  drop(held_connections);
}

/// Starts a real [`SmtpServer`] on an OS-assigned loopback port.
///
/// The listener is bound before the server task starts, so the returned
/// address accepts connections immediately and cannot be taken by another test.
async fn spawn_smtp_only(tx: mpsc::Sender<Delivery>) -> SocketAddr {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let config = SmtpServerConfig {
    max_message_size: MAX_MESSAGE_SIZE,
    ..SmtpServerConfig::default()
  };
  let server = SmtpServer::new(config, tx);
  tokio::spawn(async move {
    server.serve(listener).await.unwrap();
  });
  addr
}

/// A bulk sender reuses one connection for far more than the unproductive
/// command cap, so completing a transaction has to clear the counter.
const BULK_MESSAGES_ON_ONE_CONNECTION: usize = 400;

#[tokio::test]
async fn smtp_accepts_a_long_bulk_send_over_one_connection() {
  let (tx, mut rx) = mpsc::channel::<Delivery>(1024);
  // Stops at the expected count: the listener holds a sender for as long as it
  // runs, so the channel never closes on its own.
  let drain = tokio::spawn(async move {
    let mut seen = 0usize;
    while seen < BULK_MESSAGES_ON_ONE_CONNECTION {
      let Some(delivery) = rx.recv().await else {
        break;
      };
      let (_, ack) = delivery.into_parts();
      ack.stored();
      seen += 1;
    }
    seen
  });
  let addr = spawn_smtp_only(tx).await;

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  read_smtp_response_line(&mut stream).await;
  read_ehlo_response(&mut stream).await;

  for i in 0..BULK_MESSAGES_ON_ONE_CONNECTION {
    stream
      .write_all(b"MAIL FROM:<bulk@test.com>\r\n")
      .await
      .unwrap();
    assert!(
      read_smtp_response_line(&mut stream)
        .await
        .starts_with("250")
    );

    stream
      .write_all(b"RCPT TO:<sink@test.com>\r\n")
      .await
      .unwrap();
    assert!(
      read_smtp_response_line(&mut stream)
        .await
        .starts_with("250")
    );

    stream.write_all(b"DATA\r\n").await.unwrap();
    assert!(
      read_smtp_response_line(&mut stream)
        .await
        .starts_with("354")
    );

    stream
      .write_all(format!("Subject: bulk-{i}\r\n\r\nbody\r\n.\r\n").as_bytes())
      .await
      .unwrap();
    let reply = read_smtp_response_line(&mut stream).await;
    assert!(
      reply.starts_with("250"),
      "message {i} of {BULK_MESSAGES_ON_ONE_CONNECTION} was refused: {reply}"
    );
  }

  stream.write_all(b"QUIT\r\n").await.unwrap();
  drop(stream);

  let delivered = tokio::time::timeout(Duration::from_secs(30), drain)
    .await
    .expect("timed out waiting for the sent messages to reach the channel")
    .unwrap();
  assert_eq!(
    delivered, BULK_MESSAGES_ON_ONE_CONNECTION,
    "every message sent on the reused connection must reach the channel"
  );
}

/// Virtual time, so the per-line I/O deadline fires without the test waiting
/// out its real duration.
///
/// Losing that deadline makes this test hang rather than fail, since there is
/// then nothing left to wait for. That is inherent to asserting a disconnect
/// eventually happens; CI catches it on the job timeout.
#[tokio::test(start_paused = true)]
async fn smtp_disconnects_a_client_that_goes_silent() {
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  let addr = spawn_smtp_only(tx).await;

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  read_smtp_response_line(&mut stream).await;

  // Sessions carry no blanket duration cap, so the per-line I/O deadline is
  // the only thing that can reclaim a connection from a peer that says nothing.
  let mut tail = Vec::new();
  stream.read_to_end(&mut tail).await.unwrap();
  assert!(
    tail.is_empty(),
    "expected the server to close on the read deadline, got {:?}",
    String::from_utf8_lossy(&tail)
  );
}

#[tokio::test]
async fn smtp_still_cuts_off_a_client_that_never_delivers() {
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  let addr = spawn_smtp_only(tx).await;

  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  read_smtp_response_line(&mut stream).await;
  read_ehlo_response(&mut stream).await;

  for issued in 0..2000 {
    stream.write_all(b"NOOP\r\n").await.unwrap();
    let reply = read_smtp_response_line(&mut stream).await;
    if reply.starts_with("421") {
      return;
    }
    assert!(
      reply.starts_with("250"),
      "unexpected reply to NOOP {issued}: {reply}"
    );
  }

  panic!("a client issuing only unproductive commands was never cut off");
}

const WS_PING_OPCODE: u8 = 0x9;
const WS_TEXT_OPCODE: u8 = 0x1;
const WS_CLOSE_OPCODE: u8 = 0x8;
const WS_EXTENDED_LENGTH_MARKER: u8 = 126;
const WS_HUGE_LENGTH_MARKER: u8 = 127;

async fn ws_handshake(http_addr: std::net::SocketAddr) -> BufReader<TcpStream> {
  let mut stream = TcpStream::connect(http_addr).await.unwrap();
  stream
    .write_all(
      format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {http_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
      )
      .as_bytes(),
    )
    .await
    .unwrap();

  let mut reader = BufReader::new(stream);
  loop {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(!line.is_empty(), "connection closed during WS handshake");
    if line == "\r\n" {
      return reader;
    }
  }
}

/// Reads one unmasked server frame, returning its opcode.
async fn read_ws_opcode(reader: &mut BufReader<TcpStream>) -> u8 {
  let mut header = [0u8; 2];
  reader.read_exact(&mut header).await.unwrap();
  let opcode = header[0] & 0x0f;

  let len = match header[1] & 0x7f {
    WS_EXTENDED_LENGTH_MARKER => {
      let mut extended = [0u8; 2];
      reader.read_exact(&mut extended).await.unwrap();
      u16::from_be_bytes(extended) as usize
    }
    WS_HUGE_LENGTH_MARKER => {
      let mut extended = [0u8; 8];
      reader.read_exact(&mut extended).await.unwrap();
      u64::from_be_bytes(extended) as usize
    }
    len => len as usize,
  };

  let mut payload = vec![0u8; len];
  reader.read_exact(&mut payload).await.unwrap();
  opcode
}

/// Small enough that a handful of events overruns it deterministically.
const TINY_BROADCAST_CAPACITY: usize = 4;
const EVENTS_OVERRUNNING_CAPACITY: usize = 200;

#[tokio::test]
async fn ws_closes_a_client_that_falls_behind_instead_of_dropping_events() {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(TINY_BROADCAST_CAPACITY);
  let state = AppState::new(repo, ws_tx.clone(), None, None);
  let app = router(state);

  let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let http_addr = http_listener.local_addr().unwrap();
  tokio::spawn(async move {
    axum::serve(http_listener, app).await.unwrap();
  });

  let mut reader = ws_handshake(http_addr).await;

  // The ping on connect proves the handler is subscribed, so the burst below
  // lands in its receiver rather than before it existed.
  assert_eq!(read_ws_opcode(&mut reader).await, WS_PING_OPCODE);

  // No await inside the loop, so on a current-thread runtime the handler
  // cannot drain between sends and is guaranteed to fall behind.
  for i in 0..EVENTS_OVERRUNNING_CAPACITY {
    ws_tx
      .send(WsFrame::encode(&WsEvent::MessageDelete { id: i.to_string() }).unwrap())
      .unwrap();
  }

  let mut text_frames = 0;
  loop {
    let opcode = tokio::time::timeout(Duration::from_secs(5), read_ws_opcode(&mut reader))
      .await
      .expect("server neither closed nor kept streaming after the client fell behind");

    match opcode {
      WS_CLOSE_OPCODE => break,
      WS_TEXT_OPCODE => {
        text_frames += 1;
        assert!(
          text_frames < EVENTS_OVERRUNNING_CAPACITY,
          "server streamed a partial event set instead of closing"
        );
      }
      WS_PING_OPCODE => {}
      other => panic!("unexpected opcode {other:#x}"),
    }
  }

  assert!(
    text_frames < EVENTS_OVERRUNNING_CAPACITY,
    "a lagging client must not be served a silently incomplete stream"
  );
}

#[tokio::test]
async fn ws_server_pings_a_client_as_soon_as_it_connects() {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);
  let app = router(AppState::new(repo, ws_tx, None, None));

  let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let http_addr = http_listener.local_addr().unwrap();
  tokio::spawn(async move {
    axum::serve(http_listener, app).await.unwrap();
  });

  let mut stream = TcpStream::connect(http_addr).await.unwrap();
  stream
    .write_all(
      format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {http_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
      )
      .as_bytes(),
    )
    .await
    .unwrap();

  let mut reader = BufReader::new(stream);
  loop {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(!line.is_empty(), "connection closed during WS handshake");
    if line == "\r\n" {
      break;
    }
  }

  // The client never speaks, so only a server-side heartbeat can arrive here.
  // This covers the ping sent on connect; that pings keep coming is enforced by
  // the WS_PING_INTERVAL < WS_IDLE_TIMEOUT invariant asserted in rustmail-api.
  let mut frame_header = [0u8; 1];
  tokio::time::timeout(Duration::from_secs(5), reader.read_exact(&mut frame_header))
    .await
    .expect("server sent no frame within 5s; heartbeat is missing")
    .unwrap();

  assert_eq!(
    frame_header[0] & 0x0f,
    WS_PING_OPCODE,
    "expected a ping frame, got opcode {:#x}",
    frame_header[0] & 0x0f
  );
}

#[tokio::test]
async fn ws_connection_limit_returns_503() {
  let (app, _, _) = {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool);
    let (ws_tx, _) = broadcast::channel::<WsFrame>(256);
    let state = AppState::new(repo.clone(), ws_tx.clone(), None, None);
    (router(state.clone()), repo, state)
  };

  let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let http_addr = http_listener.local_addr().unwrap();
  tokio::spawn(async move {
    axum::serve(http_listener, app).await.unwrap();
  });

  let mut ws_connections = Vec::new();
  for _ in 0..50 {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/api/v1/ws", http_addr))
      .await
      .unwrap();
    ws_connections.push(ws);
  }

  let result = tokio_tungstenite::connect_async(format!("ws://{}/api/v1/ws", http_addr)).await;

  assert!(
    result.is_err(),
    "Expected 51st WS connection to be rejected"
  );

  drop(ws_connections);
}

#[tokio::test]
async fn config_env_overrides_toml() {
  use std::io::Write;

  let toml_port_holder = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let smtp_port_toml = toml_port_holder.local_addr().unwrap().port();

  let mut toml_file = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
  write!(
    toml_file,
    "smtp_port = {smtp_port_toml}\nhttp_port = {ANY_FREE_PORT}\nephemeral = true\n"
  )
  .unwrap();

  let mut guard = ChildGuard::new(
    rustmail_command()
      .args(["serve", "--config", toml_file.path().to_str().unwrap()])
      .env("RUSTMAIL_SMTP_PORT", ANY_FREE_PORT)
      .env("RUSTMAIL_LOG_LEVEL", CHILD_LOG_FILTER)
      .spawn()
      .expect("failed to spawn"),
  );

  let smtp_addr = guard.smtp_addr().await;
  assert_ne!(
    smtp_addr.port(),
    smtp_port_toml,
    "SMTP must bind the env port, not the TOML port {smtp_port_toml}"
  );

  let mut stream = BufReader::new(TcpStream::connect(smtp_addr).await.unwrap());
  let banner = read_banner(&mut stream).await;
  assert!(banner.starts_with("220"), "got: {banner}");
}

#[tokio::test]
async fn config_toml_used_when_no_env() {
  use std::io::Write;

  let mut toml_file = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
  write!(
    toml_file,
    "smtp_port = {ANY_FREE_PORT}\nhttp_port = {ANY_FREE_PORT}\nephemeral = true\n"
  )
  .unwrap();

  let mut guard = ChildGuard::new(
    rustmail_command()
      .args(["serve", "--config", toml_file.path().to_str().unwrap()])
      .env_remove("RUSTMAIL_SMTP_PORT")
      .env_remove("RUSTMAIL_HTTP_PORT")
      .env("RUSTMAIL_LOG_LEVEL", CHILD_LOG_FILTER)
      .spawn()
      .expect("failed to spawn"),
  );

  let smtp_addr = guard.smtp_addr().await;
  assert_ne!(
    smtp_addr.port(),
    DEFAULT_SMTP_PORT,
    "SMTP must bind the TOML port, not the built-in default"
  );
}

#[tokio::test]
async fn smtp_tls_requires_both_cert_and_key() {
  let cert_path = starttls_cert_path();
  let key_path = starttls_key_path();

  for (cert, key) in [
    (Some(cert_path.as_path()), None),
    (None, Some(key_path.as_path())),
  ] {
    let mut command = rustmail_command();
    command.args([
      "serve",
      "--smtp-port",
      ANY_FREE_PORT,
      "--http-port",
      ANY_FREE_PORT,
      "--ephemeral",
      "--log-level",
      "warn",
    ]);

    if let Some(cert) = cert {
      command.arg("--smtp-tls-cert").arg(cert);
    }
    if let Some(key) = key {
      command.arg("--smtp-tls-key").arg(key);
    }

    let output = command
      .output()
      .await
      .expect("failed to run rustmail serve");
    assert!(!output.status.success(), "expected startup failure");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
      stderr.contains("SMTP TLS configuration requires both --smtp-tls-cert and --smtp-tls-key"),
      "unexpected stderr: {stderr}"
    );
  }
}

const STARTUP_FAILURE_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn serve_refuses_a_database_from_a_newer_schema() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .max_connections(1)
    .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
    .await
    .unwrap();
  sqlx::query("CREATE TABLE message_content (seq INTEGER PRIMARY KEY)")
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query("PRAGMA user_version = 2")
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  let bytes_before = std::fs::read(&db_path).unwrap();

  let output = tokio::time::timeout(
    STARTUP_FAILURE_TIMEOUT,
    rustmail_command()
      .args([
        "serve",
        "--smtp-port",
        ANY_FREE_PORT,
        "--http-port",
        ANY_FREE_PORT,
        "--log-level",
        "warn",
      ])
      .arg("--db-path")
      .arg(&db_path)
      .output(),
  )
  .await
  .expect("rustmail serve did not exit on a newer database")
  .expect("failed to run rustmail serve");

  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    !output.status.success(),
    "expected startup failure: {stderr}"
  );
  assert!(!stderr.contains("panicked"), "startup panicked: {stderr}");
  assert!(
    stderr.contains(&db_path.display().to_string()),
    "stderr does not name the database: {stderr}"
  );
  assert!(
    stderr.contains(
      "rustmail.db is schema 2, written by a newer rustmail; this binary supports schema 1. Upgrade rustmail, or run `rustmail restore-backup` with that newer binary."
    ),
    "unexpected stderr: {stderr}"
  );
  assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
}

/// The `messages` table of rustmail 0.1.0 through 0.7.0.
const LEGACY_MESSAGES_DDL: &str = "CREATE TABLE messages (id TEXT PRIMARY KEY, sender TEXT NOT NULL, recipients TEXT NOT NULL, subject TEXT, text_body TEXT, html_body TEXT, raw BLOB NOT NULL, size INTEGER NOT NULL, has_attachments INTEGER NOT NULL DEFAULT 0, is_read INTEGER NOT NULL DEFAULT 0, is_starred INTEGER NOT NULL DEFAULT 0, tags TEXT NOT NULL DEFAULT '[]', created_at TEXT NOT NULL)";
const LEGACY_MESSAGE_ID: &str = "01J0000000000000000000LEGA";

async fn legacy_database(db_path: &std::path::Path) {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .max_connections(1)
    .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
    .await
    .unwrap();
  sqlx::query(LEGACY_MESSAGES_DDL)
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO messages (id, sender, recipients, subject, text_body, raw, size, is_read, tags, created_at) \
     VALUES (?1, 'a@test.com', '[\"b@test.com\"]', 'kept', 'body', x'00', 1, 1, '[\"work\"]', '2026-01-01T00:00:00Z')",
  )
  .bind(LEGACY_MESSAGE_ID)
  .execute(&pool)
  .await
  .unwrap();
  pool.close().await;
}

fn sidecar(path: &std::path::Path, suffix: &str) -> PathBuf {
  let mut name = path.as_os_str().to_owned();
  name.push(suffix);
  PathBuf::from(name)
}

#[tokio::test]
async fn serve_migrates_a_legacy_database_before_it_listens() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  legacy_database(&db_path).await;
  let bytes_before = std::fs::read(&db_path).unwrap();

  let mut guard = ChildGuard::new(
    rustmail_command()
      .args([
        "serve",
        "--smtp-port",
        ANY_FREE_PORT,
        "--http-port",
        ANY_FREE_PORT,
        "--log-level",
        CHILD_LOG_FILTER,
      ])
      .arg("--db-path")
      .arg(&db_path)
      .spawn()
      .expect("failed to spawn rustmail serve"),
  );
  guard.smtp_addr().await;

  let backup = sidecar(&db_path, ".schema0.bak");
  assert_eq!(
    std::fs::read(&backup).unwrap(),
    bytes_before,
    "the legacy database should be kept, unchanged, as the backup"
  );
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .max_connections(1)
    .connect(&format!("sqlite:{}", db_path.display()))
    .await
    .unwrap();
  let version: i64 = sqlx::query_scalar("PRAGMA user_version")
    .fetch_one(&pool)
    .await
    .unwrap();
  assert_eq!(version, 1);
  let (id, is_read, tags): (String, bool, String) =
    sqlx::query_as("SELECT id, is_read, tags FROM messages WHERE seq = 1")
      .fetch_one(&pool)
      .await
      .unwrap();
  pool.close().await;
  assert_eq!(
    (id.as_str(), is_read, tags.as_str()),
    (LEGACY_MESSAGE_ID, true, r#"["work"]"#)
  );
}

#[tokio::test]
async fn serve_exits_while_another_process_holds_the_migration_lock() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  legacy_database(&db_path).await;
  let bytes_before = std::fs::read(&db_path).unwrap();
  let lock_path = sidecar(&db_path, ".migration-lock");
  let lock = hold_migration_lock(std::fs::File::create(&lock_path).unwrap()).await;

  let output = tokio::time::timeout(
    STARTUP_FAILURE_TIMEOUT,
    rustmail_command()
      .args([
        "serve",
        "--smtp-port",
        ANY_FREE_PORT,
        "--http-port",
        ANY_FREE_PORT,
        "--log-level",
        "warn",
      ])
      .arg("--db-path")
      .arg(&db_path)
      .output(),
  )
  .await
  .expect("rustmail serve did not exit on a held migration lock")
  .expect("failed to run rustmail serve");
  drop(lock);

  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    !output.status.success(),
    "expected startup failure: {stderr}"
  );
  assert!(!stderr.contains("panicked"), "startup panicked: {stderr}");
  assert!(
    stderr.contains(&format!(
      "another rustmail process holds {} (storage migration or restore in progress)",
      lock_path.display()
    )),
    "unexpected stderr: {stderr}"
  );
  assert_eq!(std::fs::read(&db_path).unwrap(), bytes_before);
  assert!(!sidecar(&db_path, ".migrating").exists());
}

const RESTORE_SUBJECTS: [&str; 2] = ["before one", "before two"];
const POST_MIGRATION_SUBJECT: &str = "after the migration";
const KEPT_INFIX: &str = ".schema1-";

/// Writes a legacy database with rustmail v0.7.0's own statements and
/// returns its message ids in arrival order.
async fn legacy_v0_7_0_database(db_path: &std::path::Path) -> Vec<String> {
  let pool = legacy_v0_7_0::create_legacy_database(db_path).await;
  let mut ids = Vec::new();
  for (index, subject) in RESTORE_SUBJECTS.iter().enumerate() {
    let raw =
      format!("From: a@test.com\r\nTo: b@test.com\r\nSubject: {subject}\r\n\r\nbody {index}");
    let stored = legacy_v0_7_0::insert_as_v0_7_0(
      &pool,
      "a@test.com",
      &["b@test.com".to_string()],
      raw.as_bytes(),
      &format!("2026-09-23T10:00:0{index}Z"),
    )
    .await
    .unwrap();
    ids.push(stored.id);
  }
  pool.close().await;
  ids
}

/// Migrates a legacy database in this process, the way `serve` does at startup.
/// Takes the migration lock on `file`, waiting for any earlier holder.
///
/// A lock the test's own migration just dropped can stay held for a moment
/// while a parallel test forks a `rustmail` child, which inherits every open
/// descriptor until it execs, so a single `try_lock` is racy here.
async fn hold_migration_lock(file: std::fs::File) -> std::fs::File {
  tokio::time::timeout(
    STARTUP_FAILURE_TIMEOUT,
    tokio::task::spawn_blocking(move || {
      file.lock().unwrap();
      file
    }),
  )
  .await
  .expect("the migration lock was not released in time")
  .unwrap()
}

async fn migrated_database(db_path: &std::path::Path) {
  legacy_v0_7_0_database(db_path).await;
  let preparation = rustmail_storage::prepare_database_file(db_path, || false)
    .await
    .unwrap();
  assert!(matches!(
    preparation,
    rustmail_storage::Preparation::Ready(Some(_))
  ));
}

fn kept_schema_1_files(db_path: &std::path::Path) -> Vec<PathBuf> {
  let prefix = format!(
    "{}{KEPT_INFIX}",
    db_path.file_name().unwrap().to_string_lossy()
  );
  std::fs::read_dir(db_path.parent().unwrap())
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| {
      path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
    })
    .collect()
}

async fn run_restore_backup(db_path: &std::path::Path) -> std::process::Output {
  tokio::time::timeout(
    STARTUP_FAILURE_TIMEOUT,
    rustmail_command()
      .args(["restore-backup", "--log-level", "info"])
      .arg("--db-path")
      .arg(db_path)
      .output(),
  )
  .await
  .expect("rustmail restore-backup did not exit in time")
  .expect("failed to run rustmail restore-backup")
}

fn spawn_serve(db_path: &std::path::Path) -> ChildGuard {
  ChildGuard::new(
    rustmail_command()
      .args([
        "serve",
        "--smtp-port",
        ANY_FREE_PORT,
        "--http-port",
        ANY_FREE_PORT,
        "--log-level",
        CHILD_LOG_FILTER,
      ])
      .arg("--db-path")
      .arg(db_path)
      .spawn()
      .expect("failed to spawn rustmail serve"),
  )
}

async fn query_ids(db_path: &std::path::Path) -> Vec<String> {
  let pool = legacy_v0_7_0::open_plain(db_path).await;
  let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY rowid")
    .fetch_all(&pool)
    .await
    .unwrap();
  pool.close().await;
  ids
}

async fn query_i64(db_path: &std::path::Path, sql: &str) -> i64 {
  let pool = legacy_v0_7_0::open_plain(db_path).await;
  let value: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
  pool.close().await;
  value
}

#[tokio::test]
async fn restore_backup_puts_the_v0_7_0_mailbox_back_and_serve_migrates_it_again() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  let backup = sidecar(&db_path, ".schema0.bak");
  let legacy_ids = legacy_v0_7_0_database(&db_path).await;
  let legacy_bytes = std::fs::read(&db_path).unwrap();

  let mut server = spawn_serve(&db_path);
  let addr = server.smtp_addr().await;
  smtp_send(
    addr,
    "late@test.com",
    "b@test.com",
    POST_MIGRATION_SUBJECT,
    "new",
  )
  .await;
  server.kill().await;

  let output = run_restore_backup(&db_path).await;
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(
    output.status.success(),
    "restore-backup failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  let kept = kept_schema_1_files(&db_path);
  assert_eq!(kept.len(), 1, "expected one kept schema-1 file: {kept:?}");
  let kept = &kept[0];
  for expected in [
    format!("Restored {} to {}.", backup.display(), db_path.display()),
    format!("The migrated database is kept as {}.", kept.display()),
    format!(
      "Mail received after the storage migration is only in {}",
      kept.display()
    ),
    "event=\"storage_restore\"".to_string(),
  ] {
    assert!(
      stdout.contains(&expected),
      "missing {expected:?} in: {stdout}"
    );
  }

  assert_eq!(std::fs::read(&db_path).unwrap(), legacy_bytes);
  assert!(!backup.exists());
  for suffix in ["-wal", "-shm", "-journal"] {
    assert!(
      !sidecar(&db_path, suffix).exists(),
      "{suffix} left beside the restored file"
    );
    assert!(
      !sidecar(kept, suffix).exists(),
      "{suffix} left beside the kept file"
    );
  }
  let pool = legacy_v0_7_0::open_plain(&db_path).await;
  legacy_v0_7_0::initialize_as_v0_7_0(&pool).await.unwrap();
  pool.close().await;
  assert_eq!(query_ids(&db_path).await, legacy_ids);
  assert_eq!(query_i64(&db_path, "PRAGMA user_version").await, 0);
  assert_eq!(query_i64(kept, "PRAGMA user_version").await, 1);
  assert_eq!(
    query_i64(
      kept,
      &format!("SELECT count(*) FROM messages WHERE subject = '{POST_MIGRATION_SUBJECT}'")
    )
    .await,
    1
  );
  let kept_bytes = std::fs::read(kept).unwrap();

  let mut server = spawn_serve(&db_path);
  server.smtp_addr().await;
  assert_eq!(std::fs::read(&backup).unwrap(), legacy_bytes);
  assert_eq!(query_i64(&db_path, "PRAGMA user_version").await, 1);
  assert_eq!(
    query_i64(&db_path, "SELECT count(*) FROM messages").await,
    legacy_ids.len() as i64
  );
  assert_eq!(std::fs::read(kept).unwrap(), kept_bytes);
  assert!(!sidecar(&db_path, ".migrating").exists());
}

#[tokio::test]
async fn restore_backup_refuses_while_a_server_has_the_database_open() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  migrated_database(&db_path).await;
  let mut server = spawn_serve(&db_path);
  server.smtp_addr().await;

  let output = run_restore_backup(&db_path).await;
  server.kill().await;

  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!output.status.success(), "expected a refusal: {stderr}");
  assert!(
    !stderr.contains("panicked"),
    "restore-backup panicked: {stderr}"
  );
  assert!(
    stderr.contains(&format!(
      "{} exists, so another process still has it open",
      sidecar(&db_path, "-wal").display()
    )),
    "unexpected stderr: {stderr}"
  );
  assert!(sidecar(&db_path, ".schema0.bak").exists());
  assert!(kept_schema_1_files(&db_path).is_empty());
  assert_eq!(query_i64(&db_path, "PRAGMA user_version").await, 1);
}

#[tokio::test]
async fn restore_backup_refuses_while_another_process_holds_the_migration_lock() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  migrated_database(&db_path).await;
  let backup_bytes = std::fs::read(sidecar(&db_path, ".schema0.bak")).unwrap();
  let lock_path = sidecar(&db_path, ".migration-lock");
  let lock = hold_migration_lock(
    std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .open(&lock_path)
      .unwrap(),
  )
  .await;

  let output = run_restore_backup(&db_path).await;
  drop(lock);

  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!output.status.success(), "expected a refusal: {stderr}");
  assert!(
    stderr.contains(&format!(
      "another rustmail process holds {} (storage migration or restore in progress)",
      lock_path.display()
    )),
    "unexpected stderr: {stderr}"
  );
  assert_eq!(
    std::fs::read(sidecar(&db_path, ".schema0.bak")).unwrap(),
    backup_bytes
  );
  assert!(kept_schema_1_files(&db_path).is_empty());
  assert_eq!(query_i64(&db_path, "PRAGMA user_version").await, 1);
}

#[tokio::test]
async fn restore_backup_refuses_without_a_backup() {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");
  migrated_database(&db_path).await;
  let backup = sidecar(&db_path, ".schema0.bak");
  std::fs::remove_file(&backup).unwrap();

  let output = run_restore_backup(&db_path).await;

  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!output.status.success(), "expected a refusal: {stderr}");
  assert!(
    stderr.contains(&format!(
      "there is no backup {} to restore over {}",
      backup.display(),
      db_path.display()
    )),
    "unexpected stderr: {stderr}"
  );
  assert!(kept_schema_1_files(&db_path).is_empty());
  assert_eq!(query_i64(&db_path, "PRAGMA user_version").await, 1);
}

async fn connect_smtp_and_greet(addr: std::net::SocketAddr) -> BufReader<TcpStream> {
  let stream = TcpStream::connect(addr).await.unwrap();
  let mut stream = BufReader::new(stream);
  let _banner = read_banner(&mut stream).await;
  stream
}

async fn send_line<S>(stream: &mut BufReader<S>, line: &str) -> String
where
  S: AsyncRead + AsyncWrite + Unpin,
{
  stream.write_all(line.as_bytes()).await.unwrap();
  stream.write_all(b"\r\n").await.unwrap();
  read_smtp_response_line(stream).await
}

#[tokio::test]
async fn smtp_rejects_mail_from_before_ehlo() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let resp = send_line(&mut stream, "MAIL FROM:<alice@test.com>").await;
  assert_eq!(resp, "503 Bad sequence of commands\r\n");
}

#[tokio::test]
async fn smtp_rejects_rcpt_to_before_mail_from() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let resp = send_line(&mut stream, "RCPT TO:<bob@test.com>").await;
  assert_eq!(resp, "503 Bad sequence of commands\r\n");
}

#[tokio::test]
async fn smtp_rejects_data_before_mail_from() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let resp = send_line(&mut stream, "DATA").await;
  assert_eq!(resp, "503 Bad sequence of commands\r\n");
}

#[tokio::test]
async fn smtp_rejects_data_without_recipients() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let mail = send_line(&mut stream, "MAIL FROM:<alice@test.com>").await;
  assert_eq!(mail, "250 OK\r\n");
  let resp = send_line(&mut stream, "DATA").await;
  assert_eq!(resp, "503 Bad sequence of commands\r\n");
}

#[tokio::test]
async fn smtp_rejects_excess_recipients() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let mail = send_line(&mut stream, "MAIL FROM:<alice@test.com>").await;
  assert_eq!(mail, "250 OK\r\n");

  for i in 0..100 {
    let resp = send_line(&mut stream, &format!("RCPT TO:<r{i}@test.com>")).await;
    assert_eq!(resp, "250 OK\r\n", "recipient {i} should be accepted");
  }

  let overflow = send_line(&mut stream, "RCPT TO:<overflow@test.com>").await;
  assert_eq!(overflow, "452 Too many recipients\r\n");
}

#[tokio::test]
async fn smtp_unknown_command_returns_500() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let resp = send_line(&mut stream, "WAT").await;
  assert_eq!(resp, "500 Unknown command\r\n");
}

const LOCAL_ERROR: &str = "451 Requested action aborted: local error in processing\r\n";

/// Walks a session up to the terminating dot and returns the reply it draws.
async fn send_one_message(addr: std::net::SocketAddr) -> String {
  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  assert_eq!(
    send_line(&mut stream, "MAIL FROM:<alice@test.com>").await,
    "250 OK\r\n"
  );
  assert_eq!(
    send_line(&mut stream, "RCPT TO:<bob@test.com>").await,
    "250 OK\r\n"
  );
  assert!(send_line(&mut stream, "DATA").await.starts_with("354 "));

  stream
    .write_all(b"Subject: Verdict\r\n\r\nbody\r\n.\r\n")
    .await
    .unwrap();
  read_smtp_response_line(&mut stream).await
}

/// A message the store refuses must not leave the sender thinking it landed.
///
/// The session used to answer `250` the moment the message reached the
/// in-process channel, so a write that failed afterwards lost it with nothing
/// but a log line to show for it. `451` is what asks for the retry.
#[tokio::test]
async fn smtp_refuses_a_message_the_consumer_could_not_store() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, mut rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  tokio::spawn(async move {
    while let Some(delivery) = rx.recv().await {
      let (_, ack) = delivery.into_parts();
      ack.rejected();
    }
  });

  assert_eq!(send_one_message(addr).await, LOCAL_ERROR);
}

/// A consumer that goes away mid-write cannot vouch for the message either.
#[tokio::test]
async fn smtp_refuses_a_message_left_unanswered() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, mut rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  tokio::spawn(async move {
    while let Some(delivery) = rx.recv().await {
      drop(delivery);
    }
  });

  assert_eq!(send_one_message(addr).await, LOCAL_ERROR);
}

#[tokio::test]
async fn smtp_accepts_a_message_the_consumer_stored() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, rx) = mpsc::channel::<Delivery>(16);
  let mut messages = accept_deliveries(rx);
  spawn_smtp_with_real_session(listener, tx);

  assert_eq!(send_one_message(addr).await, "250 OK\r\n");
  assert_eq!(messages.recv().await.unwrap().sender, "alice@test.com");
}

#[tokio::test]
async fn smtp_noop_allowed_before_ehlo() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let resp = send_line(&mut stream, "NOOP").await;
  assert_eq!(resp, "250 OK\r\n");
}

/// Longer than the 4096-byte command cap, shorter than the 8 KiB read buffer.
const LONG_BODY_LINE_LEN: usize = 6000;
/// Longer than the read buffer, so the line always spans several reads.
const BUFFER_SPANNING_BODY_LINE_LEN: usize = 20_000;

/// Past the command cap, so a read that ends here used to trip it.
const LONG_BODY_LINE_SPLIT_AT: usize = 5000;

/// Sends one message whose body is a single `line_len`-byte line.
///
/// With `split_at`, the line goes out as two writes broken that many bytes in.
async fn send_long_line_message(
  addr: std::net::SocketAddr,
  line_len: usize,
  split_at: Option<usize>,
) -> String {
  let mut stream = connect_smtp_and_greet(addr).await;
  stream.get_ref().set_nodelay(true).unwrap();
  let _ehlo = read_ehlo_response(&mut stream).await;
  assert_eq!(
    send_line(&mut stream, "MAIL FROM:<alice@test.com>").await,
    "250 OK\r\n"
  );
  assert_eq!(
    send_line(&mut stream, "RCPT TO:<bob@test.com>").await,
    "250 OK\r\n"
  );
  assert!(send_line(&mut stream, "DATA").await.starts_with("354 "));

  let headers = "Subject: Long\r\n\r\n";
  let payload = format!("{headers}{}\r\n.\r\n", "X".repeat(line_len));
  let (first, rest) = payload.split_at(split_at.map_or(payload.len(), |at| headers.len() + at));
  for part in [first, rest] {
    stream.write_all(part.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
  }
  read_smtp_response_line(&mut stream).await
}

async fn assert_long_line_is_stored(line_len: usize, split_at: Option<usize>) {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, rx) = mpsc::channel::<Delivery>(16);
  let mut messages = accept_deliveries(rx);
  spawn_smtp_with_real_session(listener, tx);

  assert_eq!(
    send_long_line_message(addr, line_len, split_at).await,
    "250 OK\r\n"
  );
  let message = messages.recv().await.unwrap();
  let expected_line = format!("{}\r\n", "X".repeat(line_len));
  assert!(
    message.raw.ends_with(expected_line.as_bytes()),
    "the {line_len}-byte body line must be stored intact"
  );
}

#[tokio::test]
async fn smtp_stores_a_long_body_line_sent_in_one_write() {
  assert_long_line_is_stored(LONG_BODY_LINE_LEN, None).await;
}

#[tokio::test]
async fn smtp_stores_a_long_body_line_split_across_writes() {
  assert_long_line_is_stored(LONG_BODY_LINE_LEN, Some(LONG_BODY_LINE_SPLIT_AT)).await;
}

#[tokio::test]
async fn smtp_stores_a_body_line_longer_than_the_read_buffer() {
  assert_long_line_is_stored(BUFFER_SPANNING_BODY_LINE_LEN, None).await;
}

/// A refused message must be discarded whole, however long its lines are.
///
/// The drain used to stop at the first line past the command cap, so the rest
/// of the body was read back as commands, each drawing a `500`.
#[tokio::test]
async fn smtp_discards_an_oversized_message_with_long_lines_and_stays_in_sync() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  let small_limit: usize = 256;
  tokio::spawn(async move {
    let (stream, peer) = listener.accept().await.unwrap();
    let mut session = Session::new(stream, peer, tx, small_limit, None);
    let _ = session.handle().await;
  });

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  assert_eq!(
    send_line(&mut stream, "MAIL FROM:<alice@test.com>").await,
    "250 OK\r\n"
  );
  assert_eq!(
    send_line(&mut stream, "RCPT TO:<bob@test.com>").await,
    "250 OK\r\n"
  );
  assert!(send_line(&mut stream, "DATA").await.starts_with("354 "));

  let payload = format!(
    "Subject: Big\r\n\r\n{}\r\n{}\r\nafter\r\n.\r\n",
    "A".repeat(small_limit),
    "B".repeat(LONG_BODY_LINE_LEN)
  );
  stream.write_all(payload.as_bytes()).await.unwrap();
  assert!(
    read_smtp_response_line(&mut stream)
      .await
      .starts_with("552 ")
  );
  assert_eq!(send_line(&mut stream, "NOOP").await, "250 OK\r\n");
}

/// Capacity of the session's read buffer, tokio's `BufReader` default.
const SESSION_READ_BUFFER_LEN: usize = 8 * 1024;

/// A line the size check cuts mid-way must be skipped to its end, even when
/// what is left of it is a lone dot.
#[tokio::test]
async fn smtp_drain_does_not_mistake_the_tail_of_a_cut_line_for_the_end() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  let small_limit: usize = 256;
  tokio::spawn(async move {
    let (stream, peer) = listener.accept().await.unwrap();
    let mut session = Session::new(stream, peer, tx, small_limit, None);
    let _ = session.handle().await;
  });

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  assert_eq!(
    send_line(&mut stream, "MAIL FROM:<alice@test.com>").await,
    "250 OK\r\n"
  );
  assert_eq!(
    send_line(&mut stream, "RCPT TO:<bob@test.com>").await,
    "250 OK\r\n"
  );
  assert!(send_line(&mut stream, "DATA").await.starts_with("354 "));

  let headers = "Subject: Big\r\n\r\n";
  let payload = format!(
    "{headers}{}.\r\nafter\r\n.\r\n",
    "A".repeat(SESSION_READ_BUFFER_LEN - headers.len())
  );
  stream.write_all(payload.as_bytes()).await.unwrap();
  assert!(
    read_smtp_response_line(&mut stream)
      .await
      .starts_with("552 ")
  );
  assert_eq!(send_line(&mut stream, "NOOP").await, "250 OK\r\n");
}

/// A sender that declares an oversized message is refused before it sends it.
#[tokio::test]
async fn smtp_refuses_a_declared_size_over_the_limit_at_mail_from() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let mail = send_line(
    &mut stream,
    &format!("MAIL FROM:<alice@test.com> SIZE={}", MAX_MESSAGE_SIZE + 1),
  )
  .await;
  assert!(mail.starts_with("552 5.3.4 "), "got: {mail}");
  assert_eq!(
    send_line(&mut stream, "RCPT TO:<bob@test.com>").await,
    "503 Bad sequence of commands\r\n",
    "a refused MAIL FROM must not open a transaction"
  );
}

#[tokio::test]
async fn smtp_accepts_a_declared_size_within_the_limit() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let mail = send_line(
    &mut stream,
    &format!("MAIL FROM:<alice@test.com> size={MAX_MESSAGE_SIZE}"),
  )
  .await;
  assert_eq!(mail, "250 OK\r\n");
}

#[tokio::test]
async fn smtp_rejects_a_malformed_declared_size() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let (tx, _rx) = mpsc::channel::<Delivery>(16);
  spawn_smtp_with_real_session(listener, tx);

  let mut stream = connect_smtp_and_greet(addr).await;
  let _ehlo = read_ehlo_response(&mut stream).await;
  let mail = send_line(&mut stream, "MAIL FROM:<alice@test.com> SIZE=lots").await;
  assert!(mail.starts_with("501 5.5.4 "), "got: {mail}");
}

/// Ceiling on how long a clean stop may take with nothing left in flight.
///
/// Tighter than the server's shutdown drain deadline, so a server that merely
/// waits that deadline out instead of draining fails here.
#[cfg(unix)]
const PROMPT_EXIT_SECS: u64 = 4;

#[cfg(unix)]
async fn stop_after_one_delivery(signal: &str) {
  let data_dir = tempfile::tempdir().unwrap();
  let db_path = data_dir.path().join("rustmail.db");

  let mut guard = ChildGuard::new(
    rustmail_command()
      .args([
        "serve",
        "--smtp-port",
        ANY_FREE_PORT,
        "--http-port",
        ANY_FREE_PORT,
        "--log-level",
        CHILD_LOG_FILTER,
      ])
      .arg("--db-path")
      .arg(&db_path)
      .spawn()
      .expect("failed to spawn rustmail serve"),
  );
  let pid = guard.child.as_ref().and_then(|child| child.id()).unwrap();

  let smtp_addr = guard.smtp_addr().await;
  smtp_send(smtp_addr, "a@test.com", "b@test.com", "Before stop", "body").await;

  let kill = tokio::process::Command::new("kill")
    .arg(format!("-{signal}"))
    .arg(pid.to_string())
    .status()
    .await
    .unwrap();
  assert!(kill.success());

  let status = guard.wait_with_timeout(PROMPT_EXIT_SECS).await;
  assert!(
    status.success(),
    "SIG{signal} should stop cleanly, got {status:?}"
  );

  let wal_path = data_dir.path().join("rustmail.db-wal");
  let wal_len = std::fs::metadata(&wal_path).map_or(0, |meta| meta.len());
  assert_eq!(wal_len, 0, "the WAL must be checkpointed on the way out");

  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect(&format!("sqlite:{}", db_path.display()))
    .await
    .unwrap();
  let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
    .fetch_one(&pool)
    .await
    .unwrap();
  assert_eq!(stored, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_drains_and_checkpoints_before_exit() {
  stop_after_one_delivery("TERM").await;
}

#[cfg(unix)]
#[tokio::test]
async fn ctrl_c_drains_and_checkpoints_before_exit() {
  stop_after_one_delivery("INT").await;
}
