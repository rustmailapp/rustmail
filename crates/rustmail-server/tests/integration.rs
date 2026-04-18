use std::io::BufReader as StdBufReader;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_rustls::TlsConnector;

use rustmail_api::{AppState, WsEvent, router};
use rustmail_smtp::{ReceivedMessage, Session, SmtpServer, SmtpServerConfig, TlsConfig};
use rustmail_storage::{MessageRepository, initialize_database};

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
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

  let certs = rustls_pemfile::certs(&mut StdBufReader::new(cert_file))
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
  let key = rustls_pemfile::private_key(&mut StdBufReader::new(key_file))
    .unwrap()
    .unwrap();

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
  rustls_pemfile::certs(&mut StdBufReader::new(cert_file))
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

struct ChildGuard(Option<tokio::process::Child>);

impl ChildGuard {
  fn new(child: tokio::process::Child) -> Self {
    Self(Some(child))
  }

  async fn wait_with_timeout(&mut self, secs: u64) -> std::process::ExitStatus {
    let child = self.0.as_mut().expect("child already consumed");
    tokio::time::timeout(Duration::from_secs(secs), child.wait())
      .await
      .expect("child did not exit in time")
      .expect("failed to wait on child")
  }
}

impl Drop for ChildGuard {
  fn drop(&mut self) {
    if let Some(ref mut child) = self.0 {
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

fn spawn_smtp_with_real_session(
  listener: tokio::net::TcpListener,
  tx: mpsc::Sender<ReceivedMessage>,
) {
  spawn_smtp_with_real_session_and_tls(listener, tx, None);
}

fn spawn_smtp_with_real_session_and_tls(
  listener: tokio::net::TcpListener,
  tx: mpsc::Sender<ReceivedMessage>,
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
  let _ = stream.read(&mut buf).await.unwrap();
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

async fn wait_for_tcp(addr: std::net::SocketAddr) {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
  loop {
    match TcpStream::connect(addr).await {
      Ok(probe) => {
        drop(probe);
        tokio::time::sleep(Duration::from_millis(200)).await;
        return;
      }
      Err(_) if tokio::time::Instant::now() < deadline => {
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
      Err(e) => panic!("TCP server at {addr} did not start within 5s: {e}"),
    }
  }
}

#[tokio::test]
async fn smtp_to_api_pipeline() {
  let smtp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let smtp_addr = smtp_listener.local_addr().unwrap();

  let repo = test_repo().await;
  let (smtp_tx, mut smtp_rx) = mpsc::channel(256);
  let (ws_tx, _) = broadcast::channel::<WsEvent>(256);

  spawn_smtp_with_real_session(smtp_listener, smtp_tx);

  let repo_clone = repo.clone();
  tokio::spawn(async move {
    while let Some(msg) = smtp_rx.recv().await {
      repo_clone
        .insert(&msg.sender, &msg.recipients, &msg.raw)
        .await
        .unwrap();
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
    while let Some(msg) = smtp_rx.recv().await {
      repo_clone
        .insert(&msg.sender, &msg.recipients, &msg.raw)
        .await
        .unwrap();
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
  let (tx, mut rx) = mpsc::channel(256);
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

  let (tx, mut rx) = mpsc::channel(256);
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

  let (tx, mut rx) = mpsc::channel(256);
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
  let (ws_tx, _) = broadcast::channel::<WsEvent>(256);

  spawn_smtp_with_real_session(smtp_listener, smtp_tx);

  let webhook_client = reqwest::Client::new();
  let webhook_url_clone = webhook_url.clone();
  let repo_clone = repo.clone();
  let state = AppState::new(repo.clone(), ws_tx, None, None);
  tokio::spawn(async move {
    while let Some(received) = smtp_rx.recv().await {
      if let Ok(summary) = repo_clone
        .insert(&received.sender, &received.recipients, &received.raw)
        .await
      {
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
  let smtp_port = portpicker::pick_unused_port().expect("no free port");

  let mut guard = ChildGuard::new(
    tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"))
      .args([
        "assert",
        "--smtp-port",
        &smtp_port.to_string(),
        "--min-count",
        "1",
        "--subject",
        "CLI Test",
        "--timeout",
        "10s",
        "--log-level",
        "warn",
      ])
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .expect("failed to spawn rustmail assert"),
  );

  let addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port).parse().unwrap();
  wait_for_tcp(addr).await;

  smtp_send(addr, "cli@test.com", "dest@test.com", "CLI Test", "body").await;

  let output = guard.wait_with_timeout(15).await;
  assert!(output.success(), "Expected exit code 0, got {:?}", output);
}

#[tokio::test]
async fn cli_assert_fails_on_timeout() {
  let smtp_port = portpicker::pick_unused_port().expect("no free port");

  let mut guard = ChildGuard::new(
    tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"))
      .args([
        "assert",
        "--smtp-port",
        &smtp_port.to_string(),
        "--min-count",
        "1",
        "--subject",
        "Never Sent",
        "--timeout",
        "2s",
        "--log-level",
        "warn",
      ])
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
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
  let smtp_port = portpicker::pick_unused_port().expect("no free port");

  let mut guard = ChildGuard::new(
    tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"))
      .args([
        "assert",
        "--smtp-port",
        &smtp_port.to_string(),
        "--min-count",
        "1",
        "--subject",
        "Target",
        "--timeout",
        "10s",
        "--log-level",
        "warn",
      ])
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .expect("failed to spawn"),
  );

  let addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port).parse().unwrap();
  wait_for_tcp(addr).await;

  smtp_send(addr, "a@t.com", "b@t.com", "Decoy", "ignored").await;
  smtp_send(addr, "a@t.com", "b@t.com", "Target Email", "match").await;

  let output = guard.wait_with_timeout(15).await;
  assert!(output.success(), "Expected exit 0 after matching 'Target'");
}

#[tokio::test]
async fn smtp_session_limit_rejects_excess() {
  let smtp_port = portpicker::pick_unused_port().expect("no free port");
  let (tx, _) = mpsc::channel::<ReceivedMessage>(256);

  let config = SmtpServerConfig {
    host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    port: smtp_port,
    max_message_size: MAX_MESSAGE_SIZE,
    tls: None,
  };
  let server = SmtpServer::new(config, tx);
  tokio::spawn(async move {
    server.run().await.unwrap();
  });

  let addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port).parse().unwrap();
  wait_for_tcp(addr).await;

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

#[tokio::test]
async fn ws_connection_limit_returns_503() {
  let (app, _, _) = {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool);
    let (ws_tx, _) = broadcast::channel::<WsEvent>(256);
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

  let smtp_port_toml = portpicker::pick_unused_port().expect("no free port");
  let smtp_port_env = portpicker::pick_unused_port().expect("no free port");
  let http_port = portpicker::pick_unused_port().expect("no free port");

  let mut toml_file = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
  write!(
    toml_file,
    "smtp_port = {smtp_port_toml}\nhttp_port = {http_port}\nephemeral = true\n"
  )
  .unwrap();

  let _guard = ChildGuard::new(
    tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"))
      .args(["serve", "--config", toml_file.path().to_str().unwrap()])
      .env("RUSTMAIL_SMTP_PORT", smtp_port_env.to_string())
      .env("RUSTMAIL_LOG_LEVEL", "warn")
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .expect("failed to spawn"),
  );

  let env_addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port_env).parse().unwrap();
  let toml_addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port_toml).parse().unwrap();

  wait_for_tcp(env_addr).await;

  let toml_reachable = TcpStream::connect(toml_addr).await.is_ok();
  assert!(
    !toml_reachable,
    "TOML port {smtp_port_toml} should NOT be listening (env override)"
  );
}

#[tokio::test]
async fn config_toml_used_when_no_env() {
  use std::io::Write;

  let smtp_port = portpicker::pick_unused_port().expect("no free port");
  let http_port = portpicker::pick_unused_port().expect("no free port");

  let mut toml_file = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
  write!(
    toml_file,
    "smtp_port = {smtp_port}\nhttp_port = {http_port}\nephemeral = true\n"
  )
  .unwrap();

  let _guard = ChildGuard::new(
    tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"))
      .args(["serve", "--config", toml_file.path().to_str().unwrap()])
      .env_remove("RUSTMAIL_SMTP_PORT")
      .env_remove("RUSTMAIL_HTTP_PORT")
      .env("RUSTMAIL_LOG_LEVEL", "warn")
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .expect("failed to spawn"),
  );

  let addr: std::net::SocketAddr = format!("127.0.0.1:{}", smtp_port).parse().unwrap();
  wait_for_tcp(addr).await;
}

#[tokio::test]
async fn smtp_tls_requires_both_cert_and_key() {
  let smtp_port = portpicker::pick_unused_port().expect("no free port");
  let http_port = portpicker::pick_unused_port().expect("no free port");
  let cert_path = starttls_cert_path();
  let key_path = starttls_key_path();

  for (cert, key) in [
    (Some(cert_path.as_path()), None),
    (None, Some(key_path.as_path())),
  ] {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_rustmail"));
    command
      .args([
        "serve",
        "--smtp-port",
        &smtp_port.to_string(),
        "--http-port",
        &http_port.to_string(),
        "--ephemeral",
        "--log-level",
        "warn",
      ])
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped());

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
