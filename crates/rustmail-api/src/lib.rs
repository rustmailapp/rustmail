//! HTTP and WebSocket API for the RustMail mail catcher.
//!
//! This crate provides an [`axum`] router with:
//!
//! - **REST endpoints** — CRUD for messages/attachments, full-text search,
//!   export (EML/JSON), email release via SMTP forwarding, and CI assertion endpoints
//! - **WebSocket** — Real-time push for new messages, deletions, and read-state changes
//! - **Embedded UI** — SolidJS frontend served as static files via [`rust_embed`]
//!
//! Security layers include CORS (`Access-Control-Allow-Origin: *` without credentials),
//! `Content-Security-Policy`, `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
//! `Referrer-Policy: no-referrer`, and semaphore-based WebSocket connection limits.
//!
//! # Example
//!
//! ```no_run
//! use rustmail_api::{AppState, WsEvent, router};
//! use rustmail_storage::MessageRepository;
//! use tokio::sync::broadcast;
//!
//! # async fn example(repo: MessageRepository) -> Result<(), Box<dyn std::error::Error>> {
//! let (ws_tx, _) = broadcast::channel::<WsEvent>(256);
//! let state = AppState::new(repo, ws_tx, None, None);
//!
//! let app = router(state);
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:8025").await?;
//! axum::serve(listener, app).await?;
//! # Ok(())
//! # }
//! ```

mod handlers;
mod state;
mod static_files;
mod ws;

pub use state::{AppState, WsEvent};

use axum::Router;
use axum::http::HeaderValue;
use axum::routing::{delete, get, patch, post};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

/// Builds the complete axum router with all API routes, static file serving,
/// CORS, tracing, and security headers.
pub fn router(state: AppState) -> Router {
  let api = Router::new()
    .route("/messages", get(handlers::list_messages))
    .route("/messages", delete(handlers::delete_all_messages))
    .route("/messages/{id}", get(handlers::get_message))
    .route("/messages/{id}", patch(handlers::update_message))
    .route("/messages/{id}", delete(handlers::delete_message))
    .route("/messages/{id}/raw", get(handlers::get_raw_message))
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
    .route("/ws", get(ws::ws_handler));

  let cors = CorsLayer::new()
    .allow_origin(AllowOrigin::any())
    .allow_methods(tower_http::cors::Any)
    .allow_headers(tower_http::cors::Any);

  Router::new()
    .nest("/api/v1", api)
    .fallback(static_files::static_handler)
    .layer(cors)
    .layer(CompressionLayer::new())
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
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ws: wss:; frame-src 'self'",
      ),
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
      axum::http::header::REFERRER_POLICY,
      HeaderValue::from_static("no-referrer"),
    ))
    .with_state(state)
}
