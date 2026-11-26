use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::state::{AppState, DEFAULT_RELEASE_PORT, ReleaseTarget, ServerInfo, WsEvent};
use rustmail_storage::{MessageFilter, PageStart, Release, ReleaseOutcome, StorageError};

/// Captured mail is immutable once stored, so anything derived from a
/// message's bytes can be cached indefinitely. `private` keeps shared caches
/// out of captured mail when rustmail is bound to a non-loopback address.
///
/// A deleted message can therefore still be served from a client's cache if
/// its URL is requested again. ULIDs are never reused, so this can only ever
/// resurface the message that URL already named, and the UI drops the row on
/// `message:delete` rather than re-requesting it.
const IMMUTABLE_MESSAGE_CACHE: &str = "private, max-age=31536000, immutable";

/// Read state, stars and tags change over a message's life, so these
/// responses must not be served from a cache.
const MUTABLE_MESSAGE_CACHE: &str = "no-store";

/// Page size when a list request names none.
const DEFAULT_PAGE_LIMIT: i64 = 50;
/// Largest page a list request is served, whatever it asks for.
const MAX_PAGE_LIMIT: i64 = 200;

/// A list request's query string, parsed.
///
/// Read from raw pairs rather than derived, because `tag` may repeat.
/// Unknown keys are ignored, as they always were.
#[derive(Default)]
struct ListParams {
  q: Option<String>,
  limit: Option<i64>,
  offset: Option<i64>,
  before: Option<String>,
  filter: MessageFilter,
}

impl ListParams {
  fn parse(pairs: Vec<(String, String)>) -> Result<Self, AppError> {
    let mut params = Self::default();
    for (key, value) in pairs {
      match key.as_str() {
        "q" => params.q = Some(value),
        "limit" => params.limit = Some(parse_integer(&key, &value)?),
        "offset" => params.offset = Some(parse_integer(&key, &value)?),
        "before" => params.before = Some(value),
        "starred" => params.filter.starred = parse_flag(&key, &value)?,
        "unread" => params.filter.unread = parse_flag(&key, &value)?,
        "has_attachments" => params.filter.has_attachments = parse_flag(&key, &value)?,
        "tag" => params.filter.tags.push(value),
        _ => {}
      }
    }
    if params.filter.tags.len() > MAX_TAGS {
      return Err(AppError::BadRequest(format!(
        "Too many tag filters (max {MAX_TAGS})"
      )));
    }
    Ok(params)
  }
}

fn parse_integer(key: &str, value: &str) -> Result<i64, AppError> {
  value
    .parse()
    .map_err(|_| AppError::BadRequest(format!("{key} must be an integer")))
}

fn parse_flag(key: &str, value: &str) -> Result<bool, AppError> {
  value
    .parse()
    .map_err(|_| AppError::BadRequest(format!("{key} must be true or false")))
}

#[derive(Deserialize)]
pub struct UpdateBody {
  pub is_read: Option<bool>,
  pub is_starred: Option<bool>,
  pub tags: Option<Vec<String>>,
}

pub async fn list_messages(
  State(state): State<AppState>,
  Query(pairs): Query<Vec<(String, String)>>,
) -> Result<impl IntoResponse, AppError> {
  let params = ListParams::parse(pairs)?;
  let limit = params
    .limit
    .unwrap_or(DEFAULT_PAGE_LIMIT)
    .clamp(1, MAX_PAGE_LIMIT);
  let start = match (params.before.as_deref(), params.offset) {
    (Some(_), Some(_)) => return Err(AppError::BadRequest(CURSOR_WITH_OFFSET.to_string())),
    (Some(before), None) => PageStart::Before(state.repo.cursor(before).await.map_err(
      |error| match error {
        StorageError::NotFound(_) => AppError::UnknownCursor,
        other => AppError::Storage(other),
      },
    )?),
    (None, offset) => PageStart::Offset(offset.unwrap_or(0).max(0)),
  };
  let lookahead = limit + 1;

  let (mut messages, count) = if let Some(query) = &params.q {
    let msgs = state
      .repo
      .search_page(query, &params.filter, start, lookahead)
      .await?;
    let total = state
      .repo
      .search_count_filtered(query, &params.filter)
      .await?;
    (msgs, total)
  } else {
    let msgs = state
      .repo
      .list_page(&params.filter, start, lookahead)
      .await?;
    let total = state.repo.count_filtered(&params.filter).await?;
    (msgs, total)
  };
  let has_more = messages.len() as i64 > limit;
  messages.truncate(limit as usize);
  let next_cursor = has_more
    .then(|| messages.last().map(|m| m.id.clone()))
    .flatten();

  Ok((
    StatusCode::OK,
    [(header::CACHE_CONTROL, MUTABLE_MESSAGE_CACHE.to_string())],
    Json(serde_json::json!({
        "messages": messages,
        "total": count,
        "next_cursor": next_cursor,
        "limit": limit,
    })),
  ))
}

pub async fn get_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let message = state.repo.get(&id).await?;
  Ok((
    StatusCode::OK,
    [(header::CACHE_CONTROL, MUTABLE_MESSAGE_CACHE.to_string())],
    Json(message),
  ))
}

const MAX_TAGS: usize = 20;
const MAX_TAG_LEN: usize = 50;

fn validate_tags(tags: &[String]) -> Result<(), AppError> {
  if tags.len() > MAX_TAGS {
    return Err(AppError::BadRequest(format!(
      "Too many tags (max {MAX_TAGS})"
    )));
  }
  for tag in tags {
    if tag.len() > MAX_TAG_LEN {
      return Err(AppError::BadRequest(format!(
        "Tag too long (max {MAX_TAG_LEN} chars)"
      )));
    }
    if tag.is_empty() || tag.chars().any(|c| c.is_control()) {
      return Err(AppError::BadRequest(
        "Tags must be non-empty and contain no control characters".to_string(),
      ));
    }
  }
  Ok(())
}

pub async fn update_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
  Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, AppError> {
  if let Some(ref tags) = body.tags {
    validate_tags(tags)?;
  }

  state
    .repo
    .update_message(&id, body.is_read, body.is_starred, body.tags.as_deref())
    .await?;

  if let Some(is_read) = body.is_read {
    state.broadcast(WsEvent::MessageRead {
      id: id.clone(),
      is_read,
    });
  }
  if let Some(is_starred) = body.is_starred {
    state.broadcast(WsEvent::MessageStarred {
      id: id.clone(),
      is_starred,
    });
  }
  if let Some(tags) = body.tags {
    state.broadcast(WsEvent::MessageTags { id, tags });
  }
  Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn delete_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  state.repo.delete(&id).await?;
  state.broadcast(WsEvent::MessageDelete { id });
  Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_all_messages(
  State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
  let count = state.repo.delete_all().await?;
  state.broadcast(WsEvent::MessagesClear);
  state.repo.reclaim_after_delete_all().await;
  Ok(Json(serde_json::json!({ "deleted": count })))
}

pub async fn list_attachments(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let attachments = state.repo.get_attachments(&id).await?;
  Ok((
    StatusCode::OK,
    [(header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string())],
    Json(attachments),
  ))
}

pub async fn get_attachment(
  State(state): State<AppState>,
  Path((message_id, attachment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
  let attachment = state
    .repo
    .get_attachment(&message_id, &attachment_id)
    .await?;

  let filename = sanitize_filename(
    &attachment
      .filename
      .unwrap_or_else(|| "attachment".to_string()),
  );

  Ok((
    StatusCode::OK,
    [
      (
        header::CONTENT_TYPE,
        crate::DOWNLOAD_CONTENT_TYPE.to_string(),
      ),
      (
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", filename),
      ),
      (header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string()),
      (
        header::HeaderName::from_static("x-content-type-options"),
        "nosniff".to_string(),
      ),
    ],
    attachment.content,
  ))
}

pub async fn get_inline_attachment(
  State(state): State<AppState>,
  Path((message_id, content_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
  let attachment = state
    .repo
    .get_attachment_by_content_id(&message_id, &content_id)
    .await?;

  const SAFE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
  ];
  let content_type = attachment
    .content_type
    .filter(|ct| {
      SAFE_IMAGE_TYPES
        .iter()
        .any(|safe| ct.eq_ignore_ascii_case(safe))
    })
    .unwrap_or_else(|| "application/octet-stream".to_string());

  Ok((
    StatusCode::OK,
    [
      (header::CONTENT_TYPE, content_type),
      (header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string()),
      (
        header::HeaderName::from_static("x-content-type-options"),
        "nosniff".to_string(),
      ),
      (
        header::HeaderName::from_static("content-security-policy"),
        "default-src 'none'".to_string(),
      ),
    ],
    attachment.content,
  ))
}

/// `limit` is taken as text so that a malformed value is rejected by this
/// handler, in the same JSON shape as an out-of-range one, rather than by the
/// query extractor in a different shape.
#[derive(Deserialize)]
pub struct RawParams {
  pub limit: Option<String>,
}

const RAW_LIMIT_ERROR: &str = "limit must be a positive number of bytes";

pub async fn get_raw_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
  Query(params): Query<RawParams>,
) -> Result<impl IntoResponse, AppError> {
  let limit = match params.limit.as_deref() {
    Some(text) => match text.parse::<i64>() {
      Ok(limit) if limit > 0 => Some(limit),
      _ => return Err(AppError::BadRequest(RAW_LIMIT_ERROR.to_string())),
    },
    None => None,
  };

  let raw = match limit {
    Some(limit) => state.repo.get_raw_prefix(&id, limit).await?,
    None => state.repo.get_raw(&id).await?,
  };

  Ok(
    (
      StatusCode::OK,
      [
        (header::CONTENT_TYPE, "message/rfc822".to_string()),
        (header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string()),
      ],
      raw,
    )
      .into_response(),
  )
}

/// One header field as it appears on the wire, with folded lines joined.
#[derive(Debug, Serialize)]
pub struct MessageHeader {
  pub name: String,
  pub value: String,
}

/// Raw bytes read when only the header section is needed.
const HEADER_SECTION_PREFIX_BYTES: i64 = 64 * 1024;

fn ends_header_section(raw: &[u8]) -> bool {
  raw.windows(4).any(|w| w == b"\r\n\r\n") || raw.windows(2).any(|w| w == b"\n\n")
}

/// Reads enough of a message's source to cover its header section.
///
/// Real headers fit in [`HEADER_SECTION_PREFIX_BYTES`] many times over, so the
/// common case copies and parses a short prefix instead of the whole
/// multi-megabyte source. SQLite still loads the whole blob to cut that
/// prefix, so the saving is the copy and the parse, not the database read.
/// A message whose header section is genuinely longer falls back to the full
/// source rather than silently losing fields.
async fn read_header_section(state: &AppState, id: &str) -> Result<Vec<u8>, StorageError> {
  let prefix = state
    .repo
    .get_raw_prefix(id, HEADER_SECTION_PREFIX_BYTES)
    .await?;

  let complete =
    (prefix.len() as i64) < HEADER_SECTION_PREFIX_BYTES || ends_header_section(&prefix);
  if complete {
    return Ok(prefix);
  }

  state.repo.get_raw(id).await
}

/// Reads message `id`'s header section and hands `read` its top-level
/// headers along with the raw bytes their offsets index into.
async fn read_root_headers<R>(
  state: &AppState,
  id: &str,
  read: impl FnOnce(&[u8], &[mail_parser::Header<'_>]) -> R,
) -> Result<R, AppError> {
  let raw = read_header_section(state, id).await?;
  let parsed = mail_parser::MessageParser::default().parse_headers(&raw);
  let headers = parsed
    .as_ref()
    .and_then(|msg| msg.parts.first())
    .map(|root| root.headers.as_slice())
    .unwrap_or_default();
  Ok(read(&raw, headers))
}

pub async fn get_headers(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let headers: Vec<MessageHeader> = read_root_headers(&state, &id, |raw, headers| {
    headers
      .iter()
      .map(|h| MessageHeader {
        name: h.name.as_str().to_string(),
        value: unfold_header_value(
          raw
            .get(h.offset_start as usize..h.offset_end as usize)
            .unwrap_or_default(),
        ),
      })
      .collect()
  })
  .await?;

  Ok((
    StatusCode::OK,
    [(header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string())],
    Json(headers),
  ))
}

fn unfold_header_value(value: &[u8]) -> String {
  String::from_utf8_lossy(value)
    .split(['\r', '\n'])
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

#[derive(Deserialize)]
pub struct AssertParams {
  pub min: Option<i64>,
  pub max: Option<i64>,
  pub subject: Option<String>,
  pub sender: Option<String>,
  pub recipient: Option<String>,
}

pub async fn assert_count(
  State(state): State<AppState>,
  Query(params): Query<AssertParams>,
) -> Result<impl IntoResponse, AppError> {
  let count = state
    .repo
    .count_matching(
      params.subject.as_deref(),
      params.sender.as_deref(),
      params.recipient.as_deref(),
    )
    .await?;

  let min = params.min.unwrap_or(1);
  let max = params.max.unwrap_or(i64::MAX);

  if count >= min && count <= max {
    Ok(
      (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "count": count })),
      )
        .into_response(),
    )
  } else {
    Ok(
      (
        StatusCode::EXPECTATION_FAILED,
        Json(serde_json::json!({
            "ok": false,
            "count": count,
            "expected_min": min,
            "expected_max": max,
        })),
      )
        .into_response(),
    )
  }
}

/// What `GET /api/v1/info` answers: the [`ServerInfo`] plus the release relay.
#[derive(Serialize)]
struct InfoResponse<'a> {
  #[serde(flatten)]
  info: &'a ServerInfo,
  release: Option<ReleaseTarget>,
}

/// Reports the server's version and SMTP port, as set by
/// [`AppState::with_server_info`], and the relay release forwards to.
pub async fn get_info(State(state): State<AppState>) -> axum::response::Response {
  match &state.server_info {
    Some(info) => Json(InfoResponse {
      info,
      release: state.release_target(),
    })
    .into_response(),
    None => AppError::NotFound("Server info is not configured".to_string()).into_response(),
  }
}

#[derive(Deserialize)]
pub struct ExportParams {
  pub format: Option<String>,
}

pub async fn export_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
  Query(params): Query<ExportParams>,
) -> Result<impl IntoResponse, AppError> {
  let format = params.format.as_deref().unwrap_or("eml");

  match format {
    "eml" => {
      let raw = state.repo.get_raw(&id).await?;
      Ok(
        (
          StatusCode::OK,
          [
            (header::CONTENT_TYPE, "message/rfc822".to_string()),
            (
              header::CONTENT_DISPOSITION,
              format!("attachment; filename=\"{}.eml\"", sanitize_filename(&id)),
            ),
            (header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string()),
          ],
          raw,
        )
          .into_response(),
      )
    }
    "json" => {
      let msg = state.repo.get(&id).await?;
      match serde_json::to_vec(&msg) {
        Ok(body) => Ok(
          (
            StatusCode::OK,
            [
              (header::CONTENT_TYPE, "application/json".to_string()),
              (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}.json\"", sanitize_filename(&id)),
              ),
              (header::CACHE_CONTROL, MUTABLE_MESSAGE_CACHE.to_string()),
            ],
            body,
          )
            .into_response(),
        ),
        Err(e) => {
          tracing::error!(error = %e, message_id = %id, "Failed to serialize message for JSON export");
          Err(AppError::Internal)
        }
      }
    }
    _ => Err(AppError::BadRequest(
      "format must be 'eml' or 'json'".to_string(),
    )),
  }
}

#[derive(Deserialize)]
pub struct ReleaseBody {
  pub host: String,
  pub port: Option<u16>,
}

const ALLOWED_SMTP_PORTS: &[u16] = &[25, 465, 587, 2525];

/// The SMTPS port: the relay expects TLS from the connection's first byte.
const IMPLICIT_TLS_SMTP_PORT: u16 = 465;

/// How the connection to the release relay is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelaySecurity {
  /// TLS from the first byte (SMTPS).
  ImplicitTls,
  /// Plaintext greeting and EHLO, then a mandatory STARTTLS upgrade; the
  /// message is never sent if the relay cannot upgrade.
  RequiredStartTls,
}

impl RelaySecurity {
  fn for_port(port: u16) -> Self {
    if port == IMPLICIT_TLS_SMTP_PORT {
      Self::ImplicitTls
    } else {
      Self::RequiredStartTls
    }
  }
}

/// Longest a release may spend on the relay, from connecting to its last reply.
///
/// lettre's own timeout bounds only the TCP connect, so a relay that stalls
/// after it would hold the request open indefinitely. This stays under the
/// UI's release deadline so the server answers before the browser gives up.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);
const RELEASE_OUTCOME_UNKNOWN: &str = "The relay did not confirm the message. It may have been delivered: check the relay before releasing it again.";
const RELEASE_OUTCOME_UNKNOWN_CODE: &str = "release_outcome_unknown";

fn relay_transport(
  host: &str,
  port: u16,
  security: RelaySecurity,
  tls_parameters: lettre::transport::smtp::client::TlsParameters,
) -> Result<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>, lettre::transport::smtp::Error> {
  use lettre::transport::smtp::client::Tls;
  use lettre::{AsyncSmtpTransport, Tokio1Executor};

  let (builder, tls) = match security {
    RelaySecurity::ImplicitTls => (
      AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
      Tls::Wrapper(tls_parameters),
    ),
    RelaySecurity::RequiredStartTls => (
      AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
      Tls::Required(tls_parameters),
    ),
  };
  Ok(builder.port(port).tls(tls).build())
}

/// How a release attempt against the relay ended.
#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
enum RelayOutcome {
  /// The relay accepted the message.
  Sent,
  /// The relay never took the message: it could not be reached, failed the
  /// handshake, or answered with a refusal.
  NotSent(String),
  /// The exchange broke off or ran out of time once the message may already
  /// have been handed over; the relay may have it.
  Unknown(String),
}

/// Whether `error` proves the relay did not take the message: an SMTP
/// refusal, or a client or TLS failure that stops the send before any data.
fn refused_before_delivery(error: &lettre::transport::smtp::Error) -> bool {
  error.is_permanent() || error.is_transient() || error.is_client() || error.is_tls()
}

/// Releases `raw` to the relay within `deadline`.
///
/// The relay is probed first, connect to `QUIT`, because a lettre error does
/// not say how far the exchange got: once data may have been sent, only an
/// SMTP refusal proves the message was not delivered. A relay that cannot
/// even complete the probe never saw the message.
async fn send_to_relay(
  mailer: &lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
  envelope: &lettre::address::Envelope,
  raw: &[u8],
  deadline: Duration,
) -> RelayOutcome {
  use lettre::AsyncTransport;

  let started = tokio::time::Instant::now();
  match tokio::time::timeout(deadline, mailer.test_connection()).await {
    Ok(Ok(true)) => {}
    Ok(Ok(false)) => return RelayOutcome::NotSent("the relay did not answer NOOP".into()),
    Ok(Err(e)) => return RelayOutcome::NotSent(e.to_string()),
    Err(_) => {
      return RelayOutcome::NotSent("the relay did not finish the handshake in time".into());
    }
  }

  let remaining = deadline.saturating_sub(started.elapsed());
  match tokio::time::timeout(remaining, mailer.send_raw(envelope, raw)).await {
    Ok(Ok(_)) => RelayOutcome::Sent,
    Ok(Err(e)) if refused_before_delivery(&e) => RelayOutcome::NotSent(e.to_string()),
    Ok(Err(e)) => RelayOutcome::Unknown(e.to_string()),
    Err(_) => RelayOutcome::Unknown("the relay did not answer in time".into()),
  }
}

pub async fn release_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
  Json(body): Json<ReleaseBody>,
) -> Result<impl IntoResponse, AppError> {
  let (allowed_host, allowed_port) = match (&state.release_host, state.release_port) {
    (Some(host), port) => (host.clone(), port),
    _ => {
      return Err(AppError::Forbidden(
        "Email release is disabled. Configure --release-host to enable.".to_string(),
      ));
    }
  };

  if body.host != allowed_host {
    return Err(AppError::Forbidden(format!(
      "Release only allowed to configured host: {allowed_host}"
    )));
  }

  let port = body
    .port
    .unwrap_or(allowed_port.unwrap_or(DEFAULT_RELEASE_PORT));
  if allowed_port.is_some() && Some(port) != allowed_port {
    return Err(AppError::Forbidden(format!(
      "Release only allowed on configured port: {}",
      allowed_port.unwrap_or(DEFAULT_RELEASE_PORT)
    )));
  }
  if !ALLOWED_SMTP_PORTS.contains(&port) {
    return Err(AppError::BadRequest(format!(
      "Port {port} is not an allowed SMTP port (25, 465, 587, 2525)"
    )));
  }

  let source = state.repo.get_source(&id).await?;

  let envelope = match release_envelope(&source.sender, &source.recipients) {
    Ok(envelope) => envelope,
    Err(reason) => return Err(AppError::BadRequest(format!("Invalid envelope: {reason}"))),
  };

  let mailer = match lettre::transport::smtp::client::TlsParameters::new(body.host.clone())
    .and_then(|tls| relay_transport(&body.host, port, RelaySecurity::for_port(port), tls))
  {
    Ok(m) => m,
    Err(e) => {
      tracing::error!(error = %e, "TLS setup failed for relay host");
      return Err(AppError::BadGateway(
        "Failed to establish TLS connection to relay host".to_string(),
      ));
    }
  };

  let release_state = state.clone();
  let release_id = id.clone();
  let release = tokio::spawn(async move {
    let outcome = send_to_relay(&mailer, &envelope, &source.raw, RELEASE_TIMEOUT).await;
    let recorded = match &outcome {
      RelayOutcome::Sent => record_release(&release_state, &release_id, ReleaseOutcome::Sent).await,
      RelayOutcome::Unknown(_) => {
        record_release(&release_state, &release_id, ReleaseOutcome::Unknown).await
      }
      RelayOutcome::NotSent(_) => None,
    };
    (outcome, recorded)
  });
  let (outcome, recorded) = match release.await {
    Ok(finished) => finished,
    Err(error) => (
      RelayOutcome::Unknown(format!("the release task did not complete: {error}")),
      None,
    ),
  };
  Ok(release_response(&id, outcome, recorded).into_response())
}

/// The answer to a release the relay has finished with, carrying the
/// release recorded on the message, if one was.
fn release_response(
  id: &str,
  outcome: RelayOutcome,
  recorded: Option<Release>,
) -> (StatusCode, Json<serde_json::Value>) {
  match outcome {
    RelayOutcome::Sent => {
      let mut body = serde_json::json!({ "released": true });
      merge_release(&mut body, recorded);
      (StatusCode::OK, Json(body))
    }
    RelayOutcome::NotSent(reason) => {
      tracing::error!(error = %reason, message_id = %id, "SMTP delivery failed");
      (
        StatusCode::BAD_GATEWAY,
        Json(error_body("SMTP delivery failed", None)),
      )
    }
    RelayOutcome::Unknown(reason) => {
      tracing::warn!(error = %reason, message_id = %id, "Release relay did not confirm the message");
      let mut body = error_body(RELEASE_OUTCOME_UNKNOWN, Some(RELEASE_OUTCOME_UNKNOWN_CODE));
      merge_release(&mut body, recorded);
      (StatusCode::GATEWAY_TIMEOUT, Json(body))
    }
  }
}

/// Records a release that may have reached the relay and tells every client.
///
/// The relay has already answered, so a failure to record is logged and the
/// response still reports what the relay did. Runs with the send in its own
/// task, so a client that disconnects mid-release cannot cancel the record.
async fn record_release(state: &AppState, id: &str, outcome: ReleaseOutcome) -> Option<Release> {
  match state.repo.record_release(id, outcome).await {
    Ok(release) => {
      state.broadcast(WsEvent::MessageReleased {
        id: id.to_string(),
        released_at: release.released_at.clone(),
        release_outcome: release.release_outcome,
      });
      Some(release)
    }
    Err(error) => {
      tracing::error!(error = %error, message_id = %id, outcome = outcome.as_str(), "Could not record the release on the message");
      None
    }
  }
}

/// Adds the recorded release, if any, to a release response body.
fn merge_release(body: &mut serde_json::Value, release: Option<Release>) {
  if let (Some(object), Some(release)) = (body.as_object_mut(), release) {
    object.insert("released_at".into(), release.released_at.into());
    object.insert(
      "release_outcome".into(),
      release.release_outcome.as_str().into(),
    );
  }
}

/// The captured envelope a release sends again, exactly as captured.
///
/// Every address has to carry over: dropping one the relay cannot take would
/// deliver to fewer recipients, or from another sender, than the message was
/// sent with, and still report success. Only an empty sender, the null
/// reverse-path `MAIL FROM:<>`, becomes no sender.
fn release_envelope(sender: &str, recipients: &str) -> Result<lettre::address::Envelope, String> {
  let from = if sender.is_empty() {
    None
  } else {
    Some(
      sender
        .parse()
        .map_err(|_| "the captured sender is not an address a relay accepts".to_string())?,
    )
  };
  let to = serde_json::from_str::<Vec<String>>(recipients)
    .map_err(|_| "the captured recipients could not be read".to_string())?
    .iter()
    .map(|recipient| recipient.parse())
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| "a captured recipient is not an address a relay accepts".to_string())?;
  lettre::address::Envelope::new(from, to).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct AuthResults {
  pub dkim: Vec<AuthCheck>,
  pub spf: Vec<AuthCheck>,
  pub dmarc: Vec<AuthCheck>,
  pub arc: Vec<AuthCheck>,
}

#[derive(Debug, Serialize)]
pub struct AuthCheck {
  pub status: String,
  pub details: String,
}

pub async fn get_auth_results(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let results = read_root_headers(&state, &id, |_, headers| auth_results(headers)).await?;
  Ok((
    StatusCode::OK,
    [(header::CACHE_CONTROL, IMMUTABLE_MESSAGE_CACHE.to_string())],
    Json(results),
  ))
}

/// The authentication verdicts a message's top-level headers record.
fn auth_results(headers: &[mail_parser::Header<'_>]) -> AuthResults {
  let mut dkim = Vec::new();
  let mut spf = Vec::new();
  let mut dmarc = Vec::new();
  let mut arc = Vec::new();

  for h in headers {
    let value = match &h.value {
      mail_parser::HeaderValue::Text(t) => t.as_ref(),
      _ => continue,
    };

    match &h.name {
      mail_parser::HeaderName::Other(name)
        if name.eq_ignore_ascii_case("Authentication-Results") =>
      {
        parse_auth_results_header(value, &mut dkim, &mut spf, &mut dmarc);
      }
      mail_parser::HeaderName::ArcAuthenticationResults => {
        let mut arc_dkim = Vec::new();
        let mut arc_spf = Vec::new();
        let mut arc_dmarc = Vec::new();
        parse_auth_results_header(value, &mut arc_dkim, &mut arc_spf, &mut arc_dmarc);
        for mut check in arc_dkim.into_iter().chain(arc_spf).chain(arc_dmarc) {
          check.status = format!("arc:{}", check.status);
          arc.push(check);
        }
      }
      mail_parser::HeaderName::DkimSignature => {
        dkim.push(parse_dkim_signature(value));
      }
      mail_parser::HeaderName::Other(name) if name.eq_ignore_ascii_case("Received-SPF") => {
        spf.push(parse_received_spf(value));
      }
      _ => {}
    }
  }

  AuthResults {
    dkim,
    spf,
    dmarc,
    arc,
  }
}

fn parse_auth_results_header(
  value: &str,
  dkim: &mut Vec<AuthCheck>,
  spf: &mut Vec<AuthCheck>,
  dmarc: &mut Vec<AuthCheck>,
) {
  let checks = match value.split_once(';') {
    Some((_, rest)) => rest,
    None => return,
  };

  for segment in checks.split(';') {
    let segment = segment.trim();
    if segment.is_empty() {
      continue;
    }

    let (method, status, details) = parse_method_result(segment);

    match method.as_str() {
      "dkim" => dkim.push(AuthCheck { status, details }),
      "spf" => spf.push(AuthCheck { status, details }),
      "dmarc" => dmarc.push(AuthCheck { status, details }),
      _ => {}
    }
  }
}

fn parse_method_result(segment: &str) -> (String, String, String) {
  let segment = segment.trim();
  let (method, rest) = segment.split_once('=').unwrap_or(("", segment));
  let method = method.trim().to_string();
  let status = rest
    .split_whitespace()
    .next()
    .unwrap_or("unknown")
    .to_string();
  (method, status, segment.to_string())
}

fn parse_dkim_signature(value: &str) -> AuthCheck {
  let mut domain = "";
  let mut selector = "";
  let mut algorithm = "";

  for part in value.split(';') {
    let part = part.trim();
    if let Some((tag, val)) = part.split_once('=') {
      match tag.trim() {
        "d" => domain = val.trim(),
        "s" => selector = val.trim(),
        "a" => algorithm = val.trim(),
        _ => {}
      }
    }
  }

  AuthCheck {
    status: "info".to_string(),
    details: format!("d={domain} s={selector} a={algorithm}"),
  }
}

fn parse_received_spf(value: &str) -> AuthCheck {
  let status = value
    .split_whitespace()
    .next()
    .unwrap_or("unknown")
    .to_ascii_lowercase();

  AuthCheck {
    status,
    details: value.to_string(),
  }
}

fn sanitize_filename(name: &str) -> String {
  name
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
        c
      } else {
        '_'
      }
    })
    .collect()
}

const CURSOR_WITH_OFFSET: &str =
  "before and offset cannot be combined; page with next_cursor or with offset, not both";
const UNKNOWN_CURSOR: &str =
  "before does not name a stored message; it may have been deleted, so restart from the first page";
/// The `code` an unknown-cursor `400` carries, so a client can tell it from
/// the other reasons a list request is rejected without parsing the prose.
const UNKNOWN_CURSOR_CODE: &str = "unknown_cursor";
const READ_TIMED_OUT: &str =
  "the request ran past the server's time limit; retry it, or narrow it with limit or filters";

/// Every error a handler answers with, rendered as `{"error": ...}` (plus a
/// `code` where a client needs to branch on it) under its status.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
  #[error(transparent)]
  Storage(#[from] StorageError),
  #[error("{0}")]
  BadRequest(String),
  #[error("{0}")]
  Forbidden(String),
  #[error("{0}")]
  NotFound(String),
  #[error("{0}")]
  BadGateway(String),
  #[error("Internal server error")]
  Internal,
  #[error("{UNKNOWN_CURSOR}")]
  UnknownCursor,
  #[error("{READ_TIMED_OUT}")]
  TimedOut,
}

/// The JSON body of an error response.
fn error_body(message: &str, code: Option<&str>) -> serde_json::Value {
  match code {
    Some(code) => serde_json::json!({ "error": message, "code": code }),
    None => serde_json::json!({ "error": message }),
  }
}

/// Status and client-facing message for a storage failure, logging the
/// detail the client does not get.
fn storage_failure(error: &StorageError) -> (StatusCode, String) {
  match error {
    StorageError::NotFound(_) => (StatusCode::NOT_FOUND, "Resource not found".to_string()),
    StorageError::Database(e) => {
      tracing::error!(error = %e, "Database error");
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    StorageError::AttachmentCorrupt {
      message_id,
      attachment_id,
      transfer_encoding,
      expected_size,
      actual_size,
    } => {
      tracing::error!(
        event = "attachment_corrupt",
        message_id = %message_id,
        attachment_id = %attachment_id,
        transfer_encoding = ?transfer_encoding,
        expected_size = ?expected_size,
        actual_size = ?actual_size,
        "Stored attachment failed its integrity check; re-send the message to store it again"
      );
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    StorageError::ContentCorrupt { message_id, reason } => {
      tracing::error!(
        event = "content_corrupt",
        message_id = %message_id,
        reason = %reason,
        "Stored message source failed its integrity check; delete the message and re-send it"
      );
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    e @ StorageError::Compress(_) => {
      tracing::error!(error = %e, "Message compression failed");
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    e @ StorageError::DecodeAborted(_) => {
      tracing::error!(error = %e, "Stored content decode aborted");
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    e @ (StorageError::NewerSchema { .. }
    | StorageError::LegacySchema { .. }
    | StorageError::UnrecognizedSchema { .. }) => {
      tracing::error!(error = %e, "Unsupported database schema");
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
    e @ (StorageError::MigrationLocked { .. }
    | StorageError::MigrationRefused(_)
    | StorageError::RestoreRefused(_)
    | StorageError::MigrationVerifyFailed { .. }
    | StorageError::MigrationDiskFull { .. }
    | StorageError::MigrationIo { .. }
    | StorageError::MigrationTaskAborted(_)) => {
      tracing::error!(error = %e, "Storage migration error");
      (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
      )
    }
  }
}

impl IntoResponse for AppError {
  fn into_response(self) -> axum::response::Response {
    let (status, message) = match &self {
      AppError::Storage(error) => storage_failure(error),
      AppError::BadRequest(_) | AppError::UnknownCursor => {
        (StatusCode::BAD_REQUEST, self.to_string())
      }
      AppError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
      AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
      AppError::BadGateway(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
      AppError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
      AppError::TimedOut => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
    };
    let code = matches!(self, AppError::UnknownCursor).then_some(UNKNOWN_CURSOR_CODE);
    (status, Json(error_body(&message, code))).into_response()
  }
}

#[cfg(test)]
mod app_error_tests {
  use super::*;
  use axum::body::to_bytes;

  async fn rendered(error: AppError) -> (StatusCode, String, Vec<u8>) {
    let response = error.into_response();
    let status = response.status();
    let content_type = response.headers()[header::CONTENT_TYPE]
      .to_str()
      .unwrap()
      .to_string();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, content_type, body.to_vec())
  }

  #[tokio::test]
  async fn each_error_renders_the_status_and_body_it_always_had() {
    let cases = [
      (
        AppError::NotFound("Server info is not configured".to_string()),
        StatusCode::NOT_FOUND,
        serde_json::json!({ "error": "Server info is not configured" }),
      ),
      (
        AppError::NotFound("Unknown API endpoint".to_string()),
        StatusCode::NOT_FOUND,
        serde_json::json!({ "error": "Unknown API endpoint" }),
      ),
      (
        AppError::BadGateway("Failed to establish TLS connection to relay host".to_string()),
        StatusCode::BAD_GATEWAY,
        serde_json::json!({ "error": "Failed to establish TLS connection to relay host" }),
      ),
      (
        AppError::Forbidden(
          "Email release is disabled. Configure --release-host to enable.".to_string(),
        ),
        StatusCode::FORBIDDEN,
        serde_json::json!({ "error": "Email release is disabled. Configure --release-host to enable." }),
      ),
      (
        AppError::Internal,
        StatusCode::INTERNAL_SERVER_ERROR,
        serde_json::json!({ "error": "Internal server error" }),
      ),
      (
        AppError::TimedOut,
        StatusCode::SERVICE_UNAVAILABLE,
        serde_json::json!({ "error": READ_TIMED_OUT }),
      ),
      (
        AppError::UnknownCursor,
        StatusCode::BAD_REQUEST,
        serde_json::json!({ "error": UNKNOWN_CURSOR, "code": UNKNOWN_CURSOR_CODE }),
      ),
      (
        AppError::Storage(StorageError::NotFound(
          "01J0000000000000000000000Z".to_string(),
        )),
        StatusCode::NOT_FOUND,
        serde_json::json!({ "error": "Resource not found" }),
      ),
    ];
    for (error, status, body) in cases {
      let (got_status, content_type, got_body) = rendered(error).await;
      assert_eq!(got_status, status, "{body}");
      assert_eq!(content_type, "application/json", "{body}");
      assert_eq!(got_body, serde_json::to_vec(&body).unwrap(), "{body}");
    }
  }
}

#[cfg(test)]
mod release_envelope_tests {
  use super::release_envelope;

  #[test]
  fn a_captured_envelope_carries_over_whole() {
    let envelope = release_envelope(
      "from@example.test",
      r#"["a@example.test","b@example.test"]"#,
    )
    .unwrap();

    assert_eq!(
      envelope.from().map(ToString::to_string).as_deref(),
      Some("from@example.test")
    );
    assert_eq!(envelope.to().len(), 2);
  }

  #[test]
  fn the_null_sender_releases_without_one() {
    let envelope = release_envelope("", r#"["a@example.test"]"#).unwrap();

    assert!(envelope.from().is_none());
  }

  #[test]
  fn a_recipient_a_relay_cannot_take_refuses_the_release() {
    let refused = release_envelope(
      "from@example.test",
      r#"["a@example.test","not an address"]"#,
    );

    assert_eq!(
      refused.unwrap_err(),
      "a captured recipient is not an address a relay accepts"
    );
  }

  #[test]
  fn a_sender_a_relay_cannot_take_refuses_the_release() {
    let refused = release_envelope("not an address", r#"["a@example.test"]"#);

    assert_eq!(
      refused.unwrap_err(),
      "the captured sender is not an address a relay accepts"
    );
  }
}

#[cfg(test)]
mod auth_parser_tests {
  use super::*;

  #[test]
  fn auth_results_header_missing_semicolon_is_noop() {
    let mut dkim = Vec::new();
    let mut spf = Vec::new();
    let mut dmarc = Vec::new();
    parse_auth_results_header("mx.google.com", &mut dkim, &mut spf, &mut dmarc);
    assert!(dkim.is_empty());
    assert!(spf.is_empty());
    assert!(dmarc.is_empty());
  }

  #[test]
  fn auth_results_header_empty_input_is_noop() {
    let mut dkim = Vec::new();
    let mut spf = Vec::new();
    let mut dmarc = Vec::new();
    parse_auth_results_header("", &mut dkim, &mut spf, &mut dmarc);
    assert!(dkim.is_empty());
    assert!(spf.is_empty());
    assert!(dmarc.is_empty());
  }

  #[test]
  fn auth_results_header_parses_all_three_methods() {
    let value = "mx.example.com; dkim=pass header.d=example.com; spf=pass smtp.mailfrom=a@ex.com; dmarc=pass action=none";
    let mut dkim = Vec::new();
    let mut spf = Vec::new();
    let mut dmarc = Vec::new();
    parse_auth_results_header(value, &mut dkim, &mut spf, &mut dmarc);
    assert_eq!(dkim.len(), 1);
    assert_eq!(spf.len(), 1);
    assert_eq!(dmarc.len(), 1);
    assert_eq!(dkim[0].status, "pass");
    assert_eq!(spf[0].status, "pass");
    assert_eq!(dmarc[0].status, "pass");
  }

  #[test]
  fn auth_results_header_ignores_empty_segments() {
    let value = "mx.example.com; ;; dkim=fail";
    let mut dkim = Vec::new();
    let mut spf = Vec::new();
    let mut dmarc = Vec::new();
    parse_auth_results_header(value, &mut dkim, &mut spf, &mut dmarc);
    assert_eq!(dkim.len(), 1);
    assert_eq!(dkim[0].status, "fail");
  }

  #[test]
  fn auth_results_header_ignores_unknown_methods() {
    let value = "mx.example.com; custom=weird; dkim=pass";
    let mut dkim = Vec::new();
    let mut spf = Vec::new();
    let mut dmarc = Vec::new();
    parse_auth_results_header(value, &mut dkim, &mut spf, &mut dmarc);
    assert_eq!(dkim.len(), 1);
    assert!(spf.is_empty());
    assert!(dmarc.is_empty());
  }

  #[test]
  fn dkim_signature_missing_all_tags_produces_empty_details() {
    let check = parse_dkim_signature("junk");
    assert_eq!(check.status, "info");
    assert_eq!(check.details, "d= s= a=");
  }

  #[test]
  fn dkim_signature_extracts_domain_selector_algorithm() {
    let check = parse_dkim_signature(
      "v=1; a=rsa-sha256; d=example.com; s=selector1; h=from:to; bh=abc; b=xyz",
    );
    assert_eq!(check.details, "d=example.com s=selector1 a=rsa-sha256");
  }

  #[test]
  fn dkim_signature_tolerates_extra_whitespace_between_tags() {
    let check = parse_dkim_signature("  d = example.com ;  s = sel ; a = rsa-sha256 ");
    assert_eq!(check.details, "d=example.com s=sel a=rsa-sha256");
  }

  #[test]
  fn received_spf_empty_yields_unknown() {
    let check = parse_received_spf("");
    assert_eq!(check.status, "unknown");
    assert_eq!(check.details, "");
  }

  #[test]
  fn received_spf_extracts_status_token_lowercase() {
    let check = parse_received_spf("PASS (google.com: domain of a@b.com designates ...)");
    assert_eq!(check.status, "pass");
  }

  #[test]
  fn method_result_handles_segment_without_equals() {
    let (method, status, details) = parse_method_result("broken");
    assert_eq!(method, "");
    assert_eq!(status, "broken");
    assert_eq!(details, "broken");
  }
}

#[cfg(test)]
mod relay_transport_tests {
  use super::*;
  use lettre::AsyncTransport;
  use lettre::address::Envelope;
  use lettre::transport::smtp::client::{Certificate, TlsParameters};
  use rustls::pki_types::pem::PemObject;
  use rustls::pki_types::{CertificateDer, PrivateKeyDer};
  use rustmail_smtp::{Delivery, SmtpServer, SmtpServerConfig, TlsConfig};
  use std::sync::Arc;
  use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
  use tokio::net::TcpListener;
  use tokio::sync::mpsc;
  use tokio_rustls::TlsAcceptor;

  const LOOPBACK: &str = "127.0.0.1";
  const STARTTLS_SUBMISSION_PORT: u16 = 2525;
  const TLS_HANDSHAKE_RECORD: u8 = 0x16;
  const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);
  const STALL_DEADLINE: Duration = Duration::from_millis(200);
  const SCRIPTED_DEADLINE: Duration = Duration::from_secs(2);
  const GREETING: &[u8] = b"220 relay.test ESMTP\r\n";
  const EHLO_WITH_STARTTLS: &[u8] = b"250-relay.test\r\n250 STARTTLS\r\n";
  const EHLO_WITHOUT_STARTTLS: &[u8] = b"250 relay.test\r\n";
  const RAW_MESSAGE: &[u8] = b"Subject: relay\r\n\r\nBody.\r\n";
  const RELEASED_ID: &str = "01J0000000000000000000000Z";
  const RELEASED_AT: &str = "2026-11-26T12:00:00Z";

  /// Checks the answer the router gives for `outcome` against
  /// `docs/api.yaml`, with and without a release recorded on the message.
  fn assert_release_response_is_documented(outcome: &RelayOutcome) {
    let operation = crate::openapi::Operation {
      method: "POST".to_string(),
      path: "/api/v1/messages/{id}/release".to_string(),
    };
    let recorded_outcome = match outcome {
      RelayOutcome::Sent => Some(ReleaseOutcome::Sent),
      RelayOutcome::Unknown(_) => Some(ReleaseOutcome::Unknown),
      RelayOutcome::NotSent(_) => None,
    };
    let recordings = [
      None,
      recorded_outcome.map(|release_outcome| Release {
        released_at: RELEASED_AT.to_string(),
        release_outcome,
      }),
    ];
    for recorded in recordings {
      let (status, Json(body)) = release_response(RELEASED_ID, outcome.clone(), recorded);
      let bytes = serde_json::to_vec(&body).unwrap();
      crate::openapi::spec()
        .check_response(
          &operation,
          status.as_u16(),
          Some("application/json"),
          Some(&bytes),
        )
        .unwrap();
    }
  }

  fn envelope() -> Envelope {
    Envelope::new(
      Some("sender@example.com".parse().unwrap()),
      vec!["rcpt@example.com".parse().unwrap()],
    )
    .unwrap()
  }

  async fn loopback_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
  }

  async fn send_through(port: u16, security: RelaySecurity) -> bool {
    let tls = TlsParameters::new(LOOPBACK.to_string()).unwrap();
    let mailer = relay_transport(LOOPBACK, port, security, tls).unwrap();
    tokio::time::timeout(EXCHANGE_TIMEOUT, mailer.send_raw(&envelope(), RAW_MESSAGE))
      .await
      .expect("the client never gave up on the relay")
      .is_ok()
  }

  async fn plaintext_commands_until_close(
    listener: TcpListener,
    ehlo_reply: &'static [u8],
  ) -> Vec<String> {
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = BufReader::new(socket);
    socket.get_mut().write_all(GREETING).await.unwrap();
    let mut commands = Vec::new();
    let mut line = String::new();
    while socket.read_line(&mut line).await.unwrap() > 0 {
      let command = line.trim_end().to_string();
      line.clear();
      if command.starts_with("EHLO ") {
        socket.get_mut().write_all(ehlo_reply).await.unwrap();
      }
      let is_starttls = command == "STARTTLS";
      commands.push(command);
      if is_starttls {
        break;
      }
    }
    commands
  }

  #[test]
  fn port_465_is_implicit_tls_and_every_other_allowed_port_requires_starttls() {
    for port in ALLOWED_SMTP_PORTS {
      let expected = if *port == IMPLICIT_TLS_SMTP_PORT {
        RelaySecurity::ImplicitTls
      } else {
        RelaySecurity::RequiredStartTls
      };
      assert_eq!(RelaySecurity::for_port(*port), expected, "port {port}");
    }
  }

  #[tokio::test]
  async fn starttls_relay_sends_ehlo_then_starttls_in_plaintext() {
    let (listener, port) = loopback_listener().await;
    let relay = tokio::spawn(plaintext_commands_until_close(listener, EHLO_WITH_STARTTLS));

    assert!(!send_through(port, RelaySecurity::for_port(STARTTLS_SUBMISSION_PORT)).await);

    let commands = tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .unwrap()
      .unwrap();
    let verbs: Vec<&str> = commands
      .iter()
      .map(|c| c.split(' ').next().unwrap_or_default())
      .collect();
    assert_eq!(verbs, ["EHLO", "STARTTLS"]);
  }

  #[tokio::test]
  async fn a_bare_ipv6_relay_host_reaches_the_relay() {
    const IPV6_LOOPBACK: &str = "::1";
    let listener = TcpListener::bind((IPV6_LOOPBACK, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let relay = tokio::spawn(plaintext_commands_until_close(listener, EHLO_WITH_STARTTLS));

    let tls = TlsParameters::new(IPV6_LOOPBACK.to_string()).unwrap();
    let mailer =
      relay_transport(IPV6_LOOPBACK, port, RelaySecurity::RequiredStartTls, tls).unwrap();
    let sent = tokio::time::timeout(EXCHANGE_TIMEOUT, mailer.send_raw(&envelope(), RAW_MESSAGE))
      .await
      .expect("the client never gave up on the relay");
    assert!(sent.is_err());

    let commands = tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .unwrap()
      .unwrap();
    assert_eq!(commands.last().map(String::as_str), Some("STARTTLS"));
  }

  #[tokio::test]
  async fn starttls_relay_refuses_to_send_when_the_upgrade_is_not_offered() {
    let (listener, port) = loopback_listener().await;
    let relay = tokio::spawn(plaintext_commands_until_close(
      listener,
      EHLO_WITHOUT_STARTTLS,
    ));

    assert!(!send_through(port, RelaySecurity::RequiredStartTls).await);

    let commands = tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .unwrap()
      .unwrap();
    assert!(
      commands.iter().all(|c| !c.starts_with("MAIL")),
      "the client sent the envelope in plaintext: {commands:?}"
    );
  }

  #[tokio::test]
  async fn implicit_tls_relay_opens_with_a_tls_handshake_record() {
    let (listener, port) = loopback_listener().await;
    let relay = tokio::spawn(async move {
      let (mut socket, _) = listener.accept().await.unwrap();
      let mut first = [0u8; 1];
      socket.read_exact(&mut first).await.unwrap();
      first[0]
    });

    assert!(!send_through(port, RelaySecurity::for_port(IMPLICIT_TLS_SMTP_PORT)).await);

    let first_byte = tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .unwrap()
      .unwrap();
    assert_eq!(first_byte, TLS_HANDSHAKE_RECORD);
  }

  const RELAY_CERT: &[u8] =
    include_bytes!("../../rustmail-server/tests/fixtures/starttls-cert.pem");
  const RELAY_KEY: &[u8] = include_bytes!("../../rustmail-server/tests/fixtures/starttls-key.pem");
  const STUFFED_RAW_MESSAGE: &[u8] =
    b"From: sender@example.com\r\nTo: rcpt@example.com\r\nSubject: release\r\n\r\nFirst line.\r\n.A line that starts with a dot.\r\n.\r\nLast line.\r\n";

  fn relay_tls_config() -> TlsConfig {
    let certs = CertificateDer::pem_slice_iter(RELAY_CERT)
      .collect::<Result<Vec<_>, _>>()
      .unwrap();
    let key = PrivateKeyDer::from_pem_slice(RELAY_KEY).unwrap();
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
      rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    TlsConfig {
      server_config: Arc::new(server_config),
    }
  }

  async fn starttls_relay() -> (u16, mpsc::Receiver<Delivery>) {
    let (listener, port) = loopback_listener().await;
    let (deliveries, received) = mpsc::channel(1);
    let config = SmtpServerConfig {
      tls: Some(relay_tls_config()),
      ..SmtpServerConfig::default()
    };
    tokio::spawn(async move { SmtpServer::new(config, deliveries).serve(listener).await });
    (port, received)
  }

  fn trusting_relay_cert() -> TlsParameters {
    TlsParameters::builder(LOOPBACK.to_string())
      .add_root_certificate(Certificate::from_pem(RELAY_CERT).unwrap())
      .build_rustls()
      .unwrap()
  }

  #[tokio::test]
  async fn a_release_through_a_trusted_starttls_relay_delivers_the_captured_envelope_and_source() {
    let (port, mut received) = starttls_relay().await;
    let envelope = release_envelope(
      "sender@example.com",
      r#"["rcpt@example.com","cc@example.com","hidden-bcc@example.com"]"#,
    )
    .unwrap();
    let mailer = relay_transport(
      LOOPBACK,
      port,
      RelaySecurity::RequiredStartTls,
      trusting_relay_cert(),
    )
    .unwrap();

    let relay = tokio::spawn(async move {
      let (message, ack) = received.recv().await.unwrap().into_parts();
      ack.stored();
      message
    });
    let outcome = send_to_relay(&mailer, &envelope, STUFFED_RAW_MESSAGE, EXCHANGE_TIMEOUT).await;
    let message = tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .unwrap()
      .unwrap();

    assert!(matches!(outcome, RelayOutcome::Sent), "{outcome:?}");
    assert_release_response_is_documented(&outcome);
    assert_eq!(message.sender, "sender@example.com");
    assert_eq!(
      message.recipients,
      [
        "rcpt@example.com",
        "cc@example.com",
        "hidden-bcc@example.com"
      ]
    );
    assert_eq!(message.raw, STUFFED_RAW_MESSAGE);
  }

  #[tokio::test]
  async fn a_relay_that_stalls_before_any_data_is_a_release_that_was_not_sent() {
    let (listener, port) = loopback_listener().await;
    let relay = tokio::spawn(async move {
      let (mut socket, _) = listener.accept().await.unwrap();
      socket.write_all(GREETING).await.unwrap();
      let mut sink = Vec::new();
      let _ = socket.read_to_end(&mut sink).await;
    });
    let mailer = relay_transport(
      LOOPBACK,
      port,
      RelaySecurity::RequiredStartTls,
      trusting_relay_cert(),
    )
    .unwrap();

    let outcome = send_to_relay(&mailer, &envelope(), RAW_MESSAGE, STALL_DEADLINE).await;

    assert!(matches!(outcome, RelayOutcome::NotSent(_)), "{outcome:?}");
    assert_release_response_is_documented(&outcome);
    relay.abort();
  }

  #[derive(Clone, Copy)]
  enum AfterData {
    Disconnect,
    Stall,
  }

  async fn read_command<S: tokio::io::AsyncBufRead + Unpin>(stream: &mut S) -> Option<String> {
    let mut line = String::new();
    match stream.read_line(&mut line).await {
      Ok(0) | Err(_) => None,
      Ok(_) => Some(line.trim_end().to_ascii_uppercase()),
    }
  }

  async fn scripted_relay(listener: TcpListener, after_data: AfterData) {
    let acceptor = TlsAcceptor::from(relay_tls_config().server_config);
    loop {
      let (socket, _) = listener.accept().await.unwrap();
      let mut plain = BufReader::new(socket);
      plain.get_mut().write_all(GREETING).await.unwrap();
      while let Some(command) = read_command(&mut plain).await {
        if command.starts_with("EHLO") {
          plain.get_mut().write_all(EHLO_WITH_STARTTLS).await.unwrap();
        } else if command == "STARTTLS" {
          plain
            .get_mut()
            .write_all(b"220 go ahead\r\n")
            .await
            .unwrap();
          break;
        }
      }
      let mut secure = BufReader::new(acceptor.accept(plain.into_inner()).await.unwrap());
      while let Some(command) = read_command(&mut secure).await {
        if command == "QUIT" {
          secure.get_mut().write_all(b"221 bye\r\n").await.unwrap();
          break;
        }
        if command != "DATA" {
          secure.get_mut().write_all(b"250 ok\r\n").await.unwrap();
          continue;
        }
        secure
          .get_mut()
          .write_all(b"354 go ahead\r\n")
          .await
          .unwrap();
        while read_command(&mut secure)
          .await
          .is_some_and(|line| line != ".")
        {}
        match after_data {
          AfterData::Disconnect => return,
          AfterData::Stall => std::future::pending::<()>().await,
        }
      }
    }
  }

  async fn release_through_scripted_relay(
    after_data: AfterData,
  ) -> (RelayOutcome, tokio::task::JoinHandle<()>) {
    let (listener, port) = loopback_listener().await;
    let relay = tokio::spawn(scripted_relay(listener, after_data));
    let mailer = relay_transport(
      LOOPBACK,
      port,
      RelaySecurity::RequiredStartTls,
      trusting_relay_cert(),
    )
    .unwrap();

    let outcome = send_to_relay(&mailer, &envelope(), RAW_MESSAGE, SCRIPTED_DEADLINE).await;
    (outcome, relay)
  }

  #[tokio::test]
  async fn a_relay_that_disconnects_after_the_data_leaves_the_outcome_unknown() {
    let (outcome, relay) = release_through_scripted_relay(AfterData::Disconnect).await;

    assert!(matches!(outcome, RelayOutcome::Unknown(_)), "{outcome:?}");
    assert_release_response_is_documented(&outcome);
    tokio::time::timeout(EXCHANGE_TIMEOUT, relay)
      .await
      .expect("the relay never received the whole message")
      .unwrap();
  }

  #[tokio::test]
  async fn a_relay_that_stalls_after_the_data_leaves_the_outcome_unknown() {
    let (outcome, relay) = release_through_scripted_relay(AfterData::Stall).await;

    assert!(matches!(outcome, RelayOutcome::Unknown(_)), "{outcome:?}");
    assert_release_response_is_documented(&outcome);
    relay.abort();
  }
}

#[cfg(test)]
mod release_record_tests {
  use super::{merge_release, record_release};
  use crate::state::{AppState, WsFrame};
  use rustmail_storage::{MessageRepository, Release, ReleaseOutcome, initialize_database};
  use tokio::sync::broadcast;

  async fn state_with_message() -> (AppState, String, broadcast::Receiver<WsFrame>) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool);
    let id = repo
      .insert(
        "a@example.test",
        &["b@example.test".to_string()],
        b"Subject: released\r\n\r\nbody",
      )
      .await
      .unwrap()
      .id;
    let (ws_tx, ws_rx) = broadcast::channel::<WsFrame>(8);
    (AppState::new(repo, ws_tx, None, None), id, ws_rx)
  }

  #[tokio::test]
  async fn a_recorded_release_is_stored_and_broadcast_as_message_released() {
    let (state, id, mut ws_rx) = state_with_message().await;

    let release = record_release(&state, &id, ReleaseOutcome::Unknown)
      .await
      .unwrap();

    assert_eq!(release.release_outcome, ReleaseOutcome::Unknown);
    let stored = state.repo.get(&id).await.unwrap();
    assert_eq!(
      stored.released_at.as_deref(),
      Some(release.released_at.as_str())
    );
    assert_eq!(stored.release_outcome, Some(ReleaseOutcome::Unknown));
    let frame: serde_json::Value =
      serde_json::from_str(ws_rx.try_recv().unwrap().text().as_str()).unwrap();
    assert_eq!(
      frame,
      serde_json::json!({
        "type": "message:released",
        "data": { "id": id, "released_at": release.released_at, "release_outcome": "unknown" }
      })
    );
  }

  #[tokio::test]
  async fn a_release_of_a_message_deleted_meanwhile_records_and_broadcasts_nothing() {
    let (state, id, mut ws_rx) = state_with_message().await;
    state.repo.delete(&id).await.unwrap();

    let release = record_release(&state, &id, ReleaseOutcome::Sent).await;

    assert_eq!(release, None);
    assert!(ws_rx.try_recv().is_err());
  }

  #[test]
  fn the_recorded_release_is_added_to_the_response_body() {
    let mut body = serde_json::json!({ "released": true });

    merge_release(
      &mut body,
      Some(Release {
        released_at: "2026-11-26T13:05:09Z".into(),
        release_outcome: ReleaseOutcome::Sent,
      }),
    );

    assert_eq!(
      body,
      serde_json::json!({
        "released": true,
        "released_at": "2026-11-26T13:05:09Z",
        "release_outcome": "sent"
      })
    );
  }

  #[test]
  fn a_release_that_could_not_be_recorded_leaves_the_body_as_it_was() {
    let mut body = serde_json::json!({ "released": true });

    merge_release(&mut body, None);

    assert_eq!(body, serde_json::json!({ "released": true }));
  }
}
