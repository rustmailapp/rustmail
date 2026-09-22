use axum::body::Body;
use axum::http::{Request, StatusCode};
use rustmail_api::{AppState, Hostname, Origin, WsEvent, WsFrame, router};
use rustmail_storage::{MessageRepository, initialize_database};
use serde_json::Value;
use tokio::sync::broadcast;
use tower::ServiceExt;

async fn setup() -> (axum::Router, MessageRepository, broadcast::Sender<WsFrame>) {
  setup_with(Vec::new(), Vec::new()).await
}

async fn setup_with_allowed_hosts(
  hosts: Vec<Hostname>,
) -> (axum::Router, MessageRepository, broadcast::Sender<WsFrame>) {
  setup_with(Vec::new(), hosts).await
}

async fn setup_with_allowed_origins(
  origins: Vec<Origin>,
) -> (axum::Router, MessageRepository, broadcast::Sender<WsFrame>) {
  setup_with(origins, Vec::new()).await
}

async fn setup_with(
  origins: Vec<Origin>,
  hosts: Vec<Hostname>,
) -> (axum::Router, MessageRepository, broadcast::Sender<WsFrame>) {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();

  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);
  let state = AppState::new(repo.clone(), ws_tx.clone(), None, None)
    .with_allowed_origins(origins)
    .with_allowed_hosts(hosts);
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
  let (ws_tx, _) = broadcast::channel::<WsFrame>(256);
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

  let event = rx.try_recv().unwrap().decode().unwrap();
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

  let event = rx.try_recv().unwrap().decode().unwrap();
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

  let event = rx.try_recv().unwrap().decode().unwrap();
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

  let event = rx.try_recv().unwrap().decode().unwrap();
  match event {
    WsEvent::MessageTags { id, tags } => {
      assert_eq!(id, summary.id);
      assert_eq!(tags, vec!["urgent"]);
    }
    _ => panic!("Expected MessageTags event, got {event:?}"),
  }
}

fn email_with_folded_and_repeated_headers() -> Vec<u8> {
  concat!(
    "Received: from a.example.com by b.example.com;\r\n",
    " Tue, 1 Jul 2025 10:00:00 +0000\r\n",
    "Received: from c.example.com by d.example.com;\r\n",
    " Tue, 1 Jul 2025 10:00:01 +0000\r\n",
    "From: sender@example.com\r\n",
    "To: rcpt@example.com\r\n",
    "Subject: Folded header test\r\n",
    "X-Long: first part\r\n\tsecond part\r\n",
    "Content-Type: text/plain\r\n",
    "\r\n",
    "Body that must never reach the headers endpoint.\r\n",
  )
  .as_bytes()
  .to_vec()
}

#[tokio::test]
async fn headers_endpoint_returns_fields_in_wire_order() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "sender@example.com",
      &["rcpt@example.com".into()],
      &email_with_folded_and_repeated_headers(),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/headers", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  let headers = body.as_array().unwrap();

  let names: Vec<&str> = headers
    .iter()
    .map(|h| h["name"].as_str().unwrap())
    .collect();
  assert_eq!(
    names,
    vec![
      "Received",
      "Received",
      "From",
      "To",
      "Subject",
      "X-Long",
      "Content-Type"
    ],
    "duplicates must be preserved and order must match the wire"
  );

  let subject = headers.iter().find(|h| h["name"] == "Subject").unwrap();
  assert_eq!(subject["value"], "Folded header test");
}

#[tokio::test]
async fn headers_endpoint_unfolds_continuation_lines() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "sender@example.com",
      &["rcpt@example.com".into()],
      &email_with_folded_and_repeated_headers(),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/headers", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  let body = json_body(response).await;
  let headers = body.as_array().unwrap();

  let long = headers.iter().find(|h| h["name"] == "X-Long").unwrap();
  assert_eq!(long["value"], "first part second part");

  let first_received = headers.iter().find(|h| h["name"] == "Received").unwrap();
  assert_eq!(
    first_received["value"],
    "from a.example.com by b.example.com; Tue, 1 Jul 2025 10:00:00 +0000"
  );
}

#[tokio::test]
async fn headers_endpoint_omits_the_body() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "sender@example.com",
      &["rcpt@example.com".into()],
      &email_with_folded_and_repeated_headers(),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/headers", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
    .await
    .unwrap();
  let text = std::str::from_utf8(&bytes).unwrap();
  assert!(
    !text.contains("must never reach"),
    "the body leaked into the headers response"
  );
}

#[tokio::test]
async fn headers_endpoint_unknown_message_returns_404() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages/nonexistent/headers")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

async fn cache_control_of(app: axum::Router, uri: String) -> String {
  let response = app
    .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
    .await
    .unwrap();
  assert_eq!(response.status(), StatusCode::OK);
  response
    .headers()
    .get(axum::http::header::CACHE_CONTROL)
    .map(|v| v.to_str().unwrap().to_string())
    .unwrap_or_default()
}

#[tokio::test]
async fn message_derived_resources_are_cacheable() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Cacheable", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  for suffix in ["/raw", "/headers", "/auth"] {
    let value = cache_control_of(
      app.clone(),
      format!("/api/v1/messages/{}{}", summary.id, suffix),
    )
    .await;
    assert!(
      value.contains("immutable") && value.contains("private"),
      "{suffix} should be privately cacheable forever, got {value:?}"
    );
  }
}

#[tokio::test]
async fn mutable_message_metadata_is_not_cached() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Mutable", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  // is_read, is_starred and tags change over the message's life, so the
  // single-message and list endpoints must never be served from cache.
  for uri in [
    format!("/api/v1/messages/{}", summary.id),
    "/api/v1/messages".to_string(),
  ] {
    let value = cache_control_of(app.clone(), uri.clone()).await;
    assert_eq!(
      value, "no-store",
      "{uri} must explicitly refuse caching, got {value:?}"
    );
  }
}

fn email_with_latin1_subject() -> Vec<u8> {
  // Raw 8-bit bytes in a header, i.e. not MIME-encoded: 0xE8 is `è` in Latin-1
  // and is not valid UTF-8. Real senders emit these.
  let mut raw = b"From: sender@example.com\r\nSubject: caff".to_vec();
  raw.push(0xE8);
  raw.extend_from_slice(b" ricevuto\r\nTo: rcpt@example.com\r\n\r\nbody\r\n");
  raw
}

#[tokio::test]
async fn headers_endpoint_survives_non_utf8_header_bytes() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "sender@example.com",
      &["rcpt@example.com".into()],
      &email_with_latin1_subject(),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/headers", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  let headers = body.as_array().unwrap();

  let subject = headers
    .iter()
    .find(|h| h["name"] == "Subject")
    .expect("Subject must still be listed");
  let value = subject["value"].as_str().unwrap();
  assert!(
    value.starts_with("caff") && value.ends_with("ricevuto"),
    "undecodable bytes must be replaced, not truncate the value: {value:?}"
  );
}

/// Preview cap the UI asks for; small here so the test message can exceed it.
const RAW_PREVIEW_BYTES: usize = 64;

#[tokio::test]
async fn raw_message_limit_returns_only_the_requested_prefix() {
  let (app, repo, _) = setup().await;
  let raw = raw_email(&"A".repeat(512), "a@t.com", "b@t.com");
  assert!(raw.len() > RAW_PREVIEW_BYTES);
  let summary = repo
    .insert("a@t.com", &["b@t.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!(
          "/api/v1/messages/{}/raw?limit={RAW_PREVIEW_BYTES}",
          summary.id
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
    .await
    .unwrap();
  assert_eq!(bytes.len(), RAW_PREVIEW_BYTES);
  assert_eq!(bytes.as_ref(), &raw[..RAW_PREVIEW_BYTES]);
}

#[tokio::test]
async fn raw_message_rejects_a_non_positive_limit() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Limit", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/raw?limit=0", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn raw_message_rejects_a_malformed_limit_in_the_same_shape() {
  let (app, repo, _) = setup().await;
  let summary = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &raw_email("Limit", "a@t.com", "b@t.com"),
    )
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/raw?limit=abc", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  let body = json_body(response).await;
  assert!(
    body.get("error").is_some(),
    "a malformed limit must fail in the same JSON shape as an invalid one, got: {body}"
  );
}

#[tokio::test]
async fn unknown_api_path_is_a_json_404_not_the_spa_shell() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/there-is-no-such-endpoint")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::NOT_FOUND);
  let body = json_body(response).await;
  assert!(
    body.get("error").is_some(),
    "an API 404 must carry a JSON error, got: {body}"
  );
}

#[tokio::test]
async fn header_endpoint_reads_headers_longer_than_the_prefix_window() {
  let (app, repo, _) = setup().await;

  // A header section far past the 64 KiB prefix read, so the handler has to
  // notice the prefix was truncated and fall back to the whole message.
  let padding: String = (0..4000)
    .map(|i| format!("X-Pad-{i}: {}\r\n", "y".repeat(64)))
    .collect();
  let raw = format!(
    "From: a@t.com\r\nTo: b@t.com\r\nSubject: Long\r\n{padding}X-Last: sentinel\r\n\r\nbody"
  )
  .into_bytes();
  assert!(raw.len() > 64 * 1024);

  let summary = repo
    .insert("a@t.com", &["b@t.com".into()], &raw)
    .await
    .unwrap();

  let response = app
    .oneshot(
      Request::builder()
        .uri(format!("/api/v1/messages/{}/headers", summary.id))
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  let names: Vec<&str> = body
    .as_array()
    .unwrap()
    .iter()
    .map(|h| h["name"].as_str().unwrap())
    .collect();
  assert!(
    names.contains(&"X-Last"),
    "a header past the prefix window was dropped"
  );
}

const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const SWITCHING_PROTOCOLS: u16 = 101;
const FORBIDDEN: u16 = 403;

/// Serves `app` on a loopback port, since a WebSocket handshake only reaches
/// its verdict over a real connection.
async fn serve_router(app: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let server = tokio::spawn(async move {
    let _ = axum::serve(listener, app).await;
  });

  (addr, server)
}

/// Opens a handshake against `/api/v1/ws` and reads back its status code.
async fn ws_handshake_status(addr: std::net::SocketAddr, host: &str, origin: Option<&str>) -> u16 {
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

  let origin_header = origin
    .map(|o| format!("Origin: {o}\r\n"))
    .unwrap_or_default();
  let request = format!(
    "GET /api/v1/ws HTTP/1.1\r\n\
     Host: {host}\r\n\
     Connection: Upgrade\r\n\
     Upgrade: websocket\r\n\
     Sec-WebSocket-Version: 13\r\n\
     Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
     {origin_header}\r\n"
  );

  let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
  stream.write_all(request.as_bytes()).await.unwrap();

  let mut status_line = String::new();
  tokio::time::timeout(
    HANDSHAKE_TIMEOUT,
    BufReader::new(stream).read_line(&mut status_line),
  )
  .await
  .expect("the server left the handshake unanswered")
  .unwrap();

  status_line
    .split_whitespace()
    .nth(1)
    .and_then(|code| code.parse().ok())
    .unwrap_or_else(|| panic!("not an HTTP status line: {status_line:?}"))
}

#[tokio::test]
async fn ws_handshake_without_an_origin_upgrades() {
  let (app, _, _) = setup().await;
  let (addr, server) = serve_router(app).await;

  let status = ws_handshake_status(addr, &addr.to_string(), None).await;
  server.abort();

  assert_eq!(
    status, SWITCHING_PROTOCOLS,
    "the TUI, websocat and CI clients send no Origin at all"
  );
}

#[tokio::test]
async fn ws_handshake_from_the_served_origin_upgrades() {
  let (app, _, _) = setup().await;
  let (addr, server) = serve_router(app).await;

  let status = ws_handshake_status(addr, &addr.to_string(), Some(&format!("http://{addr}"))).await;
  server.abort();

  assert_eq!(
    status, SWITCHING_PROTOCOLS,
    "the bundled UI is served from this very origin"
  );
}

#[tokio::test]
async fn ws_handshake_from_a_foreign_origin_is_refused() {
  let (app, _, _) = setup().await;
  let (addr, server) = serve_router(app).await;

  let status = ws_handshake_status(
    addr,
    &addr.to_string(),
    Some("http://untrusted.example.test"),
  )
  .await;
  server.abort();

  assert_eq!(
    status, FORBIDDEN,
    "any page the browser has open could otherwise subscribe to inbox events"
  );
}

#[tokio::test]
async fn ws_handshake_from_a_configured_origin_upgrades() {
  let (app, _, _) =
    setup_with_allowed_origins(vec!["https://mail.example.com".parse().unwrap()]).await;
  let (addr, server) = serve_router(app).await;

  let status =
    ws_handshake_status(addr, "mail.example.com", Some("https://mail.example.com")).await;
  server.abort();

  assert_eq!(
    status, SWITCHING_PROTOCOLS,
    "behind a reverse proxy the browser's origin is the proxy's public one"
  );
}

/// A browser request, as fetch metadata marks it, addressed to `host`.
fn browser_request(host: &str, uri: &str) -> Request<Body> {
  Request::builder()
    .uri(uri)
    .header("host", host)
    .header("sec-fetch-site", "same-origin")
    .header("sec-fetch-mode", "cors")
    .body(Body::empty())
    .unwrap()
}

#[tokio::test]
async fn a_browser_on_a_rebound_name_is_refused() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(browser_request("evil.example:8025", "/api/v1/messages"))
    .await
    .unwrap();

  assert_eq!(
    response.status(),
    StatusCode::FORBIDDEN,
    "a rebound name reads as same-origin to the browser, so only the Host gives it away"
  );
}

#[tokio::test]
async fn a_browser_on_the_served_address_is_answered() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(browser_request("localhost:8025", "/api/v1/messages"))
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_client_without_fetch_metadata_keeps_any_host() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages")
        .header("host", "rustmail:8025")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(
    response.status(),
    StatusCode::OK,
    "CI harnesses reach the API by Docker service name and send no fetch metadata"
  );
}

#[tokio::test]
async fn a_configured_host_is_answered() {
  let (app, _, _) = setup_with_allowed_hosts(vec!["mail.example.com".parse().unwrap()]).await;

  let response = app
    .oneshot(browser_request("mail.example.com", "/api/v1/messages"))
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn ws_handshake_on_a_rebound_name_is_refused() {
  let (app, _, _) = setup().await;
  let (addr, server) = serve_router(app).await;

  let status =
    ws_handshake_status(addr, "evil.example:8025", Some("http://evil.example:8025")).await;
  server.abort();

  assert_eq!(
    status, FORBIDDEN,
    "rebinding makes Origin and Host agree, so the origin check alone would upgrade this"
  );
}

#[tokio::test]
async fn a_browser_without_fetch_metadata_is_still_checked() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(
      Request::builder()
        .uri("/api/v1/messages")
        .header("host", "evil.example:8025")
        .header(
          "user-agent",
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/16.3 Safari/605.1.15",
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();

  assert_eq!(
    response.status(),
    StatusCode::FORBIDDEN,
    "a same-origin GET from a pre-2023 browser carries neither Sec-Fetch-* nor Origin"
  );
}

#[tokio::test]
async fn a_refused_host_still_gets_the_security_headers() {
  let (app, _, _) = setup().await;

  let response = app
    .oneshot(browser_request("evil.example:8025", "/api/v1/messages"))
    .await
    .unwrap();

  assert_eq!(response.status(), StatusCode::FORBIDDEN);
  assert_eq!(
    response.headers().get("x-content-type-options").unwrap(),
    "nosniff"
  );
  assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
}

async fn insert_numbered(repo: &MessageRepository, count: usize) -> Vec<String> {
  let mut ids = Vec::new();
  for i in 0..count {
    let stored = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
    ids.push(stored.id);
  }
  ids
}

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
  app
    .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
    .await
    .unwrap()
}

fn listed_ids(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|m| m["id"].as_str().unwrap().to_string())
    .collect()
}

#[tokio::test]
async fn before_returns_the_messages_older_than_the_cursor() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 5).await;

  let response = get(app, &format!("/api/v1/messages?before={}", ids[2])).await;

  assert_eq!(response.status(), StatusCode::OK);
  let body = json_body(response).await;
  assert_eq!(listed_ids(&body), [ids[1].clone(), ids[0].clone()]);
}

#[tokio::test]
async fn total_ignores_the_cursor() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 5).await;

  let body = json_body(get(app, &format!("/api/v1/messages?before={}", ids[2])).await).await;

  assert_eq!(body["total"], 5);
}

#[tokio::test]
async fn next_cursor_names_the_last_message_when_more_remain() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 5).await;

  let body = json_body(get(app, "/api/v1/messages?limit=2").await).await;

  assert_eq!(body["next_cursor"], ids[3].as_str());
}

#[tokio::test]
async fn next_cursor_is_null_on_the_last_page() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 4).await;

  let body =
    json_body(get(app, &format!("/api/v1/messages?limit=2&before={}", ids[2])).await).await;

  assert_eq!(listed_ids(&body), [ids[1].clone(), ids[0].clone()]);
  assert!(body["next_cursor"].is_null());
}

#[tokio::test]
async fn following_next_cursor_walks_every_message_once() {
  let (app, repo, _) = setup().await;
  let mut ids = insert_numbered(&repo, 7).await;
  ids.reverse();

  let mut seen = Vec::new();
  let mut uri = "/api/v1/messages?limit=3".to_string();
  loop {
    let body = json_body(get(app.clone(), &uri).await).await;
    seen.extend(listed_ids(&body));
    match body["next_cursor"].as_str() {
      Some(cursor) => uri = format!("/api/v1/messages?limit=3&before={cursor}"),
      None => break,
    }
  }

  assert_eq!(seen, ids);
}

#[tokio::test]
async fn before_pages_search_results() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 4).await;

  let body =
    json_body(get(app, &format!("/api/v1/messages?q=hello&before={}", ids[3])).await).await;

  assert_eq!(
    listed_ids(&body),
    [ids[2].clone(), ids[1].clone(), ids[0].clone()]
  );
}

#[tokio::test]
async fn an_unknown_cursor_is_a_bad_request() {
  let (app, _, _) = setup().await;

  let response = get(app, "/api/v1/messages?before=01ARZ3NDEKTSV4RRFFQ69G5FAV").await;

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert!(json_body(response).await["error"].is_string());
}

#[tokio::test]
async fn before_and_offset_together_are_a_bad_request() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 3).await;

  let response = get(app, &format!("/api/v1/messages?offset=1&before={}", ids[2])).await;

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert!(json_body(response).await["error"].is_string());
}

#[tokio::test]
async fn the_response_reports_the_clamped_limit() {
  let (app, _, _) = setup().await;

  let body = json_body(get(app, "/api/v1/messages?limit=1000").await).await;

  assert_eq!(body["limit"], 200);
}

async fn star(repo: &MessageRepository, id: &str) {
  repo
    .update_message(id, None, Some(true), None)
    .await
    .unwrap();
}

async fn tag(repo: &MessageRepository, id: &str, tag: &str) {
  repo
    .update_message(id, None, None, Some(&[tag.to_string()]))
    .await
    .unwrap();
}

#[tokio::test]
async fn starred_true_lists_only_starred_messages() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 4).await;
  star(&repo, &ids[1]).await;

  let body = json_body(get(app, "/api/v1/messages?starred=true").await).await;

  assert_eq!(listed_ids(&body), [ids[1].clone()]);
}

#[tokio::test]
async fn total_counts_only_messages_passing_the_filter() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 4).await;
  star(&repo, &ids[1]).await;
  star(&repo, &ids[3]).await;

  let body = json_body(get(app, "/api/v1/messages?starred=true&limit=1").await).await;

  assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn unread_true_drops_read_messages() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 2).await;
  repo
    .update_message(&ids[0], Some(true), None, None)
    .await
    .unwrap();

  let body = json_body(get(app, "/api/v1/messages?unread=true").await).await;

  assert_eq!(listed_ids(&body), [ids[1].clone()]);
}

#[tokio::test]
async fn has_attachments_true_lists_only_messages_with_attachments() {
  let (app, repo, _) = setup().await;
  insert_numbered(&repo, 1).await;
  let with_file = repo
    .insert("a@t.com", &["b@t.com".into()], &email_with_inline_image())
    .await
    .unwrap();

  let body = json_body(get(app, "/api/v1/messages?has_attachments=true").await).await;

  assert_eq!(listed_ids(&body), [with_file.id]);
}

#[tokio::test]
async fn repeated_tags_match_messages_carrying_any_of_them() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 3).await;
  tag(&repo, &ids[0], "a").await;
  tag(&repo, &ids[2], "b").await;

  let body = json_body(get(app, "/api/v1/messages?tag=a&tag=b").await).await;

  assert_eq!(listed_ids(&body), [ids[2].clone(), ids[0].clone()]);
}

#[tokio::test]
async fn filters_narrow_a_search_and_its_total() {
  let (app, repo, _) = setup().await;
  let ids = insert_numbered(&repo, 3).await;
  star(&repo, &ids[0]).await;

  let body = json_body(get(app, "/api/v1/messages?q=hello&starred=true").await).await;

  assert_eq!(listed_ids(&body), [ids[0].clone()]);
  assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn more_tags_than_a_message_can_carry_are_a_bad_request() {
  let (app, _, _) = setup().await;
  let tags: String = (0..21).map(|i| format!("&tag=t{i}")).collect();

  let response = get(app, &format!("/api/v1/messages?limit=1{tags}")).await;

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert!(json_body(response).await["error"].is_string());
}

#[tokio::test]
async fn a_malformed_filter_value_is_a_bad_request() {
  let (app, _, _) = setup().await;

  let response = get(app, "/api/v1/messages?starred=yes").await;

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);
  assert!(json_body(response).await["error"].is_string());
}

const COMPRESSIBLE_ATTACHMENT_BYTES: usize = 4096;

fn email_with_compressible_attachment() -> Vec<u8> {
  let content = "a".repeat(COMPRESSIBLE_ATTACHMENT_BYTES);
  format!(
    concat!(
      "From: a@t.com\r\n",
      "To: b@t.com\r\n",
      "Subject: Archive\r\n",
      "MIME-Version: 1.0\r\n",
      "Content-Type: multipart/mixed; boundary=\"B\"\r\n",
      "\r\n",
      "--B\r\n",
      "Content-Type: text/plain\r\n",
      "\r\n",
      "See attached\r\n",
      "--B\r\n",
      "Content-Type: application/zip\r\n",
      "Content-Disposition: attachment; filename=\"archive.zip\"\r\n",
      "\r\n",
      "{}\r\n",
      "--B--\r\n",
    ),
    content
  )
  .into_bytes()
}

async fn get_gzip(app: axum::Router, uri: &str) -> axum::response::Response {
  app
    .oneshot(
      Request::builder()
        .uri(uri)
        .header("accept-encoding", "gzip")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn attachment_downloads_are_not_gzipped() {
  let (app, repo, _) = setup().await;
  let message = repo
    .insert(
      "a@t.com",
      &["b@t.com".into()],
      &email_with_compressible_attachment(),
    )
    .await
    .unwrap();
  let attachment = repo.get_attachments(&message.id).await.unwrap().remove(0);

  let response = get_gzip(
    app,
    &format!(
      "/api/v1/messages/{}/attachments/{}",
      message.id, attachment.id
    ),
  )
  .await;

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.headers()["content-type"],
    "application/octet-stream"
  );
  assert!(response.headers().get("content-encoding").is_none());
}

#[tokio::test]
async fn json_responses_are_still_gzipped() {
  let (app, repo, _) = setup().await;
  insert_numbered(&repo, 20).await;

  let response = get_gzip(app, "/api/v1/messages").await;

  assert_eq!(response.headers()["content-encoding"], "gzip");
}
