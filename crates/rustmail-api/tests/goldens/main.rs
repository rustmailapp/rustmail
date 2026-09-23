//! Golden snapshots of every API endpoint over a deterministic MIME corpus.
//!
//! They pin what the handlers answer today, quirks included, so a storage
//! change can prove it changed nothing observable. Snapshots live in
//! `tests/goldens/snapshots/`; regenerate them with
//! `UPDATE_GOLDENS=1 cargo test -p rustmail-api --test goldens` and review
//! the diff. See [`golden`] for what is normalized.
//!
//! Every golden that reads storage runs twice against the same snapshot:
//! `fresh::` stores the corpus through today's ingest, and `migrated::`
//! stores it through rustmail v0.7.0's write path into a legacy file, then
//! migrates that file. Identical snapshots prove the migration keeps ids,
//! cursors, search and downloads.

mod corpus;
mod golden;
mod harness;
#[path = "../../../rustmail-storage/tests/common/legacy_v0_7_0.rs"]
mod legacy_v0_7_0;
mod mime;
mod ws_wire;

use std::fmt::Write as _;

use axum::http::{Method, StatusCode, header};
use rustmail_api::{AppState, WsEvent};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::corpus::{Served, corpus};
use crate::golden::assert_golden;
use crate::harness::{Backend, Req, fixture, fixture_with};
use crate::mime::fnv1a64;
use crate::ws_wire::WsClient;

const API: &str = "/api/v1";
const UNKNOWN_ID: &str = "01J0000000000000000000000Z";
const RELEASE_HOST: &str = "127.0.0.1";
const RELEASE_PORT: u16 = 2525;
const RELAY_GREETING: &[u8] = b"220 relay.test ESMTP\r\n";
const RELAY_EHLO_REPLY: &[u8] = b"250-relay.test\r\n250 STARTTLS\r\n";
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn next_cursor(body: &[u8]) -> Option<String> {
  let value: Value = serde_json::from_slice(body).unwrap();
  value["next_cursor"].as_str().map(str::to_string)
}

fn names() -> impl Iterator<Item = &'static str> {
  corpus().iter().map(|message| message.name)
}

#[test]
fn corpus_bytes_are_pinned() {
  let mut out = String::new();
  for message in corpus() {
    let _ = writeln!(
      out,
      "{} sender={} recipients={:?} raw={} bytes fnv1a64={:016x}",
      message.name,
      message.sender,
      message.recipients,
      message.raw.len(),
      fnv1a64(&message.raw)
    );
    for payload in &message.payloads {
      let _ = writeln!(
        out,
        "  payload {} {} bytes fnv1a64={:016x} served={:?}",
        payload.label,
        payload.bytes.len(),
        fnv1a64(&payload.bytes),
        payload.served
      );
    }
  }
  assert_golden("corpus", &out);
}

async fn list_pages_filters_and_cursors(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();

  t.section("defaults and limits");
  for query in [
    "",
    "?limit=3",
    "?limit=0",
    "?limit=500",
    "?limit=abc",
    "?foo=bar&limit=2",
  ] {
    t.get(format!("{API}/messages{query}")).await;
  }

  t.section("offset");
  for query in [
    "?offset=2&limit=3",
    "?offset=-5&limit=2",
    "?offset=1000",
    "?offset=x",
  ] {
    t.get(format!("{API}/messages{query}")).await;
  }

  t.section("cursor chain, limit 7");
  let mut cursor = None;
  loop {
    let query = match &cursor {
      Some(before) => format!("?limit=7&before={before}"),
      None => "?limit=7".to_string(),
    };
    let page = t.get(format!("{API}/messages{query}")).await;
    cursor = next_cursor(&page.body);
    if cursor.is_none() {
      break;
    }
  }

  t.section("cursor errors");
  let welcome = fx.id("welcome").to_string();
  t.get(format!("{API}/messages?before={welcome}&offset=1"))
    .await;
  t.get(format!("{API}/messages?before={UNKNOWN_ID}")).await;
  t.get(format!(
    "{API}/messages?before={}&limit=2",
    fx.id("large_10mib")
  ))
  .await;

  t.section("filters");
  for query in [
    "?starred=true",
    "?starred=false&limit=5",
    "?unread=true",
    "?unread=false",
    "?has_attachments=true",
    "?has_attachments=false",
    "?starred=true&unread=true",
    "?tag=work",
    "?tag=work&tag=urgent",
    "?tag=urgent&has_attachments=true",
    "?tag=nonexistent",
    "?tag=work&limit=1",
    "?starred=maybe",
    "?unread=1",
  ] {
    t.get(format!("{API}/messages{query}")).await;
  }
  let too_many_tags: String = (0..21).map(|n| format!("&tag=t{n}")).collect();
  t.get(format!("{API}/messages?limit=1{too_many_tags}"))
    .await;

  t.section("filtered cursor chain, has_attachments, limit 4");
  let mut cursor = None;
  loop {
    let query = match &cursor {
      Some(before) => format!("?has_attachments=true&limit=4&before={before}"),
      None => "?has_attachments=true&limit=4".to_string(),
    };
    let page = t.get(format!("{API}/messages{query}")).await;
    cursor = next_cursor(&page.body);
    if cursor.is_none() {
      break;
    }
  }

  assert_golden("list", &t.finish());
}

async fn search_ids_order_and_totals(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();

  t.section("terms");
  for q in [
    "planning",
    "welcome",
    "alice@example.com",
    "bob@example.test",
    "carol",
    "receipt",
    "4471",
    "Caf%C3%A9",
    "attached",
    "base64",
    "digest",
    "forwarded",
    "Discount%20100%25",
    "off_now",
    "folded",
    "zzzz-no-match",
  ] {
    t.get(format!("{API}/messages?q={q}")).await;
  }

  t.section("sanitized to nothing");
  for q in ["", "%21%21%21", "%22%2A%28%29"] {
    t.get(format!("{API}/messages?q={q}")).await;
  }

  t.section("search with filters");
  for query in [
    "q=planning&starred=true",
    "q=planning&unread=true",
    "q=attached&has_attachments=true",
    "q=attached&tag=work",
    "q=Invoice&tag=urgent&starred=true",
    "q=planning&offset=1&limit=1",
  ] {
    t.get(format!("{API}/messages?{query}")).await;
  }

  t.section("search cursor chain, limit 3");
  let mut cursor = None;
  loop {
    let query = match &cursor {
      Some(before) => format!("?q=attached&limit=3&before={before}"),
      None => "?q=attached&limit=3".to_string(),
    };
    let page = t.get(format!("{API}/messages{query}")).await;
    cursor = next_cursor(&page.body);
    if cursor.is_none() {
      break;
    }
  }

  assert_golden("search", &t.finish());
}

async fn get_every_message(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  for name in names() {
    t.get(format!("{API}/messages/{}", fx.id(name))).await;
  }
  t.get(format!("{API}/messages/{UNKNOWN_ID}")).await;
  assert_golden("get", &t.finish());
}

async fn patch_updates_and_rejections(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  let id = fx.id("welcome").to_string();
  let uri = format!("{API}/messages/{id}");

  t.section("accepted updates, each followed by a read back");
  for body in [
    r#"{"is_read":false}"#,
    r#"{"is_starred":true}"#,
    r#"{"tags":["alpha","beta"]}"#,
    r#"{"is_read":true,"is_starred":false,"tags":[]}"#,
    r#"{}"#,
    r#"{"is_read":true,"unknown":1}"#,
    r#"{"tags":["dup","dup"]}"#,
  ] {
    t.send(Req::new(Method::PATCH, &uri).json(body)).await;
    t.get(&uri).await;
  }

  t.section("rejected updates");
  let too_many: Vec<String> = (0..21).map(|n| format!("\"t{n}\"")).collect();
  let too_long = format!(r#"{{"tags":["{}"]}}"#, "x".repeat(51));
  for body in [
    format!(r#"{{"tags":[{}]}}"#, too_many.join(",")),
    too_long,
    r#"{"tags":[""]}"#.to_string(),
    r#"{"tags":["tab\there"]}"#.to_string(),
    r#"{"is_read":"yes"}"#.to_string(),
    "not json".to_string(),
  ] {
    t.send(Req::new(Method::PATCH, &uri).json(&body)).await;
  }
  t.send(Req::new(Method::PATCH, &uri).body(r#"{"is_read":true}"#))
    .await;
  t.send(
    Req::new(Method::PATCH, format!("{API}/messages/{UNKNOWN_ID}")).json(r#"{"is_read":true}"#),
  )
  .await;
  t.get(&uri).await;

  t.section("filters see the update");
  t.get(format!("{API}/messages?tag=dup")).await;

  assert_golden("patch", &t.finish());
}

async fn delete_one_message(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  let id = fx.id("nested_related").to_string();

  t.send(Req::new(Method::DELETE, format!("{API}/messages/{id}")))
    .await;
  t.send(Req::new(Method::DELETE, format!("{API}/messages/{id}")))
    .await;
  t.send(Req::new(
    Method::DELETE,
    format!("{API}/messages/{UNKNOWN_ID}"),
  ))
  .await;
  t.get(format!("{API}/messages/{id}")).await;
  t.get(format!("{API}/messages/{id}/raw")).await;
  t.get(format!("{API}/messages/{id}/attachments")).await;
  t.get(format!(
    "{API}/messages/{id}/attachments/{}",
    fx.stored("nested_related").attachment_ids[0]
  ))
  .await;
  t.get(format!("{API}/messages/{id}/inline/logo@corpus.test"))
    .await;
  t.get(format!("{API}/messages?limit=3")).await;
  t.get(format!("{API}/messages?q=planning")).await;
  t.get(format!("{API}/messages?tag=urgent")).await;
  t.get(format!("{API}/messages?before={id}")).await;
  t.get(format!("{API}/assert/count?min=0")).await;

  assert_golden("delete_one", &t.finish());
}

async fn delete_all_messages(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();

  t.send(Req::new(Method::DELETE, format!("{API}/messages")))
    .await;
  t.get(format!("{API}/messages")).await;
  t.get(format!("{API}/messages?q=planning")).await;
  t.get(format!("{API}/messages/{}", fx.id("welcome"))).await;
  t.get(format!(
    "{API}/messages/{}/attachments",
    fx.id("base64_crlf76")
  ))
  .await;
  t.get(format!("{API}/assert/count?min=0")).await;
  t.send(Req::new(Method::DELETE, format!("{API}/messages")))
    .await;

  assert_golden("delete_all", &t.finish());
}

async fn raw_source_and_prefixes(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();

  t.section("full source of every message");
  for name in names() {
    let raw = t.get(format!("{API}/messages/{}/raw", fx.id(name))).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert!(
      raw.body == fx.stored(name).message.raw,
      "raw of {name} is not the captured bytes"
    );
  }

  t.section("limit");
  let welcome = fx.id("welcome").to_string();
  for limit in ["1", "10", "100000000", "0", "-1", "abc", "1.5", ""] {
    t.get(format!("{API}/messages/{welcome}/raw?limit={limit}"))
      .await;
  }
  t.get(format!(
    "{API}/messages/{}/raw?limit=64",
    fx.id("large_10mib")
  ))
  .await;
  t.get(format!(
    "{API}/messages/{}/raw?limit=40",
    fx.id("base64_lf_only")
  ))
  .await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/raw")).await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/raw?limit=5"))
    .await;

  assert_golden("raw", &t.finish());
}

async fn headers_of_every_message(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  for name in names() {
    t.get(format!("{API}/messages/{}/headers", fx.id(name)))
      .await;
  }
  t.get(format!("{API}/messages/{UNKNOWN_ID}/headers")).await;
  assert_golden("headers", &t.finish());
}

async fn auth_results_of_every_message(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  for name in names() {
    t.get(format!("{API}/messages/{}/auth", fx.id(name))).await;
  }
  t.get(format!("{API}/messages/{UNKNOWN_ID}/auth")).await;
  assert_golden("auth", &t.finish());
}

async fn attachment_lists(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  for name in names() {
    t.get(format!("{API}/messages/{}/attachments", fx.id(name)))
      .await;
  }
  t.get(format!("{API}/messages/{UNKNOWN_ID}/attachments"))
    .await;
  assert_golden("attachments", &t.finish());
}

async fn attachment_downloads(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  let mut served: Vec<(&str, Vec<u8>)> = Vec::new();

  for stored in &fx.stored {
    for attachment_id in &stored.attachment_ids {
      let download = t
        .get(format!(
          "{API}/messages/{}/attachments/{attachment_id}",
          stored.summary.id
        ))
        .await;
      served.push((stored.message.name, download.body.to_vec()));
    }
  }

  t.section("lookups that miss");
  let welcome = fx.id("welcome").to_string();
  let foreign = fx.stored("base64_crlf76").attachment_ids[0].clone();
  t.get(format!("{API}/messages/{welcome}/attachments/{foreign}"))
    .await;
  t.get(format!("{API}/messages/{welcome}/attachments/{UNKNOWN_ID}"))
    .await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/attachments/{foreign}"))
    .await;

  assert_golden("downloads", &t.finish());

  for message in corpus() {
    for payload in message
      .payloads
      .iter()
      .filter(|p| p.served == Served::Exact)
    {
      let matches = served
        .iter()
        .filter(|(name, body)| *name == message.name && *body == payload.bytes)
        .count();
      assert_eq!(
        matches, 1,
        "{}/{} must be served byte for byte by exactly one download",
        message.name, payload.label
      );
    }
  }
}

async fn inline_parts_by_content_id(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  let nested = fx.id("nested_related").to_string();

  for cid in [
    "logo@corpus.test",
    "chart@corpus.test",
    "spec@corpus.test",
    "photo@corpus.test",
    "%3Clogo@corpus.test%3E",
    "LOGO@corpus.test",
    "missing@corpus.test",
  ] {
    t.get(format!("{API}/messages/{nested}/inline/{cid}")).await;
  }
  t.get(format!(
    "{API}/messages/{}/inline/logo@corpus.test",
    fx.id("welcome")
  ))
  .await;
  t.get(format!(
    "{API}/messages/{UNKNOWN_ID}/inline/logo@corpus.test"
  ))
  .await;

  assert_golden("inline", &t.finish());
}

async fn export_as_eml_and_json(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();

  for name in names() {
    let id = fx.id(name).to_string();
    let eml = t.get(format!("{API}/messages/{id}/export")).await;
    assert!(
      eml.body == fx.stored(name).message.raw,
      "eml export of {name} is not the captured bytes"
    );
    t.get(format!("{API}/messages/{id}/export?format=json"))
      .await;
  }

  t.section("errors");
  let welcome = fx.id("welcome").to_string();
  t.get(format!("{API}/messages/{welcome}/export?format=eml"))
    .await;
  t.get(format!("{API}/messages/{welcome}/export?format=xml"))
    .await;
  t.get(format!("{API}/messages/{welcome}/export?format="))
    .await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/export")).await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/export?format=json"))
    .await;
  t.get(format!("{API}/messages/{UNKNOWN_ID}/export?format=xml"))
    .await;

  assert_golden("export", &t.finish());
}

async fn assert_count_matching(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  for query in [
    "",
    "?min=0",
    "?min=25",
    "?min=26",
    "?max=3",
    "?min=2&max=2",
    "?subject=welcome",
    "?subject=WELCOME",
    "?subject=100%25",
    "?subject=off_now",
    "?subject=off%5Fnow",
    "?subject=%25",
    "?subject=_",
    "?sender=example.com",
    "?sender=files@example.com&min=14",
    "?recipient=carol@example.test",
    "?recipient=CAROL",
    "?subject=Base64&sender=files&recipient=bob",
    "?subject=nothing-matches",
    "?subject=nothing-matches&min=0&max=0",
    "?min=abc",
  ] {
    t.get(format!("{API}/assert/count{query}")).await;
  }
  assert_golden("assert_count", &t.finish());
}

async fn release_refusals_without_a_relay(backend: Backend) {
  let disabled = fixture(backend).await;
  let welcome = disabled.id("welcome").to_string();
  let mut t = disabled.transcript();

  t.section("release disabled");
  t.send(
    Req::new(Method::POST, format!("{API}/messages/{welcome}/release"))
      .json(r#"{"host":"127.0.0.1"}"#),
  )
  .await;
  t.send(
    Req::new(Method::POST, format!("{API}/messages/{welcome}/release")).json(r#"{"port":25}"#),
  )
  .await;
  t.send(
    Req::new(Method::POST, format!("{API}/messages/{welcome}/release")).body(r#"{"host":"x"}"#),
  )
  .await;
  let mut out = t.finish();

  let pinned_port = fixture_with(backend, |state| {
    release_to(state, RELEASE_HOST, Some(RELEASE_PORT))
  })
  .await;
  let welcome = pinned_port.id("welcome").to_string();
  let mut t = pinned_port.transcript();
  t.section("release to 127.0.0.1:2525 configured");
  for body in [
    r#"{"host":"example.com"}"#,
    r#"{"host":"127.0.0.1","port":25}"#,
  ] {
    t.send(Req::new(Method::POST, format!("{API}/messages/{welcome}/release")).json(body))
      .await;
  }
  t.send(
    Req::new(Method::POST, format!("{API}/messages/{UNKNOWN_ID}/release"))
      .json(r#"{"host":"127.0.0.1"}"#),
  )
  .await;
  out.push_str(&t.finish());

  let mut any_port = fixture_with(backend, |state| release_to(state, RELEASE_HOST, None)).await;
  let no_recipients = any_port
    .repo
    .insert(
      "sender@example.com",
      &[],
      b"Subject: nobody\r\n\r\nNo recipients.",
    )
    .await
    .unwrap();
  any_port
    .normalizer
    .map(&no_recipients.id, "{msg:no_recipients}".to_string());
  let welcome = any_port.id("welcome").to_string();
  let mut t = any_port.transcript();
  t.section("release to 127.0.0.1 on any allowed port");
  for body in [
    r#"{"host":"127.0.0.1","port":1025}"#,
    r#"{"host":"127.0.0.1","port":0}"#,
  ] {
    t.send(Req::new(Method::POST, format!("{API}/messages/{welcome}/release")).json(body))
      .await;
  }
  t.note("message inserted with no envelope recipients:");
  t.send(
    Req::new(
      Method::POST,
      format!("{API}/messages/{}/release", no_recipients.id),
    )
    .json(r#"{"host":"127.0.0.1","port":2525}"#),
  )
  .await;
  out.push_str(&t.finish());

  assert_golden("release_refusals", &out);
}

async fn release_to_a_relay_that_drops_the_connection(backend: Backend) {
  let listener = tokio::net::TcpListener::bind((RELEASE_HOST, RELEASE_PORT))
    .await
    .unwrap_or_else(|error| {
      panic!("port {RELEASE_PORT} is needed for the release golden (the handler only relays to 25, 465, 587 or 2525): {error}")
    });
  let relay = tokio::spawn(async move {
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = BufReader::new(socket);
    socket.get_mut().write_all(RELAY_GREETING).await.unwrap();
    let mut verbs = Vec::new();
    let mut line = String::new();
    while socket.read_line(&mut line).await.unwrap() > 0 {
      let verb = line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
      line.clear();
      if verb == "EHLO" {
        socket.get_mut().write_all(RELAY_EHLO_REPLY).await.unwrap();
      }
      let upgrading = verb == "STARTTLS";
      verbs.push(verb);
      if upgrading {
        break;
      }
    }
    verbs
  });

  let fx = fixture_with(backend, |state| {
    release_to(state, RELEASE_HOST, Some(RELEASE_PORT))
  })
  .await;
  let mut t = fx.transcript();
  let release = tokio::time::timeout(
    RELEASE_TIMEOUT,
    t.send(
      Req::new(
        Method::POST,
        format!("{API}/messages/{}/release", fx.id("welcome")),
      )
      .json(r#"{"host":"127.0.0.1"}"#),
    ),
  )
  .await
  .expect("release did not give up on the dropped connection");
  assert_eq!(release.status, StatusCode::BAD_GATEWAY);

  let verbs = tokio::time::timeout(RELEASE_TIMEOUT, relay)
    .await
    .expect("the relay never saw a connection")
    .unwrap();
  t.note(&format!(
    "relay received in plaintext, before dropping the connection: {}",
    verbs.join(", ")
  ));

  assert_golden("release_relay_dropped", &t.finish());
}

fn release_to(mut state: AppState, host: &str, port: Option<u16>) -> AppState {
  state.release_host = Some(host.to_string());
  state.release_port = port;
  state
}

async fn compression_is_negotiated_for_json_but_not_downloads(backend: Backend) {
  let fx = fixture(backend).await;
  let mut t = fx.transcript();
  let nested = fx.stored("nested_related");
  for uri in [
    format!("{API}/messages"),
    format!("{API}/messages/{}", nested.summary.id),
    format!("{API}/messages/{}/raw", nested.summary.id),
    format!(
      "{API}/messages/{}/attachments/{}",
      nested.summary.id, nested.attachment_ids[0]
    ),
    format!(
      "{API}/messages/{}/inline/logo@corpus.test",
      nested.summary.id
    ),
    format!("{API}/messages/{}/export", nested.summary.id),
  ] {
    t.send_headers_only(Req::get(uri).header(header::ACCEPT_ENCODING, "gzip"))
      .await;
  }
  assert_golden("compression", &t.finish());
}

async fn websocket_frames_for_new_update_delete_and_clear(backend: Backend) {
  let fx = fixture(backend).await;
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let app = fx.app.clone();
  let server = tokio::spawn(async move { axum::serve(listener, app).await });
  let mut client = WsClient::connect(addr).await;

  for stored in &fx.stored {
    fx.state
      .broadcast(WsEvent::MessageNew(stored.summary.clone()));
  }
  let mut t = fx.transcript();
  let welcome = fx.id("welcome").to_string();
  let uri = format!("{API}/messages/{welcome}");
  t.send(
    Req::new(Method::PATCH, &uri).json(r#"{"is_read":false,"is_starred":true,"tags":["ws"]}"#),
  )
  .await;
  t.send(Req::new(Method::PATCH, &uri).json(r#"{"tags":[""]}"#))
    .await;
  t.send(
    Req::new(Method::PATCH, format!("{API}/messages/{UNKNOWN_ID}")).json(r#"{"is_read":true}"#),
  )
  .await;
  t.send(Req::new(Method::PATCH, &uri).json(r#"{}"#)).await;
  t.send(Req::new(Method::DELETE, &uri)).await;
  t.send(Req::new(Method::DELETE, &uri)).await;
  t.send(Req::new(Method::DELETE, format!("{API}/messages")))
    .await;

  t.section("frames received, in order");
  loop {
    let frame = client.next_text().await;
    t.note(&frame);
    if frame == r#"{"type":"messages:clear"}"# {
      break;
    }
  }

  server.abort();
  assert_golden("websocket", &t.finish());
}

/// Declares one `#[tokio::test]` per golden, in a module named after the
/// backend it runs on.
macro_rules! goldens_on {
  ($module:ident, $backend:expr, [$($golden:ident),* $(,)?]) => {
    mod $module {
      $(
        #[tokio::test]
        async fn $golden() {
          super::$golden($backend).await;
        }
      )*
    }
  };
}

goldens_on!(
  fresh,
  crate::Backend::Fresh,
  [
    list_pages_filters_and_cursors,
    search_ids_order_and_totals,
    get_every_message,
    patch_updates_and_rejections,
    delete_one_message,
    delete_all_messages,
    raw_source_and_prefixes,
    headers_of_every_message,
    auth_results_of_every_message,
    attachment_lists,
    attachment_downloads,
    inline_parts_by_content_id,
    export_as_eml_and_json,
    assert_count_matching,
    release_refusals_without_a_relay,
    release_to_a_relay_that_drops_the_connection,
    compression_is_negotiated_for_json_but_not_downloads,
    websocket_frames_for_new_update_delete_and_clear,
  ]
);

goldens_on!(
  migrated,
  crate::Backend::MigratedFromV0_7_0,
  [
    list_pages_filters_and_cursors,
    search_ids_order_and_totals,
    get_every_message,
    patch_updates_and_rejections,
    delete_one_message,
    delete_all_messages,
    raw_source_and_prefixes,
    headers_of_every_message,
    auth_results_of_every_message,
    attachment_lists,
    attachment_downloads,
    inline_parts_by_content_id,
    export_as_eml_and_json,
    assert_count_matching,
    release_refusals_without_a_relay,
    compression_is_negotiated_for_json_but_not_downloads,
    websocket_frames_for_new_update_delete_and_clear,
  ]
);
