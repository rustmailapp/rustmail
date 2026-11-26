//! The contract between the router and `docs/api.yaml`.
//!
//! Every exchange a [`crate::harness::Transcript`] records is checked against
//! the spec as it happens, so each golden is also a contract test. On top of
//! that, [`every_documented_response_is_exercised`] drives one request per
//! documented status, and [`the_router_and_the_spec_list_the_same_routes`]
//! compares the routes the router declares with the operations the spec
//! documents. A response field, status, route or WebSocket event added on
//! one side only fails here; adding it to both is a spec change only.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method, StatusCode, header};
use rustmail_api::{AppState, ServerInfo, WsEvent};
use rustmail_storage::ReleaseOutcome;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

use crate::harness::{Backend, Req, fixture, fixture_with};
use crate::openapi::{Operation, spec};

const ROUTER_SOURCE: &str = include_str!("../../src/lib.rs");
const API: &str = "/api/v1";
const WS_PATH: &str = "/api/v1/ws";
const UNKNOWN_ID: &str = "01J0000000000000000000000Z";
const LOOPBACK: &str = "127.0.0.1";
const UNREACHABLE_RELAY_PORT: u16 = 465;
const FOREIGN_ORIGIN: &str = "http://foreign.example";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WS_EVENT_SCHEMA: &str = "#/components/schemas/WsEvent";
const RELEASE_PATH: &str = "/api/v1/messages/{id}/release";
const INFO_PATH: &str = "/api/v1/info";
const DELIVERED_RELEASE_ELSEWHERE: &str = "the relay tests in src/handlers.rs: the router only trusts relays whose certificate the webpki roots trust";

/// A documented response of a documented operation: its status and the
/// documented content type it matched, if it has a body.
pub type Covered = (Operation, u16, Option<String>);

/// Checks one response the router gave against the spec.
///
/// Returns what it covered, or `None` for a request the spec does not
/// document an operation for (static files, unknown API paths, `HEAD`).
/// Panics when the response contradicts the spec.
pub fn observe(
  method: &Method,
  uri: &str,
  status: StatusCode,
  headers: &HeaderMap,
  body: &[u8],
) -> Option<Covered> {
  let path = uri.split('?').next().unwrap_or(uri);
  let operation = spec().operation_for(method.as_str(), path)?;
  let content_type = headers
    .get(header::CONTENT_TYPE)
    .and_then(|value| value.to_str().ok());
  let decoded_body = (!headers.contains_key(header::CONTENT_ENCODING)).then_some(body);
  match spec().check_response(&operation, status.as_u16(), content_type, decoded_body) {
    Ok(media_type) => Some((operation, status.as_u16(), media_type)),
    Err(error) => panic!("{method} {uri} answered against docs/api.yaml: {error}"),
  }
}

/// Checks one WebSocket text frame against the `WsEvent` schema.
pub fn check_ws_frame(frame: &str) {
  let event: Value = serde_json::from_str(frame)
    .unwrap_or_else(|error| panic!("WebSocket frame {frame} is not JSON: {error}"));
  if let Err(error) = spec().validate_against(WS_EVENT_SCHEMA, &event) {
    panic!("WebSocket frame {frame} contradicts docs/api.yaml: {error}");
  }
}

/// Where a documented response the router cannot produce in-process is
/// checked against the spec instead.
fn checked_elsewhere(operation: &Operation, status: u16) -> Option<&'static str> {
  match (operation.method.as_str(), operation.path.as_str(), status) {
    ("POST", RELEASE_PATH, 200 | 504) => Some(DELIVERED_RELEASE_ELSEWHERE),
    ("GET", INFO_PATH, 503) => {
      Some("nowhere: info does no I/O, so it cannot outlast the read timeout")
    }
    ("GET", path, 503) if path != WS_PATH => Some(
      "the timeout tests in src/lib.rs: the read timeout is not configurable from outside the crate",
    ),
    _ => None,
  }
}

/// Every `.route(path, method(handler))` in [`rustmail_api::router`], under
/// the prefix it is nested at.
fn routed_operations() -> BTreeSet<Operation> {
  let source: String = ROUTER_SOURCE.split_whitespace().collect();
  let router = function_body(&source, "pubfnrouter(");
  for unread in [".route_service(", ".merge(", ".nest_service("] {
    assert!(
      !router.contains(unread),
      "the router uses {unread}, which this scan does not read"
    );
  }
  assert_eq!(
    router.matches(".route(").count(),
    router.matches(".route(\"").count(),
    "a route path is not a string literal, which this scan does not read"
  );
  let prefix = router
    .split(".nest(\"")
    .nth(1)
    .and_then(|rest| rest.split('"').next())
    .expect("the router nests the API under a prefix");
  router
    .split(".route(\"")
    .skip(1)
    .map(|route| {
      let (path, call) = route.split_once("\",").expect("a route is a literal path");
      let (method, handler_and_rest) = call.split_once('(').expect("a route names a method");
      let (_, rest) = handler_and_rest
        .split_once(')')
        .expect("a route names a handler");
      assert!(
        rest.trim_start_matches(',').starts_with(')'),
        "route {path} chains methods, which this scan does not read: {method}({handler_and_rest}"
      );
      Operation {
        method: method.to_uppercase(),
        path: format!("{prefix}{path}"),
      }
    })
    .collect()
}

/// The body of the function whose signature starts with `signature`, in
/// whitespace-free `source`.
fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
  let start = source
    .find(signature)
    .and_then(|at| source[at..].find('{').map(|open| at + open))
    .unwrap_or_else(|| panic!("src/lib.rs has no {signature}"));
  let mut depth = 0usize;
  for (offset, byte) in source[start..].bytes().enumerate() {
    match byte {
      b'{' => depth += 1,
      b'}' => {
        depth -= 1;
        if depth == 0 {
          return &source[start..=start + offset];
        }
      }
      _ => {}
    }
  }
  panic!("{signature} has no closing brace");
}

#[test]
fn the_router_and_the_spec_list_the_same_routes() {
  let routed = routed_operations();
  let documented = spec().operations();
  let undocumented: Vec<String> = routed
    .difference(&documented)
    .map(ToString::to_string)
    .collect();
  let unrouted: Vec<String> = documented
    .difference(&routed)
    .map(ToString::to_string)
    .collect();
  assert!(
    undocumented.is_empty() && unrouted.is_empty(),
    "routed but not in docs/api.yaml: {undocumented:?}; in docs/api.yaml but not routed: {unrouted:?}"
  );
}

#[tokio::test]
async fn every_documented_response_is_exercised() {
  let mut covered = BTreeSet::new();
  covered.extend(message_responses().await);
  covered.extend(release_responses().await);
  covered.extend(info_responses().await);
  covered.extend(websocket_responses().await);

  let missing: Vec<String> = spec()
    .operations()
    .into_iter()
    .flat_map(|operation| {
      spec()
        .responses(&operation)
        .into_iter()
        .map(move |(status, media_type)| (operation.clone(), status, media_type))
    })
    .filter(|response| {
      !covered.contains(response) && checked_elsewhere(&response.0, response.1).is_none()
    })
    .map(|(operation, status, media_type)| {
      format!("{operation} {status} {}", media_type.unwrap_or_default())
    })
    .collect();
  assert!(
    missing.is_empty(),
    "documented but never exercised: {missing:?}"
  );
}

async fn message_responses() -> BTreeSet<Covered> {
  let fx = fixture(Backend::Fresh).await;
  let welcome = fx.id("welcome").to_string();
  let with_files = fx.stored("nested_related");
  let files_id = with_files.summary.id.clone();
  let attachment = with_files.attachment_ids[0].clone();
  let message = format!("{API}/messages/{welcome}");
  let unknown = format!("{API}/messages/{UNKNOWN_ID}");
  let release = fx
    .repo
    .record_release(&welcome, ReleaseOutcome::Sent)
    .await
    .unwrap();
  check_ws_frame(
    &serde_json::to_string(&WsEvent::MessageReleased {
      id: welcome.clone(),
      released_at: release.released_at,
      release_outcome: release.release_outcome,
    })
    .unwrap(),
  );
  check_ws_frame(
    &serde_json::to_string(&WsEvent::MessagesDelete {
      ids: vec![welcome.clone(), files_id.clone()],
    })
    .unwrap(),
  );
  let mut t = fx.transcript();

  for uri in [
    format!("{API}/messages"),
    format!("{API}/messages?limit=abc"),
    message.clone(),
    unknown.clone(),
    format!("{message}/raw"),
    format!("{message}/raw?limit=0"),
    format!("{unknown}/raw"),
    format!("{message}/headers"),
    format!("{unknown}/headers"),
    format!("{API}/messages/{files_id}/attachments"),
    format!("{API}/messages/{files_id}/attachments/{attachment}"),
    format!("{API}/messages/{files_id}/attachments/{UNKNOWN_ID}"),
    format!("{API}/messages/{files_id}/inline/logo@corpus.test"),
    format!("{API}/messages/{files_id}/inline/spec@corpus.test"),
    format!("{API}/messages/{files_id}/inline/missing@corpus.test"),
    format!("{message}/export"),
    format!("{message}/export?format=json"),
    format!("{message}/export?format=pdf"),
    format!("{unknown}/export"),
    format!("{message}/auth"),
    format!("{unknown}/auth"),
    format!("{API}/assert/count?min=0"),
    format!("{API}/assert/count?min=1000"),
    format!("{API}/assert/count?min=abc"),
  ] {
    t.get(uri).await;
  }

  for request in [
    Req::new(Method::PATCH, &message).json(r#"{"is_read":true}"#),
    Req::new(Method::PATCH, &message).json(r#"{"tags":[""]}"#),
    Req::new(Method::PATCH, &unknown).json(r#"{"is_read":true}"#),
    Req::new(Method::PATCH, &message).body(r#"{"is_read":true}"#),
    Req::new(Method::PATCH, &message).json(r#"{"is_read":"yes"}"#),
    Req::new(Method::PATCH, &message).json("not json"),
    Req::new(Method::DELETE, &message),
    Req::new(Method::DELETE, &message),
    Req::new(Method::DELETE, format!("{API}/messages")),
  ] {
    t.send(request).await;
  }
  t.covered().clone()
}

async fn release_responses() -> BTreeSet<Covered> {
  let mut covered = BTreeSet::new();

  let disabled = fixture(Backend::Fresh).await;
  let release = format!("{API}/messages/{}/release", disabled.id("welcome"));
  let mut t = disabled.transcript();
  t.send(Req::new(Method::POST, &release).json(r#"{"host":"127.0.0.1"}"#))
    .await;
  covered.extend(t.covered().clone());

  let enabled = fixture_with(Backend::Fresh, |state| release_to(state, LOOPBACK)).await;
  let release = format!("{API}/messages/{}/release", enabled.id("welcome"));
  let mut t = enabled.transcript();
  for request in [
    Req::new(Method::POST, &release).json(r#"{"host":"127.0.0.1","port":1025}"#),
    Req::new(Method::POST, format!("{API}/messages/{UNKNOWN_ID}/release"))
      .json(r#"{"host":"127.0.0.1"}"#),
    Req::new(Method::POST, &release).body(r#"{"host":"127.0.0.1"}"#),
    Req::new(Method::POST, &release).json(r#"{"host":5}"#),
    Req::new(Method::POST, &release).json("not json"),
    Req::new(Method::POST, &release).json(&format!(
      r#"{{"host":"127.0.0.1","port":{UNREACHABLE_RELAY_PORT}}}"#
    )),
  ] {
    t.send(request).await;
  }
  covered.extend(t.covered().clone());
  covered
}

async fn info_responses() -> BTreeSet<Covered> {
  let mut covered = BTreeSet::new();
  let configurations: [fn(AppState) -> AppState; 2] =
    [with_info, |state| release_to(with_info(state), LOOPBACK)];
  for configure in configurations {
    let fx = fixture_with(Backend::Fresh, configure).await;
    let mut t = fx.transcript();
    t.get(format!("{API}/info")).await;
    covered.extend(t.covered().clone());
  }
  covered
}

async fn websocket_responses() -> BTreeSet<Covered> {
  let open = fixture(Backend::Fresh).await;
  let full = fixture_with(Backend::Fresh, |mut state| {
    state.ws_semaphore = Arc::new(Semaphore::new(0));
    state
  })
  .await;
  let mut covered = BTreeSet::new();
  for (app, origin) in [
    (open.app.clone(), None),
    (open.app.clone(), Some(FOREIGN_ORIGIN)),
    (full.app.clone(), None),
  ] {
    let listener = tokio::net::TcpListener::bind((LOOPBACK, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let (status, headers, body) =
      tokio::time::timeout(HANDSHAKE_TIMEOUT, ws_handshake(addr, origin))
        .await
        .expect("the WebSocket handshake got no answer");
    server.abort();
    covered.extend(observe(&Method::GET, WS_PATH, status, &headers, &body));
  }
  covered
}

/// Opens the WebSocket handshake and reads the response head, plus the
/// body when the handshake was refused.
async fn ws_handshake(addr: SocketAddr, origin: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
  let mut stream = BufReader::new(TcpStream::connect(addr).await.unwrap());
  let origin_line = origin
    .map(|origin| format!("Origin: {origin}\r\n"))
    .unwrap_or_default();
  let request = format!(
    "GET {WS_PATH} HTTP/1.1\r\nHost: {addr}\r\n{origin_line}Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
  );
  stream
    .get_mut()
    .write_all(request.as_bytes())
    .await
    .unwrap();

  let mut status_line = String::new();
  stream.read_line(&mut status_line).await.unwrap();
  let status = status_line
    .split_whitespace()
    .nth(1)
    .and_then(|code| code.parse::<u16>().ok())
    .and_then(|code| StatusCode::from_u16(code).ok())
    .unwrap_or_else(|| panic!("not an HTTP status line: {status_line:?}"));
  let mut headers = HeaderMap::new();
  loop {
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    let Some((name, value)) = line.trim_end().split_once(':') else {
      break;
    };
    headers.append(
      header::HeaderName::from_bytes(name.trim().as_bytes()).unwrap(),
      value.trim().parse().unwrap(),
    );
  }
  let length = headers
    .get(header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok()?.parse::<usize>().ok())
    .unwrap_or(0);
  let mut body = vec![0; length];
  if status != StatusCode::SWITCHING_PROTOCOLS {
    stream.read_exact(&mut body).await.unwrap();
  }
  (status, headers, body)
}

fn with_info(state: AppState) -> AppState {
  state.with_server_info(ServerInfo {
    version: "0.0.0-contract".to_string(),
    smtp_port: 1025,
  })
}

fn release_to(mut state: AppState, host: &str) -> AppState {
  state.release_host = Some(host.to_string());
  state
}
