use axum::body::Body;
use axum::http::{Request, StatusCode};
use rustmail_api::{AppState, WsEvent, router};
use rustmail_storage::{MessageRepository, initialize_database};
use serde_json::Value;
use tokio::sync::broadcast;
use tower::ServiceExt;

async fn setup() -> (axum::Router, MessageRepository, broadcast::Sender<WsEvent>) {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();

  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsEvent>(256);
  let state = AppState::new(repo.clone(), ws_tx.clone(), None, None);
  let app = router(state);

  (app, repo, ws_tx)
}

fn raw_email(subject: &str, from: &str, to: &str) -> Vec<u8> {
  format!(
    "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\nHello world"
  )
  .into_bytes()
}

async fn json_body(response: axum::response::Response) -> Value {
  let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
    .await
    .unwrap();
  serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_messages_empty() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["messages"].as_array().unwrap().len(), 0);
  assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn list_messages_after_insert() {
  let (app, repo, _) = setup().await;
  repo
    .insert(
      "alice@test.com",
      &["bob@test.com".into()],
      &raw_email("Hello", "alice@test.com", "bob@test.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["total"], 1);
  let messages = body["messages"].as_array().unwrap();
  assert_eq!(messages.len(), 1);
  assert_eq!(messages[0]["sender"], "alice@test.com");
  assert_eq!(messages[0]["subject"], "Hello");
}

#[tokio::test]
async fn get_message_by_id() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "alice@test.com",
      &["bob@test.com".into()],
      &raw_email("Fetch me", "alice@test.com", "bob@test.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["id"], summary.id);
  assert_eq!(body["text_body"], "Hello world");
}

#[tokio::test]
async fn get_message_not_found() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages/nonexistent")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_message() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Delete me", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NO_CONTENT);
  assert_eq!(repo.count().await.unwrap(), 0);
}

#[tokio::test]
async fn delete_all_messages() {
  let (app, repo, _) = setup().await;
  for i in 0..3 {
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
  }

  let response = app
    .oneshot(
      Request::builder()
        .method("DELETE")
        .uri("/api/v1/messages")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["deleted"], 3);
  assert_eq!(repo.count().await.unwrap(), 0);
}

#[tokio::test]
async fn update_message_read_status() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Patch me", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"is_read": true}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NO_CONTENT);
  let msg = repo.get(&summary.id).await.unwrap();
  assert!(msg.is_read);
}

#[tokio::test]
async fn update_message_tags() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Tag me", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"tags": ["urgent", "review"]}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NO_CONTENT);
  let msg = repo.get(&summary.id).await.unwrap();
  let tags: Vec<String> = serde_json::from_str(&msg.tags).unwrap();
  assert_eq!(tags, vec!["urgent", "review"]);
}

#[tokio::test]
async fn update_message_rejects_too_many_tags() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Tags", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let tags: Vec<String> = (0..25).map(|i| format!("tag{i}")).collect();
  let body = serde_json::json!({ "tags": tags });

  let response = app
    .oneshot(
      Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_raw_message() {
  let (app, repo, _) = setup().await;
  let raw = raw_email("Raw", "a@t.com", "b@t.com");
  let summary = repo
    .insert("a@t.com", &["b@t.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/raw", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.headers().get("content-type").unwrap(),
    "message/rfc822"
  );
  let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
    .await
    .unwrap();
  assert_eq!(bytes.as_ref(), raw.as_slice());
}

#[tokio::test]
async fn search_via_query_param() {
  let (app, repo, _) = setup().await;
  repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Invoice #99", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();
  repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Meeting", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages?q=Invoice")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  let messages = body["messages"].as_array().unwrap();
  assert_eq!(messages.len(), 1);
  assert_eq!(messages[0]["subject"], "Invoice #99");
}

#[tokio::test]
async fn assert_count_passes() {
  let (app, repo, _) = setup().await;
  repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Welcome", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/assert/count?min=1&subject=Welcome")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["ok"], true);
  assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn assert_count_fails_when_below_min() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/assert/count?min=1")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::EXPECTATION_FAILED);
  let body = json_body(response).await;
  assert_eq!(body["ok"], false);
}

#[tokio::test]
async fn assert_count_with_max() {
  let (app, repo, _) = setup().await;
  for i in 0..5 {
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
  }

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/assert/count?min=1&max=3")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::EXPECTATION_FAILED);
  let body = json_body(response).await;
  assert_eq!(body["ok"], false);
  assert_eq!(body["count"], 5);
}

#[tokio::test]
async fn export_eml() {
  let (app, repo, _) = setup().await;
  let raw = raw_email("Export", "a@t.com", "b@t.com");
  let summary = repo
    .insert("a@t.com", &["b@t.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/export?format=eml", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.headers().get("content-type").unwrap(),
    "message/rfc822"
  );
  assert!(
    response
      .headers()
      .get("content-disposition")
      .unwrap()
      .to_str()
      .unwrap()
      .contains(".eml")
  );
}

#[tokio::test]
async fn export_json() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("JSON Export", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!(
          "/api/v1/messages/{}/export?format=json",
          summary.id
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.headers().get("content-type").unwrap(),
    "application/json"
  );
  let body = json_body(response).await;
  assert_eq!(body["subject"], "JSON Export");
}

#[tokio::test]
async fn export_invalid_format() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("X", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/export?format=csv", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn release_disabled_without_flag() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Release", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .method("POST")
        .uri(format!("/api/v1/messages/{}/release", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"host": "smtp.example.com"}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn release_rejects_wrong_host() {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsEvent>(256);
  let state = AppState::new(
    repo.clone(),
    ws_tx,
    Some("allowed.example.com".into()),
    Some(587),
  );
  let app = router(state);

  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Release", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .method("POST")
        .uri(format!("/api/v1/messages/{}/release", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"host": "evil.example.com"}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn security_headers_present() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(
    response.headers().get("x-content-type-options").unwrap(),
    "nosniff"
  );
  assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
}

#[tokio::test]
async fn ws_broadcast_on_delete() {
  let (app, repo, ws_tx) = setup().await;
  let mut rx = ws_tx.subscribe();

  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("WS", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let _response = app
    .oneshot(
      Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  let event = rx.try_recv().unwrap();
  match event {
    WsEvent::MessageDelete { id } => assert_eq!(id, summary.id),
    _ => panic!("Expected MessageDelete event"),
  }
}

#[tokio::test]
async fn ws_broadcast_on_clear() {
  let (app, repo, ws_tx) = setup().await;
  let mut rx = ws_tx.subscribe();

  repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Clear", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let _response = app
    .oneshot(
      Request::builder()
        .method("DELETE")
        .uri("/api/v1/messages")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  let event = rx.try_recv().unwrap();
  assert!(matches!(event, WsEvent::MessagesClear));
}

#[tokio::test]
async fn list_messages_respects_limit() {
  let (app, repo, _) = setup().await;
  for i in 0..10 {
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
  }

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages?limit=3")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  let body = json_body(response).await;
  assert_eq!(body["messages"].as_array().unwrap().len(), 3);
  assert_eq!(body["total"], 10);
}

fn email_with_auth_headers(subject: &str) -> Vec<u8> {
  format!(
    "From: sender@example.com\r\n\
     To: rcpt@example.com\r\n\
     Subject: {subject}\r\n\
     Authentication-Results: mx.example.com;\r\n\
     \tdkim=pass header.d=example.com header.s=sel1;\r\n\
     \tspf=pass smtp.mailfrom=sender@example.com;\r\n\
     \tdmarc=pass header.from=example.com\r\n\
     DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=sel1;\r\n\
     \th=from:to:subject; b=abc123\r\n\
     Received-SPF: Pass (sender SPF authorized) identity=mailfrom\r\n\
     ARC-Authentication-Results: i=1; mx.example.com;\r\n\
     \tdkim=pass header.d=example.com\r\n\
     Content-Type: text/plain\r\n\
     \r\n\
     Authenticated email body"
  )
  .into_bytes()
}

#[tokio::test]
async fn auth_results_parsed_correctly() {
  let (app, repo, _) = setup().await;
  let raw = email_with_auth_headers("Auth Test");
  let summary = repo
    .insert("sender@example.com", &["rcpt@example.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/auth", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;

  let dkim = body["dkim"].as_array().unwrap();
  assert!(dkim.len() >= 2);
  let has_dkim_pass = dkim.iter().any(|c| c["status"] == "pass");
  assert!(has_dkim_pass, "Expected a dkim=pass check");
  let has_dkim_sig = dkim.iter().any(|c| c["status"] == "info");
  assert!(has_dkim_sig, "Expected DKIM-Signature info entry");

  let spf = body["spf"].as_array().unwrap();
  assert!(!spf.is_empty());
  let has_spf_pass = spf.iter().any(|c| c["status"] == "pass");
  assert!(has_spf_pass, "Expected a spf=pass check");

  let dmarc = body["dmarc"].as_array().unwrap();
  assert!(!dmarc.is_empty());
  assert_eq!(dmarc[0]["status"], "pass");

  let arc = body["arc"].as_array().unwrap();
  assert!(!arc.is_empty());
  let has_arc_dkim = arc
    .iter()
    .any(|c| c["status"].as_str().unwrap_or("").starts_with("arc:"));
  assert!(has_arc_dkim, "Expected ARC authentication result");
}

#[tokio::test]
async fn auth_results_empty_for_plain_email() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("No Auth", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/auth", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(body["dkim"].as_array().unwrap().len(), 0);
  assert_eq!(body["spf"].as_array().unwrap().len(), 0);
  assert_eq!(body["dmarc"].as_array().unwrap().len(), 0);
  assert_eq!(body["arc"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn auth_results_not_found() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages/nonexistent/auth")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

fn email_with_inline_image() -> Vec<u8> {
  let boundary = "----=_Part_12345";
  [
    "From: sender@example.com\r\n",
    "To: rcpt@example.com\r\n",
    "Subject: Inline Image Test\r\n",
    "MIME-Version: 1.0\r\n",
    &format!("Content-Type: multipart/related; boundary=\"{boundary}\"\r\n"),
    "\r\n",
    &format!("--{boundary}\r\n"),
    "Content-Type: text/html; charset=utf-8\r\n",
    "\r\n",
    "<html><body><img src=\"cid:logo@example.com\" /></body></html>\r\n",
    &format!("--{boundary}\r\n"),
    "Content-Type: image/png\r\n",
    "Content-Transfer-Encoding: base64\r\n",
    "Content-ID: <logo@example.com>\r\n",
    "Content-Disposition: inline; filename=\"logo.png\"\r\n",
    "\r\n",
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\r\n",
    &format!("--{boundary}--\r\n"),
  ]
  .concat()
  .into_bytes()
}

#[tokio::test]
async fn inline_image_by_content_id() {
  let (app, repo, _) = setup().await;
  let raw = email_with_inline_image();
  let summary = repo
    .insert("sender@example.com", &["rcpt@example.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!(
          "/api/v1/messages/{}/inline/logo@example.com",
          summary.id
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(response.headers().get("content-type").unwrap(), "image/png");
  assert_eq!(
    response.headers().get("x-content-type-options").unwrap(),
    "nosniff"
  );
  assert_eq!(
    response.headers().get("content-security-policy").unwrap(),
    "default-src 'none'"
  );
  let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
    .await
    .unwrap();
  assert!(!bytes.is_empty());
}

#[tokio::test]
async fn inline_image_not_found() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("No inline", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!(
          "/api/v1/messages/{}/inline/nonexistent@example.com",
          summary.id
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn starred_message_ws_event() {
  let (app, repo, ws_tx) = setup().await;
  let mut rx = ws_tx.subscribe();

  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Star me", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let _response = app
    .oneshot(
      Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"is_starred": true}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  let event = rx.try_recv().unwrap();
  match event {
    WsEvent::MessageStarred { id, is_starred } => {
      assert_eq!(id, summary.id);
      assert!(is_starred);
    }
    _ => panic!("Expected MessageStarred event, got {event:?}"),
  }
}

#[tokio::test]
async fn tags_update_ws_event() {
  let (app, repo, ws_tx) = setup().await;
  let mut rx = ws_tx.subscribe();

  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Tag me", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let _response = app
    .oneshot(
      Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/messages/{}", summary.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"tags": ["urgent"]}"#))
        .unwrap(),
    )
    .await
    .unwrap();

  let event = rx.try_recv().unwrap();
  match event {
    WsEvent::MessageTags { id, tags } => {
      assert_eq!(id, summary.id);
      assert_eq!(tags, vec!["urgent"]);
    }
    _ => panic!("Expected MessageTags event, got {event:?}"),
  }
}
