use std::borrow::Cow;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::{Embed, EmbeddedFile};

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
const WEAK_PREFIX: &str = "W/";
const ANY_ETAG: &str = "*";
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
/// tell apart from a real response. So are unmatched hashed assets: a tab
/// left open across an upgrade asks for the old build's files, and a `404`
/// fails cleanly where HTML served as a script is a MIME error.
pub async fn static_handler(uri: Uri, headers: HeaderMap) -> Response {
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
    if path.starts_with(HASHED_ASSET_PREFIX) {
      (
        StatusCode::OK,
        [
          (header::CONTENT_TYPE, mime.as_ref().to_string()),
          (header::CACHE_CONTROL, IMMUTABLE_CACHE_CONTROL.to_string()),
        ],
        body_of(file.data),
      )
        .into_response()
    } else {
      revalidated(file, mime.as_ref(), &headers)
    }
  } else if path.starts_with(HASHED_ASSET_PREFIX) {
    (StatusCode::NOT_FOUND, "Not found").into_response()
  } else if let Some(index) = Assets::get("index.html") {
    revalidated(index, HTML_CONTENT_TYPE, &headers)
  } else {
    (StatusCode::NOT_FOUND, "Not found").into_response()
  }
}

/// Serves a file the browser must revalidate, answering `304` when the
/// request already holds this build of it.
///
/// The ETag is weak because the compression layer may gzip the body, and a
/// strong one would then name two different byte sequences.
fn revalidated(file: EmbeddedFile, content_type: &str, headers: &HeaderMap) -> Response {
  let etag = etag_of(&file);
  if holds_current(headers, &etag) {
    return (
      StatusCode::NOT_MODIFIED,
      [
        (header::ETAG, etag),
        (header::CACHE_CONTROL, REVALIDATE_CACHE_CONTROL.to_string()),
      ],
    )
      .into_response();
  }
  (
    StatusCode::OK,
    [
      (header::CONTENT_TYPE, content_type.to_string()),
      (header::CACHE_CONTROL, REVALIDATE_CACHE_CONTROL.to_string()),
      (header::ETAG, etag),
    ],
    body_of(file.data),
  )
    .into_response()
}

fn etag_of(file: &EmbeddedFile) -> String {
  let hex: String = file
    .metadata
    .sha256_hash()
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect();
  format!("W/\"{hex}\"")
}

fn holds_current(headers: &HeaderMap, etag: &str) -> bool {
  let opaque = etag.trim_start_matches(WEAK_PREFIX);
  headers
    .get_all(header::IF_NONE_MATCH)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .map(str::trim)
    .any(|candidate| candidate == ANY_ETAG || candidate.trim_start_matches(WEAK_PREFIX) == opaque)
}
