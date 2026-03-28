use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::state::{AppState, WsEvent};
use rustmail_storage::StorageError;

#[derive(Deserialize)]
pub struct ListParams {
  pub q: Option<String>,
  pub limit: Option<i64>,
  pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateBody {
  pub is_read: Option<bool>,
  pub is_starred: Option<bool>,
  pub tags: Option<Vec<String>>,
}

pub async fn list_messages(
  State(state): State<AppState>,
  Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, AppError> {
  let limit = params.limit.unwrap_or(50).clamp(1, 200);
  let offset = params.offset.unwrap_or(0).max(0);

  let (messages, count) = if let Some(query) = &params.q {
    let msgs = state.repo.search(query, limit, offset).await?;
    let total = state.repo.search_count(query).await?;
    (msgs, total)
  } else {
    let msgs = state.repo.list(limit, offset).await?;
    let total = state.repo.count().await?;
    (msgs, total)
  };

  Ok(Json(serde_json::json!({
      "messages": messages,
      "total": count,
  })))
}

pub async fn get_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let message = state.repo.get(&id).await?;
  Ok(Json(message))
}

const MAX_TAGS: usize = 20;
const MAX_TAG_LEN: usize = 50;

fn validate_tags(tags: &[String]) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
  if tags.len() > MAX_TAGS {
    return Err((
      StatusCode::BAD_REQUEST,
      Json(serde_json::json!({ "error": format!("Too many tags (max {})", MAX_TAGS) })),
    ));
  }
  for tag in tags {
    if tag.len() > MAX_TAG_LEN {
      return Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": format!("Tag too long (max {} chars)", MAX_TAG_LEN) })),
      ));
    }
    if tag.is_empty() || tag.chars().any(|c| c.is_control()) {
      return Err((
        StatusCode::BAD_REQUEST,
        Json(
          serde_json::json!({ "error": "Tags must be non-empty and contain no control characters" }),
        ),
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
  if let Some(ref tags) = body.tags
    && let Err(e) = validate_tags(tags)
  {
    return Ok(e.into_response());
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
  Ok(Json(serde_json::json!({ "deleted": count })))
}

pub async fn list_attachments(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let attachments = state.repo.get_attachments(&id).await?;
  Ok(Json(attachments))
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
      (header::CONTENT_TYPE, "application/octet-stream".to_string()),
      (
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", filename),
      ),
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

  let content_type = attachment
    .content_type
    .filter(|ct| ct.starts_with("image/") && !ct.contains("svg"))
    .unwrap_or_else(|| "application/octet-stream".to_string());

  Ok((
    StatusCode::OK,
    [
      (header::CONTENT_TYPE, content_type),
      (
        header::CACHE_CONTROL,
        "public, max-age=31536000, immutable".to_string(),
      ),
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

pub async fn get_raw_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
  let raw = state.repo.get_raw(&id).await?;
  Ok((
    StatusCode::OK,
    [(header::CONTENT_TYPE, "message/rfc822".to_string())],
    raw,
  ))
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
          ],
          raw,
        )
          .into_response(),
      )
    }
    "json" => {
      let msg = state.repo.get(&id).await?;
      Ok(
        (
          StatusCode::OK,
          [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (
              header::CONTENT_DISPOSITION,
              format!("attachment; filename=\"{}.json\"", sanitize_filename(&id)),
            ),
          ],
          serde_json::to_vec(&msg).unwrap_or_default(),
        )
          .into_response(),
      )
    }
    _ => Ok(
      (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": "format must be 'eml' or 'json'" })),
      )
        .into_response(),
    ),
  }
}

#[derive(Deserialize)]
pub struct ReleaseBody {
  pub host: String,
  pub port: Option<u16>,
}

const ALLOWED_SMTP_PORTS: &[u16] = &[25, 465, 587, 2525];

pub async fn release_message(
  State(state): State<AppState>,
  Path(id): Path<String>,
  Json(body): Json<ReleaseBody>,
) -> Result<impl IntoResponse, AppError> {
  let (allowed_host, allowed_port) = match (&state.release_host, state.release_port) {
    (Some(host), port) => (host.clone(), port),
    _ => {
      return Ok(
        (
          StatusCode::FORBIDDEN,
          Json(
            serde_json::json!({ "error": "Email release is disabled. Configure --release-host to enable." }),
          ),
        )
          .into_response(),
      );
    }
  };

  if body.host != allowed_host {
    return Ok(
      (
        StatusCode::FORBIDDEN,
        Json(
          serde_json::json!({ "error": format!("Release only allowed to configured host: {}", allowed_host) }),
        ),
      )
        .into_response(),
    );
  }

  let port = body.port.unwrap_or(allowed_port.unwrap_or(587));
  if allowed_port.is_some() && Some(port) != allowed_port {
    return Ok(
      (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": format!("Release only allowed on configured port: {}", allowed_port.unwrap_or(587)) })),
      )
        .into_response(),
    );
  }
  if !ALLOWED_SMTP_PORTS.contains(&port) {
    return Ok(
      (
        StatusCode::BAD_REQUEST,
        Json(
          serde_json::json!({ "error": format!("Port {} is not an allowed SMTP port (25, 465, 587, 2525)", port) }),
        ),
      )
        .into_response(),
    );
  }

  let raw = state.repo.get_raw(&id).await?;
  let msg = state.repo.get(&id).await?;

  let envelope = lettre::address::Envelope::new(
    msg.sender.parse().ok(),
    serde_json::from_str::<Vec<String>>(&msg.recipients)
      .unwrap_or_default()
      .iter()
      .filter_map(|r| r.parse().ok())
      .collect(),
  );

  match envelope {
    Ok(envelope) => {
      use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

      let mailer_result =
        AsyncSmtpTransport::<Tokio1Executor>::relay(&body.host).map(|b| b.port(port).build());

      let mailer = match mailer_result {
        Ok(m) => m,
        Err(e) => {
          tracing::error!(error = %e, "TLS setup failed for relay host");
          return Ok(
            (
              StatusCode::BAD_GATEWAY,
              Json(
                serde_json::json!({ "error": "Failed to establish TLS connection to relay host" }),
              ),
            )
              .into_response(),
          );
        }
      };

      match mailer.send_raw(&envelope, &raw).await {
        Ok(_) => Ok(
          (
            StatusCode::OK,
            Json(serde_json::json!({ "released": true })),
          )
            .into_response(),
        ),
        Err(e) => {
          tracing::error!(error = %e, "SMTP delivery failed");
          Ok(
            (
              StatusCode::BAD_GATEWAY,
              Json(serde_json::json!({ "error": "SMTP delivery failed" })),
            )
              .into_response(),
          )
        }
      }
    }
    Err(e) => Ok(
      (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": format!("Invalid envelope: {}", e) })),
      )
        .into_response(),
    ),
  }
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
  let raw = state.repo.get_raw(&id).await?;

  let parsed = mail_parser::MessageParser::default().parse(&raw);
  let headers = parsed
    .as_ref()
    .and_then(|msg| msg.parts.first())
    .map(|root| root.headers.as_slice())
    .unwrap_or_default();

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

  Ok(Json(AuthResults {
    dkim,
    spf,
    dmarc,
    arc,
  }))
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

pub struct AppError(StorageError);

impl From<StorageError> for AppError {
  fn from(e: StorageError) -> Self {
    Self(e)
  }
}

impl IntoResponse for AppError {
  fn into_response(self) -> axum::response::Response {
    let (status, message) = match &self.0 {
      StorageError::NotFound(_) => (StatusCode::NOT_FOUND, "Resource not found".to_string()),
      StorageError::Database(e) => {
        tracing::error!(error = %e, "Database error");
        (
          StatusCode::INTERNAL_SERVER_ERROR,
          "Internal server error".to_string(),
        )
      }
    };

    (status, Json(serde_json::json!({ "error": message }))).into_response()
  }
}
