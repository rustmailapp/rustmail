use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

use rustmail_api::{AppState, WsEvent};
use rustmail_smtp::{ReceivedMessage, SmtpServer, SmtpServerConfig};
use rustmail_storage::{MessageRepository, format_iso8601, initialize_database};

#[derive(Parser)]
#[command(name = "rustmail", about = "A modern SMTP mail catcher")]
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
  retention: Option<u64>,
  max_messages: Option<i64>,
  log_level: Option<String>,
  webhook_url: Option<String>,
  release_host: Option<String>,
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

  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .max_connections(2)
    .connect("sqlite::memory:")
    .await?;
  initialize_database(&pool).await?;

  let repo = MessageRepository::new(pool);
  let (smtp_tx, mut smtp_rx) = mpsc::channel::<ReceivedMessage>(256);

  let smtp_config = SmtpServerConfig {
    host: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    port: args.smtp_port,
    max_message_size: args.max_message_size,
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
      while let Some(received) = smtp_rx.recv().await {
        match repo
          .insert(&received.sender, &received.recipients, &received.raw)
          .await
        {
          Ok(_) => {
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
          Err(e) => tracing::error!(error = %e, "Failed to store message"),
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

async fn run_serve(args: ServeArgs) -> Result<()> {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| args.log_level.clone().into()),
    )
    .init();

  let bind_addr = parse_bind_addr(&args.bind)?;

  if !bind_addr.is_loopback() {
    tracing::warn!(
      bind = %args.bind,
      "Binding to non-loopback address with no authentication. All API endpoints are accessible to the network."
    );
  }

  let db_url = if args.ephemeral {
    info!("Running in ephemeral mode (in-memory database)");
    "sqlite::memory:".to_string()
  } else {
    let db_path = args.db_path.unwrap_or_else(default_db_path);
    if let Some(parent) = db_path.parent() {
      std::fs::create_dir_all(parent)?;
    }
    info!(path = %db_path.display(), "Using persistent database");
    format!("sqlite:{}?mode=rwc", db_path.display())
  };

  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .max_connections(5)
    .connect(&db_url)
    .await?;

  initialize_database(&pool).await?;

  let (release_host, release_port) = args.release_host.as_deref().map(parse_release_host).unzip();
  let release_host: Option<String> = release_host;
  let release_port: Option<u16> = release_port.flatten();

  let repo = MessageRepository::new(pool);
  let (smtp_tx, mut smtp_rx) = mpsc::channel::<ReceivedMessage>(256);
  let (ws_tx, _) = broadcast::channel::<WsEvent>(256);

  let state = AppState::new(repo.clone(), ws_tx, release_host, release_port);

  let smtp_config = SmtpServerConfig {
    host: bind_addr,
    port: args.smtp_port,
    max_message_size: args.max_message_size,
  };
  let smtp_server = SmtpServer::new(smtp_config, smtp_tx);

  if let Some(ref url) = args.webhook_url {
    validate_webhook_url(url)?;
  }

  let webhook_client = args.webhook_url.as_ref().map(|_| reqwest::Client::new());
  let webhook_url = args.webhook_url.clone();
  let webhook_semaphore = Arc::new(tokio::sync::Semaphore::new(10));

  let message_processor = {
    let repo = repo.clone();
    let state = state.clone();
    tokio::spawn(async move {
      while let Some(received) = smtp_rx.recv().await {
        match repo
          .insert(&received.sender, &received.recipients, &received.raw)
          .await
        {
          Ok(summary) => {
            state.broadcast(WsEvent::MessageNew(summary.clone()));

            if let (Some(client), Some(url)) = (&webhook_client, &webhook_url) {
              let client = client.clone();
              let url = url.clone();
              let payload = summary;
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
          Err(e) => {
            tracing::error!(error = %e, "Failed to store message");
          }
        }
      }
    })
  };

  let retention_task = {
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
        if retention_hours > 0 {
          let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(retention_hours as i64);
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

  let app = rustmail_api::router(state);

  tokio::select! {
      result = smtp_server.run() => {
          if let Err(e) = result {
              tracing::error!(error = %e, "SMTP server error");
          }
      }
      result = axum::serve(listener, app) => {
          if let Err(e) = result {
              tracing::error!(error = %e, "HTTP server error");
          }
      }
      _ = message_processor => {
          tracing::error!("Message processor stopped unexpectedly");
      }
      _ = retention_task => {}
  }

  Ok(())
}
