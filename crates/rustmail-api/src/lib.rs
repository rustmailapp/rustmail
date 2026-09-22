//! HTTP and WebSocket API for the RustMail mail catcher.
//!
//! This crate provides an [`axum`] router with:
//!
//! - **REST endpoints** — CRUD for messages/attachments, full-text search,
//!   export (EML/JSON), email release via SMTP forwarding, and CI assertion endpoints
//! - **WebSocket** — Real-time push for new messages, deletions, and read-state changes
//! - **Embedded UI** — SolidJS frontend served as static files via [`rust_embed`]
//!
//! No CORS headers are sent, so a browser will not hand a cross-origin page the
//! response to a REST call; the bundled UI is served same-origin. The WebSocket
//! handshake is not governed by CORS, so it carries its own origin check —
//! see [`Origin`] and `--allowed-origin`. Browser requests are answered only on
//! an address, on `localhost`, or on a name given to `--allowed-host`, which is
//! what keeps DNS rebinding from turning either check into a formality — see
//! [`Hostname`]. Security layers include
//! `Content-Security-Policy`, `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
//! `Referrer-Policy: no-referrer`, and semaphore-based WebSocket connection limits.
//!
//! # Example
//!
//! ```no_run
//! use rustmail_api::{AppState, WsFrame, router};
//! use rustmail_storage::MessageRepository;
//! use tokio::sync::broadcast;
//!
//! # async fn example(repo: MessageRepository) -> Result<(), Box<dyn std::error::Error>> {
//! let (ws_tx, _) = broadcast::channel::<WsFrame>(256);
//! let state = AppState::new(repo, ws_tx, None, None);
//!
//! let app = router(state);
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:8025").await?;
//! axum::serve(listener, app).await?;
//! # Ok(())
//! # }
//! ```

mod handlers;
mod host;
mod origin;
mod state;
mod static_files;
mod ws;

pub use host::{Hostname, HostnameError};
pub use origin::{Origin, OriginError};
pub use state::{AppState, WsEvent, WsFrame, WsFrameError};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use tower_http::compression::predicate::{NotForContentType, Predicate};
use tower_http::compression::{CompressionLayer, DefaultPredicate};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

/// Content type of attachment downloads.
///
/// Attachments are mostly archives, PDFs and images that are compressed
/// already, so gzipping them again spends CPU on every download for nothing.
const DOWNLOAD_CONTENT_TYPE: &str = "application/octet-stream";

/// Builds the complete axum router with all API routes, static file serving,
/// compression, tracing, and security headers.
///
/// A `GET` or `HEAD` on `/api/v1` is answered `503` once it runs past the
/// request timeout. Writes are left to finish, because the storage and SMTP
/// work behind them carries on after the response is dropped, and a `503`
/// would then report a failure for a change that still lands.
pub fn router(state: AppState) -> Router {
  let api = Router::new()
    .route("/messages", get(handlers::list_messages))
    .route("/messages", delete(handlers::delete_all_messages))
    .route("/messages/{id}", get(handlers::get_message))
    .route("/messages/{id}", patch(handlers::update_message))
    .route("/messages/{id}", delete(handlers::delete_message))
    .route("/messages/{id}/raw", get(handlers::get_raw_message))
    .route("/messages/{id}/headers", get(handlers::get_headers))
    .route(
      "/messages/{id}/attachments",
      get(handlers::list_attachments),
    )
    .route(
      "/messages/{id}/attachments/{aid}",
      get(handlers::get_attachment),
    )
    .route(
      "/messages/{id}/inline/{cid}",
      get(handlers::get_inline_attachment),
    )
    .route("/messages/{id}/auth", get(handlers::get_auth_results))
    .route("/messages/{id}/export", get(handlers::export_message))
    .route("/messages/{id}/release", post(handlers::release_message))
    .route("/assert/count", get(handlers::assert_count))
    .layer(axum::middleware::from_fn_with_state(
      state.clone(),
      time_out_reads,
    ))
    .route("/ws", get(ws::ws_handler));

  Router::new()
    .nest("/api/v1", api)
    .fallback(static_files::static_handler)
    .layer(axum::middleware::from_fn_with_state(
      state.clone(),
      host::guard_host,
    ))
    .layer(CompressionLayer::new().compress_when(
      DefaultPredicate::new().and(NotForContentType::const_new(DOWNLOAD_CONTENT_TYPE)),
    ))
    .layer(TraceLayer::new_for_http())
    .layer(SetResponseHeaderLayer::if_not_present(
      axum::http::header::X_CONTENT_TYPE_OPTIONS,
      HeaderValue::from_static("nosniff"),
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
      axum::http::header::X_FRAME_OPTIONS,
      HeaderValue::from_static("DENY"),
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
      axum::http::header::CONTENT_SECURITY_POLICY,
      HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: http: https:; connect-src 'self' ws: wss:; frame-src 'self'",
      ),
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
      axum::http::header::REFERRER_POLICY,
      HeaderValue::from_static("no-referrer"),
    ))
    .with_state(state)
}

async fn time_out_reads(State(state): State<AppState>, request: Request, next: Next) -> Response {
  if !is_read(request.method()) {
    return next.run(request).await;
  }
  tokio::time::timeout(state.api_timeout, next.run(request))
    .await
    .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

fn is_read(method: &Method) -> bool {
  method == Method::GET || method == Method::HEAD
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::body::Body;
  use axum::http::{Method, Request, StatusCode};
  use rustmail_storage::{MessageRepository, initialize_database};
  use sqlx::SqlitePool;
  use std::time::Duration;
  use tokio::sync::broadcast;
  use tower::ServiceExt;

  const SHORT_TIMEOUT: Duration = Duration::from_millis(50);
  const POOL_WAIT: Duration = Duration::from_secs(30);

  async fn single_connection_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .max_connections(1)
      .acquire_timeout(POOL_WAIT)
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    pool
  }

  fn short_timeout_router(pool: &SqlitePool) -> Router {
    let (ws_tx, _) = broadcast::channel::<WsFrame>(1);
    let mut state = AppState::new(MessageRepository::new(pool.clone()), ws_tx, None, None);
    state.api_timeout = SHORT_TIMEOUT;
    router(state)
  }

  fn request(method: Method, uri: &str) -> Request<Body> {
    Request::builder()
      .method(method)
      .uri(uri)
      .body(Body::empty())
      .unwrap()
  }

  #[tokio::test]
  async fn a_read_that_outlasts_the_timeout_is_answered_503() {
    let pool = single_connection_pool().await;
    let _only_connection = pool.acquire().await.unwrap();

    let response = short_timeout_router(&pool)
      .oneshot(request(Method::GET, "/api/v1/messages"))
      .await
      .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
  }

  #[tokio::test]
  async fn a_write_that_outlasts_the_timeout_runs_to_completion() {
    let pool = single_connection_pool().await;
    let only_connection = pool.acquire().await.unwrap();
    let mut response =
      Box::pin(short_timeout_router(&pool).oneshot(request(Method::DELETE, "/api/v1/messages")));

    let still_running = tokio::time::timeout(SHORT_TIMEOUT * 4, &mut response).await;
    assert!(
      still_running.is_err(),
      "the write was cut at the read timeout"
    );
    drop(only_connection);

    assert_eq!(response.await.unwrap().status(), StatusCode::OK);
  }
}
