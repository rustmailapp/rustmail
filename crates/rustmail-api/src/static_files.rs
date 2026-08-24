use std::borrow::Cow;

use axum::body::Bytes;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../ui/dist"]
#[exclude = ".DS_Store"]
struct Assets;

/// Vite writes content-hashed filenames into this directory, so those URLs
/// never change meaning and can be cached forever.
const HASHED_ASSET_PREFIX: &str = "assets/";
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
const REVALIDATE_CACHE_CONTROL: &str = "no-cache";
const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
/// Requests under this prefix are API calls, never client-side routes.
const API_PREFIX: &str = "api/";

fn body_of(data: Cow<'static, [u8]>) -> Bytes {
  match data {
    Cow::Borrowed(data) => Bytes::from_static(data),
    Cow::Owned(data) => Bytes::from(data),
  }
}

/// Serves the embedded UI, falling back to `index.html` so the client-side
/// router owns unknown paths.
///
/// Unmatched API paths are excluded from that fallback: answering them with
/// the SPA shell hands an API client a `200` full of HTML, which it cannot
/// tell apart from a real response.
pub async fn static_handler(uri: Uri) -> Response {
  let path = uri.path().trim_start_matches('/');

  if path.starts_with(API_PREFIX) {
    return (
      StatusCode::NOT_FOUND,
      axum::Json(serde_json::json!({ "error": "Unknown API endpoint" })),
    )
      .into_response();
  }

  if let Some(file) = Assets::get(path) {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache_control = if path.starts_with(HASHED_ASSET_PREFIX) {
      IMMUTABLE_CACHE_CONTROL
    } else {
      REVALIDATE_CACHE_CONTROL
    };

    (
      StatusCode::OK,
      [
        (header::CONTENT_TYPE, mime.as_ref().to_string()),
        (header::CACHE_CONTROL, cache_control.to_string()),
      ],
      body_of(file.data),
    )
      .into_response()
  } else if let Some(index) = Assets::get("index.html") {
    (
      StatusCode::OK,
      [
        (header::CONTENT_TYPE, HTML_CONTENT_TYPE.to_string()),
        (header::CACHE_CONTROL, REVALIDATE_CACHE_CONTROL.to_string()),
      ],
      body_of(index.data),
    )
      .into_response()
  } else {
    (StatusCode::NOT_FOUND, "Not found").into_response()
  }
}
