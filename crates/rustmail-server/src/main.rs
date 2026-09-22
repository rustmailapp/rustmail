use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{info, warn};

use rustmail_api::{AppState, Hostname, Origin, WsEvent, WsFrame};
use rustmail_smtp::{
  Delivery, DeliveryAck, ReceivedMessage, STORE_ACK_TIMEOUT, SmtpServer, SmtpServerConfig,
  TlsConfig,
};
use rustmail_storage::{
  MessageRepository, MessageSummary, PreparedMessage, format_iso8601, initialize_database,
};

#[derive(Parser)]
#[command(
  name = "rustmail",
  version = env!("RUSTMAIL_BUILD_VERSION"),
  about = "A modern SMTP mail catcher"
)]
struct Cli {
  #[command(subcommand)]
  command: Option<Command>,

  #[command(flatten)]
  serve: ServeArgs,
}

#[derive(Subcommand)]
enum Command {
  /// Start the mail catcher server (default when no subcommand is given)
  Serve(ServeArgs),
  /// Start ephemeral SMTP, wait for matching emails, exit 0/1
  Assert(AssertArgs),
  /// Launch the interactive terminal UI
  #[cfg(feature = "tui")]
  Tui(TuiArgs),
}

#[derive(Parser, Clone, Default)]
struct ServeArgs {
  #[arg(long, env = "RUSTMAIL_BIND", default_value = "127.0.0.1")]
  bind: String,

  #[arg(long, env = "RUSTMAIL_SMTP_PORT", default_value = "1025")]
  smtp_port: u16,

  #[arg(long, env = "RUSTMAIL_HTTP_PORT", default_value = "8025")]
  http_port: u16,

  #[arg(long, env = "RUSTMAIL_DB_PATH")]
  db_path: Option<PathBuf>,

  #[arg(long, env = "RUSTMAIL_EPHEMERAL", default_value = "false")]
  ephemeral: bool,

  #[arg(long, env = "RUSTMAIL_MAX_MESSAGE_SIZE", default_value = "10485760")]
  max_message_size: usize,

  #[arg(long, env = "RUSTMAIL_SMTP_TLS_CERT")]
  smtp_tls_cert: Option<PathBuf>,

  #[arg(long, env = "RUSTMAIL_SMTP_TLS_KEY")]
  smtp_tls_key: Option<PathBuf>,

  #[arg(long, env = "RUSTMAIL_RETENTION", default_value = "0")]
  retention: u64,

  #[arg(long, env = "RUSTMAIL_MAX_MESSAGES", default_value = "0")]
  max_messages: i64,

  #[arg(long, env = "RUSTMAIL_LOG_LEVEL", default_value = "info")]
  log_level: String,

  #[arg(long, env = "RUSTMAIL_WEBHOOK_URL")]
  webhook_url: Option<String>,

  /// Allowed release target in host:port format (e.g., smtp.example.com:587)
  #[arg(long, env = "RUSTMAIL_RELEASE_HOST")]
  release_host: Option<String>,

  /// Extra origin allowed to open the WebSocket, e.g. https://mail.example.com
  #[arg(
    long = "allowed-origin",
    env = "RUSTMAIL_ALLOWED_ORIGINS",
    value_delimiter = ','
  )]
  allowed_origins: Vec<String>,

  /// Host name browsers may reach RustMail on, e.g. mail.example.com
  #[arg(
    long = "allowed-host",
    env = "RUSTMAIL_ALLOWED_HOSTS",
    value_delimiter = ','
  )]
  allowed_hosts: Vec<String>,

  #[arg(long)]
  config: Option<String>,
}

#[derive(Parser)]
struct AssertArgs {
  #[arg(long, default_value = "1025")]
  smtp_port: u16,

  #[arg(long, default_value = "10485760")]
  max_message_size: usize,

  #[arg(long, default_value = "1")]
  min_count: u64,

  #[arg(long)]
  subject: Option<String>,

  #[arg(long)]
  sender: Option<String>,

  #[arg(long)]
  recipient: Option<String>,

  #[arg(long, default_value = "30s")]
  timeout: String,

  #[arg(long, default_value = "info")]
  log_level: String,
}

#[cfg(feature = "tui")]
#[derive(Parser)]
struct TuiArgs {
  #[arg(long, env = "RUSTMAIL_BIND", default_value = "127.0.0.1")]
  host: String,

  #[arg(long, env = "RUSTMAIL_HTTP_PORT", default_value = "8025")]
  port: u16,
}

#[derive(Deserialize, Default)]
struct TomlConfig {
  bind: Option<String>,
  smtp_port: Option<u16>,
  http_port: Option<u16>,
  db_path: Option<String>,
  ephemeral: Option<bool>,
  max_message_size: Option<usize>,
  smtp_tls_cert: Option<String>,
  smtp_tls_key: Option<String>,
  retention: Option<u64>,
  max_messages: Option<i64>,
  log_level: Option<String>,
  webhook_url: Option<String>,
  release_host: Option<String>,
  allowed_origins: Option<Vec<String>>,
  allowed_hosts: Option<Vec<String>>,
}

fn apply_toml_to_env(config: &TomlConfig) {
  fn set_if_absent(key: &str, value: &str) {
    if std::env::var(key).is_err() {
      unsafe { std::env::set_var(key, value) };
    }
  }

  if let Some(v) = &config.bind {
    set_if_absent("RUSTMAIL_BIND", v);
  }
  if let Some(v) = config.smtp_port {
    set_if_absent("RUSTMAIL_SMTP_PORT", &v.to_string());
  }
  if let Some(v) = config.http_port {
    set_if_absent("RUSTMAIL_HTTP_PORT", &v.to_string());
  }
  if let Some(v) = &config.db_path {
    set_if_absent("RUSTMAIL_DB_PATH", v);
  }
  if let Some(v) = config.ephemeral {
    set_if_absent("RUSTMAIL_EPHEMERAL", &v.to_string());
  }
  if let Some(v) = config.max_message_size {
    set_if_absent("RUSTMAIL_MAX_MESSAGE_SIZE", &v.to_string());
  }
  if let Some(v) = &config.smtp_tls_cert {
    set_if_absent("RUSTMAIL_SMTP_TLS_CERT", v);
  }
  if let Some(v) = &config.smtp_tls_key {
    set_if_absent("RUSTMAIL_SMTP_TLS_KEY", v);
  }
  if let Some(v) = config.retention {
    set_if_absent("RUSTMAIL_RETENTION", &v.to_string());
  }
  if let Some(v) = config.max_messages {
    set_if_absent("RUSTMAIL_MAX_MESSAGES", &v.to_string());
  }
  if let Some(v) = &config.log_level {
    set_if_absent("RUSTMAIL_LOG_LEVEL", v);
  }
  if let Some(v) = &config.webhook_url {
    set_if_absent("RUSTMAIL_WEBHOOK_URL", v);
  }
  if let Some(v) = &config.release_host {
    set_if_absent("RUSTMAIL_RELEASE_HOST", v);
  }
  if let Some(v) = &config.allowed_origins {
    set_if_absent("RUSTMAIL_ALLOWED_ORIGINS", &v.join(","));
  }
  if let Some(v) = &config.allowed_hosts {
    set_if_absent("RUSTMAIL_ALLOWED_HOSTS", &v.join(","));
  }
}

fn main() -> Result<()> {
  pre_load_toml_config()?;

  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()?
    .block_on(async_main())
}

fn pre_load_toml_config() -> Result<()> {
  let args: Vec<String> = std::env::args().collect();
  let config_path = args
    .windows(2)
    .find_map(|w| (w[0] == "--config").then(|| w[1].clone()))
    .or_else(|| {
      args
        .iter()
        .find_map(|a| a.strip_prefix("--config=").map(String::from))
    });

  if let Some(path) = config_path {
    let contents = std::fs::read_to_string(&path)?;
    let toml_config: TomlConfig = toml::from_str(&contents)?;
    // SAFETY: Called before tokio runtime starts; only the main thread exists.
    apply_toml_to_env(&toml_config);
  }
  Ok(())
}

async fn async_main() -> Result<()> {
  let cli = Cli::parse();

  match cli.command {
    Some(Command::Assert(args)) => run_assert(args).await,
    Some(Command::Serve(args)) => run_serve(args).await,
    #[cfg(feature = "tui")]
    Some(Command::Tui(args)) => rustmail_tui::run(&args.host, args.port).await,
    None => run_serve(cli.serve).await,
  }
}

async fn run_assert(args: AssertArgs) -> Result<()> {
  tracing_subscriber::fmt()
    .with_env_filter(args.log_level.as_str())
    .init();

  let timeout = parse_duration(&args.timeout)?;
  info!(
    smtp_port = args.smtp_port,
    min_count = args.min_count,
    ?timeout,
    "Assert mode: waiting for matching emails"
  );

  let repo = open_repository(IN_MEMORY_DB_URL, true).await?;
  let (smtp_tx, mut smtp_rx) = mpsc::channel::<Delivery>(256);

  let smtp_config = SmtpServerConfig {
    host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    port: args.smtp_port,
    max_message_size: args.max_message_size,
    tls: None,
  };
  let smtp_server = SmtpServer::new(smtp_config, smtp_tx);

  let smtp_handle = tokio::spawn(async move {
    if let Err(e) = smtp_server.run().await {
      tracing::error!(error = %e, "SMTP server error");
    }
  });

  let min_count = args.min_count;
  let subject_filter = args.subject.clone();
  let sender_filter = args.sender.clone();
  let recipient_filter = args.recipient.clone();

  let checker = {
    let repo = repo.clone();
    tokio::spawn(async move {
      let mut batch = Vec::with_capacity(MAX_BATCH_MESSAGES);
      while next_batch(&mut smtp_rx, &mut None, &mut batch).await {
        if store_batch(&repo, std::mem::take(&mut batch))
          .await
          .is_empty()
        {
          continue;
        }
        let count = repo
          .count_matching(
            subject_filter.as_deref(),
            sender_filter.as_deref(),
            recipient_filter.as_deref(),
          )
          .await
          .unwrap_or(0);
        if count as u64 >= min_count {
          info!(count, "Assert criteria met");
          return true;
        }
      }
      false
    })
  };

  let result = tokio::select! {
      result = checker => result.unwrap_or(false),
      _ = tokio::time::sleep(timeout) => {
          tracing::error!("Timeout: not enough matching emails received");
          false
      }
  };

  smtp_handle.abort();

  if result {
    info!("Assert passed");
    std::process::exit(0);
  } else {
    std::process::exit(1);
  }
}

fn parse_duration(s: &str) -> Result<std::time::Duration> {
  let s = s.trim();
  if let Some(secs) = s.strip_suffix('s') {
    Ok(std::time::Duration::from_secs(secs.parse()?))
  } else if let Some(mins) = s.strip_suffix('m') {
    Ok(std::time::Duration::from_secs(mins.parse::<u64>()? * 60))
  } else {
    Ok(std::time::Duration::from_secs(s.parse()?))
  }
}

fn default_db_path() -> PathBuf {
  dirs::data_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join("rustmail")
    .join("rustmail.db")
}

const IN_MEMORY_DB_URL: &str = "sqlite::memory:";
/// Connections a file database's reader pool may open, besides its writer.
const FILE_DB_READER_CONNECTIONS: u32 = 4;
/// How long a stop waits for queued deliveries and open HTTP connections to
/// finish before closing the database anyway.
///
/// Together with [`DB_CLOSE_DEADLINE`] it stays well under Docker's default
/// 10 s stop timeout, so the WAL is checkpointed before the container runtime
/// resorts to `SIGKILL`.
const SHUTDOWN_DRAIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
/// Most queued deliveries taken into one batch.
const MAX_BATCH_MESSAGES: usize = 32;
/// Most raw mail, in bytes, committed in one transaction.
///
/// Small transactional mail gains the most from sharing a commit; large mail
/// is bound by writing its bytes, so it gains little and would only hold the
/// write lock longer.
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;
/// Raw size from which a message is parsed off the async runtime.
const BLOCKING_PARSE_THRESHOLD_BYTES: usize = 256 * 1024;
/// How long closing the database, and with it the final WAL checkpoint, may take.
const DB_CLOSE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// Resolves when the process is asked to stop, by `SIGTERM` or Ctrl-C.
///
/// `SIGTERM` needs a handler of its own: as PID 1 in a container the kernel
/// ignores it by default, so `docker stop` would otherwise wait out its
/// timeout and then `SIGKILL` the server.
async fn shutdown_signal() -> std::io::Result<()> {
  #[cfg(unix)]
  {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
      result = tokio::signal::ctrl_c() => result,
      _ = terminate.recv() => Ok(()),
    }
  }
  #[cfg(not(unix))]
  tokio::signal::ctrl_c().await
}

/// Receives the next run of queued deliveries into `batch`, closing the queue
/// once `stop` fires.
///
/// Takes whatever is already queued, up to [`MAX_BATCH_MESSAGES`], without
/// waiting for more: a lone message is stored as promptly as before, and a
/// burst shares its commits. Closing refuses further hand-offs, so a session
/// still in progress tells its sender to retry, while deliveries already
/// queued keep coming. The caller therefore drains the queue and sees `false`
/// once it is empty, even though SMTP sessions still hold senders.
async fn next_batch(
  deliveries: &mut mpsc::Receiver<Delivery>,
  stop: &mut Option<oneshot::Receiver<()>>,
  batch: &mut Vec<Delivery>,
) -> bool {
  if let Some(signal) = stop {
    tokio::select! {
      received = deliveries.recv_many(batch, MAX_BATCH_MESSAGES) => return received > 0,
      _ = signal => {
        deliveries.close();
        *stop = None;
      }
    }
  }
  deliveries.recv_many(batch, MAX_BATCH_MESSAGES).await > 0
}

/// Parses a captured message ahead of its write.
///
/// A message at or past [`BLOCKING_PARSE_THRESHOLD_BYTES`] is parsed on the
/// blocking pool: decoding megabytes of base64 inline would stall a runtime
/// worker, and with it every SMTP session scheduled there.
async fn prepare(received: ReceivedMessage) -> Result<PreparedMessage, tokio::task::JoinError> {
  let ReceivedMessage {
    sender,
    recipients,
    raw,
  } = received;
  if raw.len() < BLOCKING_PARSE_THRESHOLD_BYTES {
    return Ok(PreparedMessage::parse(sender, &recipients, raw));
  }
  tokio::task::spawn_blocking(move || PreparedMessage::parse(sender, &recipients, raw)).await
}

/// Splits a received batch into groups of at most [`MAX_BATCH_BYTES`] of raw
/// mail, each to be committed as one transaction.
///
/// Arrival order is kept across and within groups. A message larger than the
/// cap forms a group of its own, so large mail still commits one at a time.
fn transactions(deliveries: Vec<Delivery>) -> Vec<Vec<(ReceivedMessage, DeliveryAck)>> {
  let mut groups: Vec<Vec<(ReceivedMessage, DeliveryAck)>> = Vec::new();
  let mut group_bytes = 0;
  for (received, ack) in deliveries.into_iter().map(Delivery::into_parts) {
    let size = received.raw.len();
    match groups.last_mut() {
      Some(group) if group_bytes + size <= MAX_BATCH_BYTES => {
        group_bytes += size;
        group.push((received, ack));
      }
      _ => {
        group_bytes = size;
        groups.push(vec![(received, ack)]);
      }
    }
  }
  groups
}

/// How long after a batch is taken off the queue it may still fall back to
/// storing its messages one at a time.
///
/// A session answers `451` once [`STORE_ACK_TIMEOUT`] passes without a
/// verdict, and a message committed after that is captured twice once the
/// sender retries. Half the timeout leaves the other half for the insert
/// already running when the budget runs out, which SQLite's `busy_timeout`
/// and the repository's retries can stretch to ten seconds, and for the time
/// the delivery waited in the queue before its batch was taken.
const FALLBACK_BUDGET: std::time::Duration =
  std::time::Duration::from_secs(STORE_ACK_TIMEOUT.as_secs() / 2);

/// Stores a batch of captured messages and tells each waiting SMTP session
/// what happened.
///
/// Each session holds its reply until this answers, and a message is only
/// acknowledged once the transaction holding it has committed, so a `250`
/// still means stored. A refused message is reported to its sender as a
/// temporary failure, which is the whole point of waiting — a catcher that
/// answered on the hand-off would lose it instead. Returns the stored
/// messages in arrival order.
async fn store_batch(repo: &MessageRepository, deliveries: Vec<Delivery>) -> Vec<MessageSummary> {
  let fallback_deadline = Instant::now() + FALLBACK_BUDGET;
  let mut stored = Vec::with_capacity(deliveries.len());
  for group in transactions(deliveries) {
    stored.extend(store_group(repo, group, fallback_deadline).await);
  }
  stored
}

/// Stores a batch, then announces each stored message to WebSocket clients in
/// the order it was stored.
async fn process_batch(
  repo: &MessageRepository,
  state: &AppState,
  deliveries: Vec<Delivery>,
) -> Vec<MessageSummary> {
  let stored = store_batch(repo, deliveries).await;
  for summary in &stored {
    state.broadcast(WsEvent::MessageNew(summary.clone()));
  }
  stored
}

async fn store_group(
  repo: &MessageRepository,
  group: Vec<(ReceivedMessage, DeliveryAck)>,
  fallback_deadline: Instant,
) -> Vec<MessageSummary> {
  let mut messages = Vec::with_capacity(group.len());
  let mut acks = Vec::with_capacity(group.len());
  for (received, ack) in group {
    if ack.is_abandoned() {
      warn!("Skipped a message whose session gave up waiting; the sender was asked to retry");
      continue;
    }
    match prepare(received).await {
      Ok(message) => {
        messages.push(message);
        acks.push(ack);
      }
      Err(e) => {
        ack.rejected();
        tracing::error!(error = %e, "Refused a message that could not be parsed; the sender was asked to retry");
      }
    }
  }
  if messages.is_empty() {
    return Vec::new();
  }

  match repo.insert_batch(&messages).await {
    Ok(summaries) => {
      acks.into_iter().for_each(DeliveryAck::stored);
      summaries
    }
    Err(e) if messages.len() == 1 || e.is_store_wide() => {
      acks.into_iter().for_each(DeliveryAck::rejected);
      tracing::error!(error = %e, count = messages.len(), "Refused messages the store would not take; the senders were asked to retry");
      Vec::new()
    }
    Err(e) => {
      warn!(error = %e, count = messages.len(), "A batch failed to commit; storing its messages one at a time");
      store_one_by_one(repo, messages, acks, fallback_deadline).await
    }
  }
}

/// Stores each message in its own transaction, so a message the store
/// refuses fails alone rather than taking its batch with it.
///
/// Refuses every message not yet tried once the store reports a failure that
/// is not specific to one message, or once `deadline` has passed, since a
/// message committed after its session answered `451` is captured again when
/// the sender retries.
async fn store_one_by_one(
  repo: &MessageRepository,
  messages: Vec<PreparedMessage>,
  acks: Vec<DeliveryAck>,
  deadline: Instant,
) -> Vec<MessageSummary> {
  let mut stored = Vec::with_capacity(messages.len());
  let mut pending = messages.iter().zip(acks);
  while let Some((message, ack)) = pending.next() {
    if ack.is_abandoned() {
      warn!("Skipped a message whose session gave up waiting; the sender was asked to retry");
      continue;
    }
    if Instant::now() >= deadline {
      ack.rejected();
      let remaining = 1 + reject_rest(pending);
      warn!(
        count = remaining,
        "A batch ran out of time to store its messages one at a time; the senders were asked to retry"
      );
      break;
    }
    match repo.insert_prepared(message).await {
      Ok(summary) => {
        ack.stored();
        stored.push(summary);
      }
      Err(e) if e.is_store_wide() => {
        ack.rejected();
        let remaining = 1 + reject_rest(pending);
        tracing::error!(error = %e, count = remaining, "Refused messages the store would not take; the senders were asked to retry");
        break;
      }
      Err(e) => {
        ack.rejected();
        tracing::error!(error = %e, "Refused a message the store would not take; the sender was asked to retry");
      }
    }
  }
  stored
}

/// Refuses every delivery left in `pending`, returning how many there were.
fn reject_rest<'a>(pending: impl Iterator<Item = (&'a PreparedMessage, DeliveryAck)>) -> usize {
  pending.map(|(_, ack)| ack.rejected()).count()
}

/// Pool options for one connection that is never reaped or recycled.
fn single_connection() -> sqlx::sqlite::SqlitePoolOptions {
  sqlx::sqlite::SqlitePoolOptions::new()
    .min_connections(1)
    .max_connections(1)
    .idle_timeout(None)
    .max_lifetime(None)
}

/// Opens a SQLite connection pool for `db_url`: the reader pool of a file
/// database, or the one connection an in-memory database lives in.
///
/// In-memory pools are pinned to a single permanent connection. SQLite drops
/// an in-memory database once its last connection closes, and the pool would
/// otherwise reap every idle connection after ten minutes, silently discarding
/// all captured mail. A single connection also keeps concurrent writers off
/// shared-cache locking, which reports `SQLITE_LOCKED` instead of the
/// `SQLITE_BUSY` that `busy_timeout` retries.
async fn connect_pool(db_url: &str, in_memory: bool) -> Result<sqlx::SqlitePool> {
  let pool_options = if in_memory {
    single_connection()
  } else {
    sqlx::sqlite::SqlitePoolOptions::new().max_connections(FILE_DB_READER_CONNECTIONS)
  };

  let connect_options = rustmail_storage::connect_options(db_url)
    .with_context(|| format!("invalid database URL: {db_url}"))?;

  pool_options
    .connect_with(connect_options)
    .await
    .with_context(|| format!("failed to open database: {db_url}"))
}

/// Opens the one connection every write to a file database goes through.
///
/// It stays open for the life of the process, so its page cache stays warm.
async fn connect_writer(db_url: &str) -> Result<sqlx::SqlitePool> {
  let connect_options = rustmail_storage::connect_options(db_url)
    .with_context(|| format!("invalid database URL: {db_url}"))?;

  single_connection()
    .connect_with(connect_options)
    .await
    .with_context(|| format!("failed to open database for writing: {db_url}"))
}

/// Opens the repository for `db_url` and creates its schema.
///
/// A file database gets a dedicated writer connection beside its pool of
/// readers. An in-memory database lives in a single connection, which then
/// serves both.
async fn open_repository(db_url: &str, in_memory: bool) -> Result<MessageRepository> {
  if in_memory {
    let pool = connect_pool(db_url, true).await?;
    initialize_database(&pool).await?;
    return Ok(MessageRepository::new(pool));
  }
  let writer = connect_writer(db_url).await?;
  initialize_database(&writer).await?;
  let readers = connect_pool(db_url, false).await?;
  Ok(MessageRepository::with_writer(readers, writer))
}

fn install_rustls_crypto_provider() {
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }
}

fn load_smtp_tls_config(cert_path: &Path, key_path: &Path) -> Result<TlsConfig> {
  install_rustls_crypto_provider();

  let cert_file = std::fs::File::open(cert_path).with_context(|| {
    format!(
      "failed to open SMTP TLS certificate: {}",
      cert_path.display()
    )
  })?;
  let key_file = std::fs::File::open(key_path).with_context(|| {
    format!(
      "failed to open SMTP TLS private key: {}",
      key_path.display()
    )
  })?;

  let cert_chain = CertificateDer::pem_reader_iter(cert_file)
    .collect::<Result<Vec<_>, _>>()
    .context("failed to parse SMTP TLS certificate PEM")?;
  if cert_chain.is_empty() {
    anyhow::bail!("SMTP TLS certificate file did not contain any certificates");
  }

  let private_key = match PrivateKeyDer::from_pem_reader(key_file) {
    Ok(key) => key,
    Err(rustls::pki_types::pem::Error::NoItemsFound) => {
      anyhow::bail!("SMTP TLS private key file did not contain a supported private key")
    }
    Err(err) => return Err(err).context("failed to parse SMTP TLS private key PEM"),
  };

  let server_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(cert_chain, private_key)
    .context("failed to build SMTP TLS server config")?;

  Ok(TlsConfig {
    server_config: Arc::new(server_config),
  })
}

fn build_smtp_tls_config(cert: Option<&Path>, key: Option<&Path>) -> Result<Option<TlsConfig>> {
  match (cert, key) {
    (Some(cert), Some(key)) => load_smtp_tls_config(cert, key).map(Some),
    (None, None) => Ok(None),
    (Some(_), None) => {
      anyhow::bail!("SMTP TLS configuration requires both --smtp-tls-cert and --smtp-tls-key")
    }
    (None, Some(_)) => {
      anyhow::bail!("SMTP TLS configuration requires both --smtp-tls-cert and --smtp-tls-key")
    }
  }
}

fn validate_webhook_url(url: &str) -> Result<()> {
  let parsed: reqwest::Url = url
    .parse()
    .map_err(|_| anyhow::anyhow!("invalid webhook URL: {}", url))?;

  match parsed.scheme() {
    "http" | "https" => {}
    s => anyhow::bail!("webhook URL scheme must be http or https, got: {}", s),
  }

  let host = parsed
    .host_str()
    .ok_or_else(|| anyhow::anyhow!("webhook URL has no host"))?;

  if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]" {
    anyhow::bail!("webhook URL must not point to localhost: {}", url);
  }

  if let Ok(ip) = host.parse::<std::net::IpAddr>()
    && is_private_ip(ip)
  {
    anyhow::bail!(
      "webhook URL must not point to a private/reserved IP: {}",
      url
    );
  }

  Ok(())
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
  match ip {
    std::net::IpAddr::V4(v4) => {
      v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // CGN 100.64/10
    }
    std::net::IpAddr::V6(v6) => {
      if let Some(v4) = v6.to_ipv4_mapped() {
        return is_private_ip(std::net::IpAddr::V4(v4));
      }
      let segs = v6.segments();
      v6.is_loopback()
        || v6.is_unspecified()
        || (segs[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (segs[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
    }
  }
}

/// Reads the configured WebSocket origins, refusing startup on a bad one.
///
/// A misspelled origin would otherwise fail silently at handshake time, which
/// is the wrong place to learn about it. Blank entries are dropped rather than
/// refused: an empty TOML list and an unset `RUSTMAIL_ALLOWED_ORIGINS` both
/// reach clap as one empty value, and both mean no origin was configured.
fn parse_allowed_origins(values: &[String]) -> Result<Vec<Origin>> {
  values
    .iter()
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
    .map(|value| value.parse::<Origin>().context("invalid --allowed-origin"))
    .collect()
}

/// Reads the host names browsers may reach RustMail on.
///
/// Blank entries are dropped for the same reason as in
/// [`parse_allowed_origins`].
fn parse_allowed_hosts(values: &[String]) -> Result<Vec<Hostname>> {
  values
    .iter()
    .map(|value| value.trim())
    .filter(|value| !value.is_empty())
    .map(|value| value.parse::<Hostname>().context("invalid --allowed-host"))
    .collect()
}

fn parse_bind_addr(bind: &str) -> Result<std::net::IpAddr> {
  bind.parse().map_err(|_| {
    anyhow::anyhow!(
      "invalid bind address '{}': expected IP address (e.g., 127.0.0.1 or ::1)",
      bind
    )
  })
}

fn parse_release_host(s: &str) -> (String, Option<u16>) {
  if let Some((host, port_str)) = s.rsplit_once(':')
    && let Ok(port) = port_str.parse::<u16>()
  {
    return (host.to_string(), Some(port));
  }
  (s.to_string(), None)
}

/// Runs a single retention sweep: purges messages older than `retention_hours`
/// and trims to `max_messages` when configured, broadcasting `MessageDelete`
/// events for each removed id.
///
/// `now` is injected so callers can drive deterministic cutoffs in tests.
async fn run_retention_tick(
  repo: &MessageRepository,
  state: &AppState,
  retention_hours: u64,
  max_messages: i64,
  now: OffsetDateTime,
) {
  if retention_hours == 0 && max_messages == 0 {
    return;
  }
  if retention_hours > 0 {
    let cutoff = now - time::Duration::hours(retention_hours as i64);
    let cutoff_str = format_iso8601(cutoff);
    match repo.delete_older_than(&cutoff_str).await {
      Ok(ids) if !ids.is_empty() => {
        tracing::info!(deleted = ids.len(), "Retention: purged old messages");
        for id in ids {
          state.broadcast(WsEvent::MessageDelete { id });
        }
      }
      Err(e) => {
        tracing::error!(error = %e, "Retention: failed to purge");
      }
      _ => {}
    }
  }
  if max_messages > 0 {
    match repo.trim_to_max(max_messages).await {
      Ok(ids) if !ids.is_empty() => {
        tracing::info!(deleted = ids.len(), "Retention: trimmed to max");
        for id in ids {
          state.broadcast(WsEvent::MessageDelete { id });
        }
      }
      Err(e) => {
        tracing::error!(error = %e, "Retention: failed to trim");
      }
      _ => {}
    }
  }
}

async fn run_serve(args: ServeArgs) -> Result<()> {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| args.log_level.clone().into()),
    )
    .init();

  let bind_addr = parse_bind_addr(&args.bind)?;
  let allowed_origins = parse_allowed_origins(&args.allowed_origins)?;
  let allowed_hosts = parse_allowed_hosts(&args.allowed_hosts)?;
  let smtp_tls =
    build_smtp_tls_config(args.smtp_tls_cert.as_deref(), args.smtp_tls_key.as_deref())?;

  if !bind_addr.is_loopback() {
    tracing::warn!(
      bind = %args.bind,
      "Binding to non-loopback address with no authentication. All API endpoints are accessible to the network."
    );
  }

  let db_url = if args.ephemeral {
    info!("Running in ephemeral mode (in-memory database)");
    IN_MEMORY_DB_URL.to_string()
  } else {
    let db_path = args.db_path.unwrap_or_else(default_db_path);
    if let Some(parent) = db_path.parent() {
      std::fs::create_dir_all(parent)?;
    }
    info!(path = %db_path.display(), "Using persistent database");
    format!("sqlite:{}?mode=rwc", db_path.display())
  };

  let repo = open_repository(&db_url, args.ephemeral).await?;

  let (release_host, release_port) = args.release_host.as_deref().map(parse_release_host).unzip();
  let release_host: Option<String> = release_host;
  let release_port: Option<u16> = release_port.flatten();

  let (smtp_tx, mut smtp_rx) = mpsc::channel::<Delivery>(256);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);

  let state = AppState::new(repo.clone(), ws_tx, release_host, release_port)
    .with_allowed_origins(allowed_origins.clone())
    .with_allowed_hosts(allowed_hosts.clone());

  let smtp_config = SmtpServerConfig {
    host: bind_addr,
    port: args.smtp_port,
    max_message_size: args.max_message_size,
    tls: smtp_tls,
  };
  let smtp_server = SmtpServer::new(smtp_config, smtp_tx);

  if let Some(ref url) = args.webhook_url {
    validate_webhook_url(url)?;
  }

  let webhook_client = args.webhook_url.as_ref().map(|_| reqwest::Client::new());
  let webhook_url = args.webhook_url.clone();
  let webhook_semaphore = Arc::new(tokio::sync::Semaphore::new(10));

  let (stop_processor, processor_stop) = oneshot::channel::<()>();
  let mut message_processor = {
    let repo = repo.clone();
    let state = state.clone();
    let mut stop = Some(processor_stop);
    tokio::spawn(async move {
      let mut batch = Vec::with_capacity(MAX_BATCH_MESSAGES);
      while next_batch(&mut smtp_rx, &mut stop, &mut batch).await {
        let stored = process_batch(&repo, &state, std::mem::take(&mut batch)).await;
        let (Some(client), Some(url)) = (&webhook_client, &webhook_url) else {
          continue;
        };
        for payload in stored {
          let client = client.clone();
          let url = url.clone();
          let sem = webhook_semaphore.clone();
          tokio::spawn(async move {
            let _permit = match sem.acquire().await {
              Ok(p) => p,
              Err(_) => return,
            };
            if let Err(e) = client
              .post(&url)
              .json(&payload)
              .timeout(std::time::Duration::from_secs(5))
              .send()
              .await
            {
              tracing::warn!(error = %e, "Webhook delivery failed");
            }
          });
        }
      }
    })
  };

  let mut retention_task = {
    let repo = repo.clone();
    let state = state.clone();
    let retention_hours = args.retention;
    let max_messages = args.max_messages;
    tokio::spawn(async move {
      if retention_hours == 0 && max_messages == 0 {
        std::future::pending::<()>().await;
        return;
      }
      let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
      loop {
        interval.tick().await;
        run_retention_tick(
          &repo,
          &state,
          retention_hours,
          max_messages,
          OffsetDateTime::now_utc(),
        )
        .await;
      }
    })
  };

  let http_addr = format!("{}:{}", args.bind, args.http_port);
  let listener = tokio::net::TcpListener::bind(&http_addr).await?;
  info!(port = args.http_port, "HTTP server listening");

  if args.retention > 0 {
    info!(
      hours = args.retention,
      "Retention policy: delete after hours"
    );
  }
  if args.max_messages > 0 {
    info!(max = args.max_messages, "Retention policy: max messages");
  }
  if args.webhook_url.is_some() {
    info!("Webhook notifications enabled");
  }
  if let Some(ref host) = args.release_host {
    info!(host = %host, "Email release enabled");
  }
  if !allowed_origins.is_empty() {
    let origins: Vec<String> = allowed_origins.iter().map(Origin::to_string).collect();
    info!(
      origins = %origins.join(", "),
      "WebSocket accepts these origins besides the one it is served from"
    );
  }
  if !allowed_hosts.is_empty() {
    let hosts: Vec<String> = allowed_hosts.iter().map(Hostname::to_string).collect();
    info!(
      hosts = %hosts.join(", "),
      "Browsers may reach RustMail on these names besides addresses and localhost"
    );
  }

  let app = rustmail_api::router(state);
  let (stop_http, http_stop) = oneshot::channel::<()>();
  let http_server = axum::serve(listener, app)
    .with_graceful_shutdown(async {
      let _ = http_stop.await;
    })
    .into_future();
  tokio::pin!(http_server);

  tokio::select! {
      result = smtp_server.run() => {
          match result {
              Err(e) => anyhow::bail!("SMTP server failed: {e}"),
              Ok(()) => anyhow::bail!("SMTP server exited unexpectedly"),
          }
      }
      result = &mut http_server => {
          match result {
              Err(e) => anyhow::bail!("HTTP server failed: {e}"),
              Ok(()) => anyhow::bail!("HTTP server exited unexpectedly"),
          }
      }
      _ = &mut message_processor => {
          anyhow::bail!("Message processor stopped unexpectedly");
      }
      _ = &mut retention_task => {}
      result = shutdown_signal() => {
          result.context("failed to listen for shutdown signals")?;
      }
  }

  info!("Shutting down: SMTP closed, draining queued messages");
  let _ = stop_processor.send(());
  let _ = stop_http.send(());

  let drained = tokio::time::timeout(SHUTDOWN_DRAIN_DEADLINE, async {
    tokio::join!(&mut http_server, &mut message_processor)
  })
  .await;
  match drained {
    Ok((Err(e), _)) => warn!(error = %e, "HTTP server failed while shutting down"),
    Ok((_, Err(e))) => warn!(error = %e, "Message processor failed while draining"),
    Ok(_) => {}
    Err(_) => {
      message_processor.abort();
      warn!(
        deadline_secs = SHUTDOWN_DRAIN_DEADLINE.as_secs(),
        "Shutdown drain deadline elapsed with work in flight; closing the database anyway"
      );
    }
  }

  retention_task.abort();

  if tokio::time::timeout(DB_CLOSE_DEADLINE, repo.close())
    .await
    .is_err()
  {
    warn!(
      deadline_secs = DB_CLOSE_DEADLINE.as_secs(),
      "Database did not close in time; the WAL will be recovered on next start"
    );
  }

  Ok(())
}

#[cfg(test)]
mod version_tests {
  use super::*;
  use clap::CommandFactory;
  use clap::error::ErrorKind;

  #[test]
  fn answers_the_version_flag() {
    let error = Cli::command()
      .try_get_matches_from(["rustmail", "--version"])
      .expect_err("--version stops parsing to print the version");

    assert_eq!(error.kind(), ErrorKind::DisplayVersion);
    assert!(
      error.to_string().contains(env!("RUSTMAIL_BUILD_VERSION")),
      "the Homebrew formula asserts this output carries the release version"
    );
  }

  #[test]
  fn resolves_a_non_empty_version() {
    assert!(!env!("RUSTMAIL_BUILD_VERSION").is_empty());
  }
}

#[cfg(test)]
mod delivery_tests {
  use super::*;
  use rustmail_smtp::DeliveryOutcome;

  fn sample() -> ReceivedMessage {
    titled("Hello")
  }

  fn titled(subject: &str) -> ReceivedMessage {
    ReceivedMessage {
      sender: "alice@test.com".to_string(),
      recipients: vec!["bob@test.com".to_string()],
      raw: format!("Subject: {subject}\r\n\r\nbody\r\n").into_bytes(),
    }
  }

  fn sized(size: usize) -> ReceivedMessage {
    let mut message = sample();
    message.raw.resize(size, b'x');
    message
  }

  async fn memory_repo() -> MessageRepository {
    open_repository(IN_MEMORY_DB_URL, true).await.unwrap()
  }

  fn deliveries(
    messages: Vec<ReceivedMessage>,
  ) -> (Vec<Delivery>, Vec<oneshot::Receiver<DeliveryOutcome>>) {
    messages.into_iter().map(Delivery::new).unzip()
  }

  async fn outcomes(verdicts: Vec<oneshot::Receiver<DeliveryOutcome>>) -> Vec<DeliveryOutcome> {
    let mut outcomes = Vec::with_capacity(verdicts.len());
    for verdict in verdicts {
      outcomes.push(verdict.await.unwrap());
    }
    outcomes
  }

  #[tokio::test]
  async fn a_stored_message_lets_the_session_accept_it() {
    let repo = memory_repo().await;

    let (batch, verdicts) = deliveries(vec![sample()]);
    let stored = store_batch(&repo, batch).await;

    assert_eq!(stored.len(), 1, "a healthy store must yield a summary");
    assert_eq!(outcomes(verdicts).await, [DeliveryOutcome::Stored]);
  }

  #[tokio::test]
  async fn a_message_parsed_off_the_runtime_is_stored_whole() {
    let repo = memory_repo().await;
    let message = sized(BLOCKING_PARSE_THRESHOLD_BYTES + 1);
    let expected_size = message.raw.len() as i64;

    let (batch, verdicts) = deliveries(vec![message]);
    let stored = store_batch(&repo, batch).await;

    assert_eq!(outcomes(verdicts).await, [DeliveryOutcome::Stored]);
    assert_eq!(stored[0].size, expected_size);
    assert_eq!(stored[0].subject.as_deref(), Some("Hello"));
  }

  /// A write the store will not take must reach the sender as a refusal.
  ///
  /// Closing the pool is the fast stand-in for the cases that produce this in
  /// the wild — a lock outlasting the retry budget, a full disk, an I/O error.
  /// Before the session waited on this verdict, the message was accepted over
  /// SMTP and then lost with only a log line to show for it.
  #[tokio::test]
  async fn a_refused_message_is_not_accepted_over_smtp() {
    let pool = connect_pool(IN_MEMORY_DB_URL, true).await.unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool.clone());
    pool.close().await;

    let (batch, verdicts) = deliveries(vec![sample(), sample()]);
    let stored = store_batch(&repo, batch).await;

    assert!(
      stored.is_empty(),
      "a refused write must not yield a summary"
    );
    assert_eq!(
      outcomes(verdicts).await,
      [DeliveryOutcome::Rejected, DeliveryOutcome::Rejected]
    );
  }

  /// A `250` has to mean stored, so no session in a batch may hear back
  /// before the transaction holding its message has committed.
  ///
  /// The count runs on a reader connection of a file database, which only
  /// sees committed rows: an answer sent mid-transaction would find fewer
  /// than the whole batch there.
  #[tokio::test]
  async fn a_batch_is_acknowledged_only_once_it_is_committed() {
    const BATCH: usize = 4;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("acks.db").display());
    let repo = open_repository(&url, false).await.unwrap();

    let (batch, mut verdicts) = deliveries((0..BATCH).map(|_| sample()).collect());
    let writer = tokio::spawn({
      let repo = repo.clone();
      async move { store_batch(&repo, batch).await }
    });

    let first = verdicts.remove(0).await.unwrap();
    let visible = repo.count().await.unwrap();

    assert_eq!(first, DeliveryOutcome::Stored);
    assert_eq!(
      visible, BATCH as i64,
      "the first answer went out before its batch committed"
    );
    assert_eq!(writer.await.unwrap().len(), BATCH);
    repo.close().await;
  }

  /// A store whose trigger refuses any message titled `poison`.
  async fn poisoned_repo() -> MessageRepository {
    let pool = connect_pool(IN_MEMORY_DB_URL, true).await.unwrap();
    initialize_database(&pool).await.unwrap();
    sqlx::query(
      "CREATE TRIGGER poison BEFORE INSERT ON messages WHEN NEW.subject = 'poison' BEGIN SELECT RAISE(ABORT, 'poisoned'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    MessageRepository::new(pool)
  }

  /// One message the store refuses must not cost the rest of its batch.
  ///
  /// The trigger stands in for whatever makes a single row unwritable; it
  /// aborts the shared transaction exactly as such a failure would.
  #[tokio::test]
  async fn a_poisoned_message_does_not_fail_its_neighbours() {
    let repo = poisoned_repo().await;

    let (batch, verdicts) = deliveries(vec![titled("before"), titled("poison"), titled("after")]);
    let stored = store_batch(&repo, batch).await;

    assert_eq!(
      outcomes(verdicts).await,
      [
        DeliveryOutcome::Stored,
        DeliveryOutcome::Rejected,
        DeliveryOutcome::Stored
      ]
    );
    let subjects: Vec<_> = stored.iter().map(|s| s.subject.as_deref()).collect();
    assert_eq!(subjects, [Some("before"), Some("after")]);
    assert_eq!(repo.count().await.unwrap(), 2);
  }

  /// A lock outlasting the retry budget fails every message alike, so trying
  /// them one at a time only holds each session longer.
  ///
  /// The sleep stands in for a concurrent writer's duration: it outlasts the
  /// batch's own retries but ends well inside what retrying each of the
  /// messages would take, so any per-message insert would store its message.
  #[tokio::test]
  async fn a_store_wide_failure_rejects_the_group_without_retrying_each_message() {
    const MESSAGES: usize = 6;
    const LOCK_HOLD: std::time::Duration = std::time::Duration::from_millis(1500);
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("locked.db").display());
    let options = rustmail_storage::connect_options(&url)
      .unwrap()
      .busy_timeout(std::time::Duration::ZERO);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .max_connections(2)
      .connect_with(options)
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool.clone());
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();
    let releaser = tokio::spawn(async move {
      tokio::time::sleep(LOCK_HOLD).await;
      sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    });

    let (batch, verdicts) = deliveries((0..MESSAGES).map(|_| sample()).collect());
    let stored = store_batch(&repo, batch).await;
    releaser.await.unwrap();

    assert!(stored.is_empty());
    assert_eq!(
      outcomes(verdicts).await,
      [DeliveryOutcome::Rejected; MESSAGES]
    );
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  /// Past its budget, a fallback would commit messages whose sessions are
  /// about to answer `451`, and each of those is captured again on retry.
  #[tokio::test]
  async fn the_fallback_stops_once_the_batch_has_spent_its_budget() {
    let repo = poisoned_repo().await;

    let (batch, verdicts) = deliveries(vec![titled("before"), titled("poison"), titled("after")]);
    let group = transactions(batch).into_iter().next().unwrap();
    let stored = store_group(&repo, group, Instant::now()).await;

    assert!(stored.is_empty());
    assert_eq!(outcomes(verdicts).await, [DeliveryOutcome::Rejected; 3]);
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  /// The UI prepends each `message:new` as it arrives, so the events have to
  /// follow the order the mailbox lists messages in.
  #[tokio::test]
  async fn message_new_events_follow_insert_order() {
    let repo = memory_repo().await;
    let (ws_tx, mut ws_rx) = broadcast::channel::<WsFrame>(64);
    let state = AppState::new(repo.clone(), ws_tx, None, None);
    let subjects = ["one", "two", "three", "four", "five"];

    let (batch, _verdicts) = deliveries(subjects.iter().map(|s| titled(s)).collect());
    process_batch(&repo, &state, batch).await;

    let mut announced = Vec::new();
    while let Ok(frame) = ws_rx.try_recv() {
      announced.push(frame);
    }
    let oldest_first: Vec<WsFrame> = repo
      .list(50, 0)
      .await
      .unwrap()
      .into_iter()
      .rev()
      .map(|summary| WsFrame::encode(&WsEvent::MessageNew(summary)).unwrap())
      .collect();
    assert_eq!(announced.len(), subjects.len());
    assert_eq!(announced, oldest_first);
  }

  /// A session that stopped waiting has already told the sender to retry.
  ///
  /// Storing the message anyway would capture it a second time once the
  /// retry lands.
  #[tokio::test]
  async fn a_delivery_its_session_gave_up_on_is_not_stored() {
    let repo = memory_repo().await;

    let (batch, mut verdicts) = deliveries(vec![titled("abandoned"), titled("waiting")]);
    drop(verdicts.remove(0));
    let stored = store_batch(&repo, batch).await;

    let subjects: Vec<_> = stored.iter().map(|s| s.subject.as_deref()).collect();
    assert_eq!(subjects, [Some("waiting")]);
    assert_eq!(repo.count().await.unwrap(), 1);
    assert_eq!(outcomes(verdicts).await, [DeliveryOutcome::Stored]);
  }

  #[test]
  fn large_mail_commits_one_message_per_transaction() {
    let (batch, _verdicts) = deliveries(vec![
      sized(MAX_BATCH_BYTES),
      sized(MAX_BATCH_BYTES / 2),
      sized(MAX_BATCH_BYTES / 2),
      sized(MAX_BATCH_BYTES / 2),
    ]);

    let sizes: Vec<usize> = transactions(batch)
      .into_iter()
      .map(|group| group.len())
      .collect();

    assert_eq!(sizes, [1, 2, 1]);
  }

  #[test]
  fn small_mail_shares_one_transaction() {
    let (batch, _verdicts) = deliveries((0..MAX_BATCH_MESSAGES).map(|_| sample()).collect());

    let groups = transactions(batch);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), MAX_BATCH_MESSAGES);
  }

  #[tokio::test]
  async fn a_batch_takes_at_most_its_cap_of_queued_deliveries() {
    let (tx, mut rx) = mpsc::channel::<Delivery>(MAX_BATCH_MESSAGES * 2);
    for _ in 0..=MAX_BATCH_MESSAGES {
      tx.send(Delivery::new(sample()).0).await.unwrap();
    }
    let mut batch = Vec::new();

    assert!(next_batch(&mut rx, &mut None, &mut batch).await);
    assert_eq!(batch.len(), MAX_BATCH_MESSAGES);
    batch.clear();
    assert!(next_batch(&mut rx, &mut None, &mut batch).await);
    assert_eq!(batch.len(), 1);
  }

  #[tokio::test]
  async fn stopping_drains_queued_deliveries_then_refuses_new_ones() {
    let (tx, mut rx) = mpsc::channel::<Delivery>(4);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let mut stop = Some(stop_rx);
    for _ in 0..2 {
      tx.send(Delivery::new(sample()).0).await.unwrap();
    }

    stop_tx.send(()).unwrap();

    let mut batch = Vec::new();
    let mut drained = 0;
    while next_batch(&mut rx, &mut stop, &mut batch).await {
      drained += batch.len();
      batch.clear();
    }
    assert_eq!(
      drained, 2,
      "deliveries queued before the stop must be stored"
    );
    assert!(
      tx.send(Delivery::new(sample()).0).await.is_err(),
      "a session handing over after the stop must be told to retry"
    );
  }
}

#[cfg(test)]
mod allowed_origin_tests {
  use super::*;

  #[test]
  fn reads_every_configured_origin() {
    let origins = parse_allowed_origins(&[
      "https://mail.example.com".to_string(),
      "http://ui.test:3000".to_string(),
    ])
    .unwrap();

    let spelled: Vec<String> = origins.iter().map(Origin::to_string).collect();
    assert_eq!(spelled, ["https://mail.example.com", "http://ui.test:3000"]);
  }

  #[test]
  fn an_empty_list_configures_no_origin() {
    assert!(
      parse_allowed_origins(&[String::new()]).unwrap().is_empty(),
      "an empty TOML list and an unset env var must not refuse startup"
    );
  }

  #[test]
  fn reads_every_configured_host() {
    let hosts = parse_allowed_hosts(&["Mail.Example.com".to_string(), String::new()]).unwrap();

    let spelled: Vec<String> = hosts.iter().map(Hostname::to_string).collect();
    assert_eq!(spelled, ["mail.example.com"]);
  }

  #[test]
  fn a_bad_host_refuses_startup() {
    let error = parse_allowed_hosts(&["https://mail.example.com".to_string()]).unwrap_err();

    assert!(
      format!("{error:#}").contains("bare name"),
      "the failure has to say what to write instead: {error:#}"
    );
  }

  #[test]
  fn a_bad_origin_refuses_startup() {
    let error = parse_allowed_origins(&["mail.example.com".to_string()]).unwrap_err();

    assert!(
      format!("{error:#}").contains("must start with http:// or https://"),
      "the failure has to say what to write instead: {error:#}"
    );
  }
}

#[cfg(test)]
mod pool_tests {
  use super::*;

  #[tokio::test]
  async fn ephemeral_pool_never_drops_its_only_connection() {
    let pool = connect_pool(IN_MEMORY_DB_URL, true).await.unwrap();

    let options = pool.options();
    assert_eq!(
      options.get_min_connections(),
      1,
      "an in-memory database is destroyed once its last connection closes"
    );
    assert_eq!(options.get_idle_timeout(), None);
    assert_eq!(options.get_max_lifetime(), None);
  }

  #[tokio::test]
  async fn ephemeral_pool_retains_stored_messages() {
    let pool = connect_pool(IN_MEMORY_DB_URL, true).await.unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool.clone());

    repo
      .insert(
        "a@test.com",
        &["b@test.com".into()],
        b"From: a@test.com\r\nSubject: kept\r\n\r\nbody",
      )
      .await
      .unwrap();

    assert_eq!(repo.count().await.unwrap(), 1);
    assert!(pool.size() >= 1);
  }

  const CONCURRENT_WORKERS: usize = 16;
  const CONCURRENCY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

  #[tokio::test(flavor = "multi_thread")]
  async fn single_connection_pool_serializes_without_deadlocking() {
    let repo = open_repository(IN_MEMORY_DB_URL, true).await.unwrap();

    exercise_concurrently(&repo).await;
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn file_repository_serializes_writes_without_deadlocking() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("split.db").display());
    let repo = open_repository(&url, false).await.unwrap();

    exercise_concurrently(&repo).await;
    repo.close().await;
  }

  /// Runs inserts, counts, searches and lists from many tasks at once.
  ///
  /// Ephemeral mode funnels SMTP inserts, retention and every HTTP request
  /// through one connection, and a file database funnels every write through
  /// one. A repository method that acquired a second connection while holding
  /// one would deadlock here rather than hang the whole server in production.
  async fn exercise_concurrently(repo: &MessageRepository) {
    let mut handles = Vec::new();
    for i in 0..CONCURRENT_WORKERS {
      let repo = repo.clone();
      handles.push(tokio::spawn(async move {
        repo
          .insert(
            "a@test.com",
            &["b@test.com".into()],
            format!("From: a@test.com\r\nSubject: worker{i}\r\n\r\nbody").as_bytes(),
          )
          .await
          .unwrap();
        repo.count().await.unwrap();
        repo.search(&format!("worker{i}"), 10, 0).await.unwrap();
        repo.list(10, 0).await.unwrap();
      }));
    }

    tokio::time::timeout(CONCURRENCY_DEADLINE, async {
      for handle in handles {
        handle.await.unwrap();
      }
    })
    .await
    .expect("a single write connection deadlocked");

    assert_eq!(repo.count().await.unwrap(), CONCURRENT_WORKERS as i64);
  }

  #[tokio::test]
  async fn file_pool_allows_concurrent_connections() {
    let dir = tempfile::tempdir().unwrap();

    let pool = connect_pool(
      &format!("sqlite:{}?mode=rwc", dir.path().join("pool.db").display()),
      false,
    )
    .await
    .unwrap();

    assert_eq!(
      pool.options().get_max_connections(),
      FILE_DB_READER_CONNECTIONS
    );

    pool.close().await;
  }

  #[tokio::test]
  async fn file_writer_is_one_connection_kept_for_the_process_lifetime() {
    let dir = tempfile::tempdir().unwrap();

    let writer = connect_writer(&format!(
      "sqlite:{}?mode=rwc",
      dir.path().join("writer.db").display()
    ))
    .await
    .unwrap();

    let options = writer.options();
    assert_eq!(options.get_max_connections(), 1);
    assert_eq!(options.get_min_connections(), 1);
    assert_eq!(options.get_idle_timeout(), None);
    assert_eq!(options.get_max_lifetime(), None);
    writer.close().await;
  }
}

#[cfg(test)]
mod retention_tests {
  use super::*;
  use rustmail_storage::{MessageRepository, initialize_database};
  use sqlx::sqlite::SqlitePoolOptions;
  use tokio::sync::broadcast;

  async fn build_state() -> (MessageRepository, AppState, broadcast::Receiver<WsFrame>) {
    let pool = SqlitePoolOptions::new()
      .max_connections(1)
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool);
    let (ws_tx, ws_rx) = broadcast::channel::<WsFrame>(64);
    let state = AppState::new(repo.clone(), ws_tx, None, None);
    (repo, state, ws_rx)
  }

  fn raw_email(subject: &str) -> Vec<u8> {
    format!(
      "From: a@test.com\r\nTo: b@test.com\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\nbody"
    )
    .into_bytes()
  }

  async fn insert_sample(repo: &MessageRepository, subject: &str) -> String {
    repo
      .insert("a@test.com", &["b@test.com".into()], &raw_email(subject))
      .await
      .unwrap()
      .id
  }

  fn drain_delete_events(rx: &mut broadcast::Receiver<WsFrame>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(frame) = rx.try_recv() {
      match frame.decode().unwrap() {
        WsEvent::MessageDelete { id } => ids.push(id),
        other => panic!("unexpected WebSocket event from retention tick: {other:?}"),
      }
    }
    ids
  }

  #[tokio::test]
  async fn tick_is_noop_when_retention_and_max_disabled() {
    let (repo, state, mut rx) = build_state().await;
    let _id = insert_sample(&repo, "keep-me").await;

    run_retention_tick(&repo, &state, 0, 0, OffsetDateTime::now_utc()).await;

    assert_eq!(repo.count().await.unwrap(), 1);
    assert!(drain_delete_events(&mut rx).is_empty());
  }

  #[tokio::test]
  async fn tick_purges_messages_older_than_cutoff() {
    let (repo, state, mut rx) = build_state().await;
    let old_id = insert_sample(&repo, "old").await;

    // Advance "now" 24h into the future with a 1h retention window so the
    // stored row is older than the cutoff and gets deleted.
    let future_now = OffsetDateTime::now_utc() + time::Duration::hours(24);
    run_retention_tick(&repo, &state, 1, 0, future_now).await;

    assert_eq!(repo.count().await.unwrap(), 0);
    let events = drain_delete_events(&mut rx);
    assert_eq!(events, vec![old_id]);
  }

  #[tokio::test]
  async fn tick_preserves_rows_newer_than_cutoff() {
    let (repo, state, mut rx) = build_state().await;
    let fresh_id = insert_sample(&repo, "fresh").await;

    // Retention window far larger than any elapsed time → nothing to purge.
    run_retention_tick(&repo, &state, 24, 0, OffsetDateTime::now_utc()).await;

    assert_eq!(repo.count().await.unwrap(), 1);
    assert_eq!(repo.get(&fresh_id).await.unwrap().id, fresh_id);
    assert!(drain_delete_events(&mut rx).is_empty());
  }

  #[tokio::test]
  async fn tick_trims_to_max_messages() {
    let (repo, state, mut rx) = build_state().await;

    let mut ids = Vec::new();
    for i in 0..5 {
      ids.push(insert_sample(&repo, &format!("msg-{i}")).await);
    }

    // Storage trims by arrival order, so the last inserted rows survive.
    // These are inserted within the same millisecond, where ULID order and
    // arrival order genuinely differ.
    let expected_survivors: Vec<String> = ids.iter().rev().take(2).cloned().collect();
    let expected_deleted: Vec<String> = ids.iter().rev().skip(2).cloned().collect();

    run_retention_tick(&repo, &state, 0, 2, OffsetDateTime::now_utc()).await;

    assert_eq!(repo.count().await.unwrap(), 2);
    let events = drain_delete_events(&mut rx);
    let mut got = events.clone();
    got.sort();
    let mut want = expected_deleted.clone();
    want.sort();
    assert_eq!(got, want, "emitted delete events must match deleted ids");

    for surviving in &expected_survivors {
      assert_eq!(repo.get(surviving).await.unwrap().id, *surviving);
    }
  }

  #[tokio::test]
  async fn tick_emits_events_when_both_policies_enabled() {
    let (repo, state, mut rx) = build_state().await;

    let mut ids = Vec::new();
    for i in 0..4 {
      ids.push(insert_sample(&repo, &format!("m-{i}")).await);
    }

    // Cutoff keeps all four rows; trim drops the two that arrived first, so
    // exactly two delete events should fire and match those ids.
    run_retention_tick(&repo, &state, 24, 2, OffsetDateTime::now_utc()).await;

    assert_eq!(repo.count().await.unwrap(), 2);
    let mut got = drain_delete_events(&mut rx);
    got.sort();

    let mut want: Vec<String> = ids.iter().rev().skip(2).cloned().collect();
    want.sort();
    assert_eq!(got, want);
  }
}
