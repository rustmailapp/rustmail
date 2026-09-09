use axum::extract::{Request, State};
use axum::http::header::{HOST, USER_AGENT};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::warn;

use crate::origin::{Origin, hostname_of};
use crate::state::AppState;

const LOCALHOST: &str = "localhost";
const LOCALHOST_SUFFIX: &str = ".localhost";
const BROWSER_MARKERS: [&str; 4] = [
  "sec-fetch-site",
  "sec-fetch-mode",
  "sec-fetch-dest",
  "origin",
];
const BROWSER_AGENT_PREFIX: &str = "Mozilla/";
const REFUSAL: &str =
  "This Host is not allowed. Start rustmail with --allowed-host <name> to serve it.\n";

/// A host name RustMail will answer a browser on.
///
/// IP addresses and `localhost` never need to be listed — they are always
/// answered, and neither can be pointed somewhere else by an attacker's DNS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hostname(String);

/// Why a string could not be read as a [`Hostname`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HostnameError {
  #[error("host must not be empty")]
  Empty,
  #[error("host `{0}` must be a bare name such as mail.example.com, with no scheme, port or path")]
  NotBare(String),
}

impl std::str::FromStr for Hostname {
  type Err = HostnameError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let raw = s.trim();
    if raw.is_empty() {
      return Err(HostnameError::Empty);
    }

    if !raw
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
      return Err(HostnameError::NotBare(raw.to_string()));
    }

    Ok(Self(raw.to_ascii_lowercase()))
  }
}

impl std::fmt::Display for Hostname {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

/// Refuses a browser request addressed to a host RustMail was not asked to serve.
///
/// This is what stops DNS rebinding, which the origin check on its own cannot:
/// once the attacker's name resolves to the machine running RustMail, the page's
/// `Origin` and the request's `Host` agree, and only the host itself gives the
/// attack away.
///
/// Requests carrying none of a browser's fetch metadata are let through on any
/// host. They are CI scripts, `curl`, the TUI and `rustmail-action`, which reach
/// the API by names RustMail cannot know — a Docker service name, a CI hostname
/// — and DNS rebinding is an attack on browsers alone.
pub(crate) async fn guard_host(
  State(state): State<AppState>,
  request: Request,
  next: Next,
) -> Response {
  let refused = {
    let headers = request.headers();
    (is_browser_request(headers)
      && !host_allowed(headers, &state.allowed_hosts, &state.allowed_origins))
    .then(|| spelled_host(headers))
  };

  if let Some(host) = refused {
    warn!(host = %host, "Request rejected: Host not allowed");
    return (StatusCode::FORBIDDEN, REFUSAL).into_response();
  }

  next.run(request).await
}

/// Recognizes a request a browser made, which is the only kind DNS rebinding
/// can produce.
///
/// Fetch metadata is the reliable signal — a page cannot strip `Sec-Fetch-*`,
/// and every engine has sent it since Safari 16.4 landed in March 2023 — and an
/// `Origin` covers the WebSocket handshake and every non-`GET`. The user agent
/// catches what is left: a browser older than that still says `Mozilla/`, while
/// `curl`, the TUI and CI clients do not.
fn is_browser_request(headers: &HeaderMap) -> bool {
  BROWSER_MARKERS
    .iter()
    .any(|marker| headers.contains_key(*marker))
    || headers
      .get(USER_AGENT)
      .and_then(|agent| agent.to_str().ok())
      .is_some_and(|agent| agent.starts_with(BROWSER_AGENT_PREFIX))
}

fn spelled_host(headers: &HeaderMap) -> String {
  headers
    .get(HOST)
    .map(|host| String::from_utf8_lossy(host.as_bytes()).into_owned())
    .unwrap_or_default()
}

/// Decides whether RustMail should answer a request addressed to this `Host`.
///
/// An IP address is always answered: a page loaded from one talks to that
/// address and no other, so there is no name for an attacker to re-point.
/// `localhost` and its subdomains are reserved for the loopback interface and
/// cannot be registered, so they are answered too. Every other name has to be
/// configured, either as a host or as the host of an allowed origin.
fn host_allowed(headers: &HeaderMap, allowed: &[Hostname], origins: &[Origin]) -> bool {
  let Some(host) = headers.get(HOST).and_then(|host| host.to_str().ok()) else {
    return false;
  };

  let host = hostname_of(host.trim()).to_ascii_lowercase();
  if host.is_empty() {
    return false;
  }

  if is_ip_literal(&host) || host == LOCALHOST || host.ends_with(LOCALHOST_SUFFIX) {
    return true;
  }

  allowed.iter().any(|allowed| allowed.0 == host)
    || origins
      .iter()
      .any(|origin| hostname_of(origin.authority()) == host)
}

fn is_ip_literal(host: &str) -> bool {
  host
    .strip_prefix('[')
    .and_then(|host| host.strip_suffix(']'))
    .unwrap_or(host)
    .parse::<std::net::IpAddr>()
    .is_ok()
}

#[cfg(test)]
mod tests {
  use super::{Hostname, HostnameError, host_allowed, is_browser_request};
  use axum::http::HeaderMap;
  use axum::http::header::{HOST, ORIGIN};

  fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
      headers.insert(*name, value.parse().unwrap());
    }
    headers
  }

  fn allowing(host: &str) -> Vec<Hostname> {
    vec![host.parse().unwrap()]
  }

  #[test]
  fn an_address_no_dns_can_move_is_answered() {
    for host in [
      "127.0.0.1:8025",
      "192.168.1.5:8025",
      "[::1]:8025",
      "localhost:8025",
      "rustmail.localhost:8025",
      "LOCALHOST",
    ] {
      assert!(
        host_allowed(&headers(&[("host", host)]), &[], &[]),
        "expected `{host}` to be answered without configuration"
      );
    }
  }

  #[test]
  fn a_rebound_name_is_refused() {
    assert!(!host_allowed(
      &headers(&[("host", "evil.example:8025")]),
      &[],
      &[],
    ));
  }

  #[test]
  fn a_configured_name_is_answered() {
    assert!(host_allowed(
      &headers(&[("host", "mail.example.com:8025")]),
      &allowing("mail.example.com"),
      &[],
    ));
  }

  #[test]
  fn the_host_of_an_allowed_origin_is_answered_too() {
    assert!(
      host_allowed(
        &headers(&[("host", "mail.example.com")]),
        &[],
        &["https://mail.example.com".parse().unwrap()],
      ),
      "a reverse-proxy deployment must not have to name the same host twice"
    );
  }

  #[test]
  fn a_request_without_a_host_header_is_refused() {
    assert!(!host_allowed(&headers(&[]), &[], &[]));
  }

  #[test]
  fn fetch_metadata_and_origin_mark_a_browser() {
    assert!(is_browser_request(&headers(&[(
      "sec-fetch-site",
      "same-origin"
    )])));
    assert!(is_browser_request(&headers(&[(
      "sec-fetch-mode",
      "websocket"
    )])));
    assert!(is_browser_request(&headers(&[(
      ORIGIN.as_str(),
      "http://localhost:8025"
    )])));
    assert!(!is_browser_request(&headers(&[(
      HOST.as_str(),
      "rustmail:8025"
    )])));
  }

  #[test]
  fn a_browser_older_than_fetch_metadata_is_still_a_browser() {
    assert!(
      is_browser_request(&headers(&[(
        "user-agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/16.3 Safari/605.1.15"
      )])),
      "a same-origin GET from such a browser carries neither Sec-Fetch-* nor Origin"
    );

    for agent in ["curl/8.7.1", "rustmail-tui/0.6.0", "node"] {
      assert!(
        !is_browser_request(&headers(&[("user-agent", agent)])),
        "expected `{agent}` to be left alone"
      );
    }
  }

  #[test]
  fn hostnames_are_bare_names() {
    assert_eq!(
      "Mail.Example.COM".parse::<Hostname>().unwrap().to_string(),
      "mail.example.com"
    );
    assert_eq!(
      "rustmail_test".parse::<Hostname>().unwrap().to_string(),
      "rustmail_test"
    );

    for case in [
      "",
      "  ",
      "https://mail.example.com",
      "mail.example.com:8025",
      "[::1]",
    ] {
      assert!(
        case.parse::<Hostname>().is_err(),
        "expected `{case}` to be refused"
      );
    }

    assert_eq!(
      "mail.example.com:8025".parse::<Hostname>().unwrap_err(),
      HostnameError::NotBare("mail.example.com:8025".to_string())
    );
  }
}
