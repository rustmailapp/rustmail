//! A located attachment that no longer decodes to its recorded size is
//! answered with a 500 and an error log, never with other bytes.

use std::fmt::{Debug, Write};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rustmail_api::{AppState, WsFrame, router};
use rustmail_storage::{MessageRepository, initialize_database};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

const PAYLOAD: &str = "CONFIDENTIAL-ATTACHMENT-PAYLOAD";

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<(Level, String)>>>);

impl Recorder {
  fn events(&self) -> Vec<(Level, String)> {
    self.0.lock().unwrap().clone()
  }
}

struct FieldText(String);

impl Visit for FieldText {
  fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
    write!(self.0, "{}={:?} ", field.name(), value).unwrap();
  }
}

impl Subscriber for Recorder {
  fn enabled(&self, _: &Metadata<'_>) -> bool {
    true
  }

  fn new_span(&self, _: &Attributes<'_>) -> Id {
    Id::from_u64(1)
  }

  fn record(&self, _: &Id, _: &Record<'_>) {}

  fn record_follows_from(&self, _: &Id, _: &Id) {}

  fn event(&self, event: &Event<'_>) {
    let mut fields = FieldText(String::new());
    event.record(&mut fields);
    self
      .0
      .lock()
      .unwrap()
      .push((*event.metadata().level(), fields.0));
  }

  fn enter(&self, _: &Id) {}

  fn exit(&self, _: &Id) {}
}

fn email_with_binary_attachment() -> Vec<u8> {
  format!(
    "From: a@test.com\r\nTo: b@test.com\r\nSubject: Corrupt\r\nMIME-Version: 1.0\r\n\
     Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
     --B\r\nContent-Type: text/plain\r\n\r\nBody\r\n\
     --B\r\nContent-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"blob.bin\"\r\n\r\n\
     {PAYLOAD}\r\n--B--\r\n"
  )
  .into_bytes()
}

async fn setup() -> (axum::Router, MessageRepository, SqlitePool) {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool.clone());
  let (ws_tx, _) = broadcast::channel::<WsFrame>(16);
  let app = router(AppState::new(repo.clone(), ws_tx, None, None));
  (app, repo, pool)
}

#[tokio::test]
async fn a_corrupt_locator_answers_500_and_logs_without_the_payload() {
  let recorder = Recorder::default();
  let _guard = tracing::subscriber::set_default(recorder.clone());
  let (app, repo, pool) = setup().await;
  let message = repo
    .insert(
      "a@test.com",
      &["b@test.com".to_string()],
      &email_with_binary_attachment(),
    )
    .await
    .unwrap();
  let attachment = repo.get_attachments(&message.id).await.unwrap().remove(0);
  let located = sqlx::query_scalar::<_, i64>(
    "UPDATE attachments SET raw_len = raw_len + 2 WHERE id = ?1 AND raw_offset IS NOT NULL RETURNING 1",
  )
  .bind(&attachment.id)
  .fetch_optional(&pool)
  .await
  .unwrap()
  .is_some();
  assert!(located, "the binary attachment should have been located");

  let response = app
    .oneshot(
      Request::get(format!(
        "/api/v1/messages/{}/attachments/{}",
        message.id, attachment.id
      ))
      .body(Body::empty())
      .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert!(!String::from_utf8_lossy(&body).contains(PAYLOAD));

  let events = recorder.events();
  let (level, fields) = events
    .iter()
    .find(|(_, fields)| fields.contains("event=\"attachment_corrupt\""))
    .unwrap_or_else(|| panic!("no attachment_corrupt event in {events:?}"));
  assert_eq!(*level, Level::ERROR);
  assert!(fields.contains(&format!("message_id={}", message.id)));
  assert!(fields.contains(&format!("attachment_id={}", attachment.id)));
  assert!(fields.contains("transfer_encoding=Some(0)"));
  assert!(fields.contains(&format!("expected_size=Some({})", PAYLOAD.len())));
  assert!(fields.contains(&format!("actual_size=Some({})", PAYLOAD.len() + 2)));
  assert!(
    events.iter().all(|(_, fields)| !fields.contains(PAYLOAD)),
    "the payload must never reach the log"
  );
}
