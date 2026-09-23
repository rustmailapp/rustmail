//! An in-process RustMail API over the stored corpus, and a transcript that
//! records every exchange with it.
//!
//! The repository is an in-memory SQLite database on one permanent
//! connection, the way `--ephemeral` runs it, and every request goes through
//! the full [`router`] with its middleware.

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use rustmail_api::{AppState, WsFrame, router};
use rustmail_storage::{MessageRepository, MessageSummary, connect_options, initialize_database};
use tokio::sync::broadcast;
use tower::ServiceExt;

use crate::corpus::{CorpusMessage, corpus};
use crate::golden::{KnownBlob, Normalizer, render_body};

const IN_MEMORY_DB_URL: &str = "sqlite::memory:";
const WS_CHANNEL_CAPACITY: usize = 256;

/// A corpus message as stored, with the ids the store minted for it.
pub struct Stored {
  pub message: &'static CorpusMessage,
  pub summary: MessageSummary,
  pub attachment_ids: Vec<String>,
}

/// The router, its state and the stored corpus.
pub struct Fixture {
  pub app: Router,
  pub state: AppState,
  pub repo: MessageRepository,
  pub stored: Vec<Stored>,
  pub normalizer: Normalizer,
}

impl Fixture {
  /// The stored message built by the corpus entry `name`.
  pub fn stored(&self, name: &str) -> &Stored {
    self
      .stored
      .iter()
      .find(|stored| stored.message.name == name)
      .unwrap_or_else(|| panic!("no corpus message named {name}"))
  }

  /// The id the store minted for the corpus entry `name`.
  pub fn id(&self, name: &str) -> &str {
    &self.stored(name).summary.id
  }

  /// Every raw message and payload, labelled, for naming response bodies.
  pub fn known_blobs(&self) -> Vec<KnownBlob<'static>> {
    let mut blobs = Vec::new();
    for message in corpus() {
      blobs.push(KnownBlob {
        label: format!("raw:{}", message.name),
        bytes: &message.raw,
      });
      for payload in &message.payloads {
        blobs.push(KnownBlob {
          label: format!("payload:{}/{}", message.name, payload.label),
          bytes: &payload.bytes,
        });
      }
    }
    blobs
  }

  /// A transcript of exchanges with this fixture's router.
  pub fn transcript(&self) -> Transcript<'_> {
    Transcript {
      fixture: self,
      known: self.known_blobs(),
      out: String::new(),
    }
  }
}

/// A router over the stored corpus, with the default state.
pub async fn fixture() -> Fixture {
  fixture_with(|state| state).await
}

/// A router over the stored corpus, with its state adjusted by `configure`.
pub async fn fixture_with(configure: impl FnOnce(AppState) -> AppState) -> Fixture {
  let pool = sqlx::sqlite::SqlitePoolOptions::new()
    .min_connections(1)
    .max_connections(1)
    .idle_timeout(None)
    .max_lifetime(None)
    .connect_with(connect_options(IN_MEMORY_DB_URL).unwrap())
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool);
  let (ws_tx, _) = broadcast::channel::<WsFrame>(WS_CHANNEL_CAPACITY);
  let state = configure(AppState::new(repo.clone(), ws_tx, None, None));

  let mut normalizer = Normalizer::default();
  let mut stored = Vec::new();
  for message in corpus() {
    let summary = repo
      .insert(&message.sender, &message.recipients, &message.raw)
      .await
      .unwrap();
    apply_state(&repo, message, &summary.id).await;
    let attachment_ids: Vec<String> = repo
      .get_attachments(&summary.id)
      .await
      .unwrap()
      .into_iter()
      .map(|attachment| attachment.id)
      .collect();
    normalizer.map(&summary.id, format!("{{msg:{}}}", message.name));
    for (index, id) in attachment_ids.iter().enumerate() {
      normalizer.map(id, format!("{{att:{}/{index}}}", message.name));
    }
    stored.push(Stored {
      message,
      summary,
      attachment_ids,
    });
  }

  Fixture {
    app: router(state.clone()),
    state,
    repo,
    stored,
    normalizer,
  }
}

async fn apply_state(repo: &MessageRepository, message: &CorpusMessage, id: &str) {
  let state = &message.state;
  let tags: Vec<String> = state.tags.iter().map(ToString::to_string).collect();
  if !state.read && !state.starred && tags.is_empty() {
    return;
  }
  repo
    .update_message(
      id,
      state.read.then_some(true),
      state.starred.then_some(true),
      (!tags.is_empty()).then_some(tags.as_slice()),
    )
    .await
    .unwrap();
}

/// A request to record: method, URI, headers and body.
pub struct Req {
  method: Method,
  uri: String,
  headers: Vec<(header::HeaderName, String)>,
  body: Vec<u8>,
}

impl Req {
  pub fn new(method: Method, uri: impl Into<String>) -> Self {
    Self {
      method,
      uri: uri.into(),
      headers: Vec::new(),
      body: Vec::new(),
    }
  }

  pub fn get(uri: impl Into<String>) -> Self {
    Self::new(Method::GET, uri)
  }

  pub fn header(mut self, name: header::HeaderName, value: impl Into<String>) -> Self {
    self.headers.push((name, value.into()));
    self
  }

  /// Sends `body` as `application/json`.
  pub fn json(self, body: &str) -> Self {
    self
      .header(header::CONTENT_TYPE, "application/json")
      .body(body)
  }

  pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
    self.body = body.into();
    self
  }
}

/// A response as the client received it.
pub struct Captured {
  pub status: StatusCode,
  pub body: Bytes,
}

/// An ordered text record of exchanges, compared against a snapshot.
pub struct Transcript<'a> {
  fixture: &'a Fixture,
  known: Vec<KnownBlob<'static>>,
  out: String,
}

impl Transcript<'_> {
  /// Adds a heading, so a snapshot reads as a list of cases.
  pub fn section(&mut self, title: &str) {
    self.out.push_str(&format!("## {title}\n\n"));
  }

  /// Adds a normalized line of free text.
  pub fn note(&mut self, text: &str) {
    self.out.push_str(&self.fixture.normalizer.apply(text));
    self.out.push('\n');
  }

  pub async fn get(&mut self, uri: impl Into<String>) -> Captured {
    self.send(Req::get(uri)).await
  }

  /// Sends `request` through the router and records it with its response.
  pub async fn send(&mut self, request: Req) -> Captured {
    let captured = self.exchange(&request, true).await;
    self.out.push('\n');
    captured
  }

  /// Like [`Self::send`], but records the response headers only.
  pub async fn send_headers_only(&mut self, request: Req) -> Captured {
    let captured = self.exchange(&request, false).await;
    self.out.push('\n');
    captured
  }

  async fn exchange(&mut self, request: &Req, with_body: bool) -> Captured {
    let normalizer = &self.fixture.normalizer;
    self.out.push_str(&format!(
      ">>> {} {}\n",
      request.method,
      normalizer.apply(&request.uri)
    ));
    for (name, value) in &request.headers {
      self.out.push_str(&format!("> {name}: {value}\n"));
    }
    if !request.body.is_empty() {
      self.out.push_str(&format!(
        "> {}\n",
        normalizer.apply(&String::from_utf8_lossy(&request.body))
      ));
    }

    let mut builder = Request::builder()
      .method(request.method.clone())
      .uri(&request.uri);
    for (name, value) in &request.headers {
      builder = builder.header(name, value);
    }
    let response = self
      .fixture
      .app
      .clone()
      .oneshot(builder.body(Body::from(request.body.clone())).unwrap())
      .await
      .unwrap();

    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
      .await
      .unwrap();

    self.out.push_str(&format!(
      "<<< {} {}\n",
      status.as_u16(),
      status.canonical_reason().unwrap_or("")
    ));
    self.out.push_str(&render_headers(&headers, normalizer));
    if with_body {
      let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
      self
        .out
        .push_str(&render_body(&body, content_type, &self.known, normalizer));
    } else {
      self.out.push_str("body: (not rendered)\n");
    }

    Captured { status, body }
  }

  /// The recorded text.
  pub fn finish(self) -> String {
    self.out
  }
}

fn render_headers(headers: &HeaderMap, normalizer: &Normalizer) -> String {
  let mut names: Vec<&header::HeaderName> = headers.keys().collect();
  names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
  let mut out = String::new();
  for name in names {
    for value in headers.get_all(name) {
      out.push_str(&format!(
        "{name}: {}\n",
        normalizer.apply(&String::from_utf8_lossy(value.as_bytes()))
      ));
    }
  }
  out
}
