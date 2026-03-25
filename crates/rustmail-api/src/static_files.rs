use std::borrow::Cow;

use axum::body::Bytes;
use axum::http::{StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../ui/dist"]
#[exclude = ".DS_Store"]
struct Assets;

pub async fn static_handler(uri: Uri) -> Response {
  let path = uri.path().trim_start_matches('/');

  if let Some(file) = Assets::get(path) {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let bytes = match file.data {
      Cow::Borrowed(data) => Bytes::from_static(data),
      Cow::Owned(data) => Bytes::from(data),
    };
    (
      StatusCode::OK,
      [(header::CONTENT_TYPE, mime.as_ref().to_string())],
      bytes,
    )
      .into_response()
  } else if let Some(index) = Assets::get("index.html") {
    Html(
      std::str::from_utf8(&index.data)
        .unwrap_or_default()
        .to_string(),
    )
    .into_response()
  } else {
    (StatusCode::NOT_FOUND, "Not found").into_response()
  }
}
