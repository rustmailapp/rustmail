use axum::http::HeaderValue;

const HTTP: &str = "http";
const HTTPS: &str = "https";
const HTTP_DEFAULT_PORT: u16 = 80;
const HTTPS_DEFAULT_PORT: u16 = 443;

/// A web origin — the `scheme://host[:port]` triple a browser puts in `Origin`.
///
/// Parsing normalizes what a browser already normalizes: scheme and host are
/// lowercased and a default port is dropped, so a configured origin compares
/// equal to the header a browser actually sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
  scheme: &'static str,
  authority: String,
}

/// Why a string could not be read as an [`Origin`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OriginError {
  #[error("origin `{0}` must start with http:// or https://")]
  UnsupportedScheme(String),
  #[error("origin `{0}` must be scheme://host[:port], with no path, query or credentials")]
  MalformedAuthority(String),
  #[error("origin `{0}` must have a port between 1 and 65535")]
  InvalidPort(String),
}

impl Origin {
  /// The `host[:port]` part, which is what the `Host` header of a request
  /// addressed to this origin carries.
  fn authority(&self) -> &str {
    &self.authority
  }
}

impl std::str::FromStr for Origin {
  type Err = OriginError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let raw = s.trim();
    let (scheme, authority) = raw
      .split_once("://")
      .ok_or_else(|| OriginError::UnsupportedScheme(raw.to_string()))?;

    let scheme = match scheme.to_ascii_lowercase().as_str() {
      HTTP => HTTP,
      HTTPS => HTTPS,
      _ => return Err(OriginError::UnsupportedScheme(raw.to_string())),
    };

    Ok(Self {
      scheme,
      authority: normalize_authority(authority, scheme, raw)?,
    })
  }
}

impl std::fmt::Display for Origin {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}://{}", self.scheme, self.authority)
  }
}

fn normalize_authority(
  authority: &str,
  scheme: &'static str,
  raw: &str,
) -> Result<String, OriginError> {
  let malformed = || OriginError::MalformedAuthority(raw.to_string());

  if authority.is_empty() || authority.contains(['/', '?', '#', '@', ' ', '\t']) {
    return Err(malformed());
  }

  let (host, port) = split_host_port(authority).ok_or_else(malformed)?;
  if host.is_empty() {
    return Err(malformed());
  }

  let Some(port) = port else {
    return Ok(host);
  };

  let port: u16 = port
    .parse()
    .map_err(|_| OriginError::InvalidPort(raw.to_string()))?;
  if port == 0 {
    return Err(OriginError::InvalidPort(raw.to_string()));
  }

  if is_default_port(scheme, port) {
    Ok(host)
  } else {
    Ok(format!("{host}:{port}"))
  }
}

fn split_host_port(authority: &str) -> Option<(String, Option<&str>)> {
  if authority.starts_with('[') {
    let close = authority.find(']')?;
    let host = authority[..=close].to_ascii_lowercase();
    match &authority[close + 1..] {
      "" => Some((host, None)),
      rest => rest.strip_prefix(':').map(|port| (host, Some(port))),
    }
  } else {
    match authority.split_once(':') {
      Some((host, port)) => Some((host.to_ascii_lowercase(), Some(port))),
      None => Some((authority.to_ascii_lowercase(), None)),
    }
  }
}

fn is_default_port(scheme: &str, port: u16) -> bool {
  (scheme == HTTP && port == HTTP_DEFAULT_PORT) || (scheme == HTTPS && port == HTTPS_DEFAULT_PORT)
}

/// Decides whether a handshake carrying this `Origin` may open the WebSocket.
///
/// The origin the server is reached at is always allowed: that is the bundled
/// UI, and `host` is set by the browser from the address it dialled, not by the
/// page that asked for the connection. Any other origin has to be configured.
/// An origin the browser would never send — `null`, or anything unparsable —
/// is refused.
///
/// `host` is normalized against the origin's own scheme, so a proxy that spells
/// out a default port in `Host` still compares equal to the origin a browser
/// derived from the same address.
pub(crate) fn origin_allowed(
  origin: &HeaderValue,
  host: Option<&HeaderValue>,
  allowed: &[Origin],
) -> bool {
  let Some(origin) = origin
    .to_str()
    .ok()
    .and_then(|value| value.parse::<Origin>().ok())
  else {
    return false;
  };

  if allowed.contains(&origin) {
    return true;
  }

  host
    .and_then(|host| host.to_str().ok())
    .map(str::trim)
    .and_then(|host| normalize_authority(host, origin.scheme, host).ok())
    .is_some_and(|host| host == origin.authority())
}

#[cfg(test)]
mod tests {
  use super::{Origin, OriginError, origin_allowed};
  use axum::http::HeaderValue;

  fn origin(value: &str) -> Origin {
    value.parse().unwrap()
  }

  #[test]
  fn parses_scheme_host_and_port() {
    assert_eq!(
      origin("http://localhost:8025").to_string(),
      "http://localhost:8025"
    );
    assert_eq!(
      origin("https://mail.example.com").to_string(),
      "https://mail.example.com"
    );
    assert_eq!(origin("http://[::1]:8025").to_string(), "http://[::1]:8025");
  }

  #[test]
  fn normalizes_case_and_default_ports() {
    assert_eq!(
      origin("HTTP://LocalHost:8025"),
      origin("http://localhost:8025")
    );
    assert_eq!(
      origin("http://example.com:80"),
      origin("http://example.com")
    );
    assert_eq!(
      origin("https://example.com:443"),
      origin("https://example.com")
    );
    assert_ne!(
      origin("http://example.com:443"),
      origin("http://example.com")
    );
  }

  #[test]
  fn rejects_anything_that_is_not_a_bare_origin() {
    let cases = [
      "example.com",
      "ftp://example.com",
      "null",
      "http://",
      "http://example.com/inbox",
      "http://example.com?a=1",
      "http://user@example.com",
      "http://example.com:0",
      "http://example.com:http",
      "http://example.com:",
      "http://example.com:70000",
    ];
    for case in cases {
      assert!(
        case.parse::<Origin>().is_err(),
        "expected `{case}` to be refused"
      );
    }
  }

  #[test]
  fn parse_errors_name_the_offending_value() {
    assert_eq!(
      "ftp://example.com".parse::<Origin>().unwrap_err(),
      OriginError::UnsupportedScheme("ftp://example.com".to_string())
    );
    assert_eq!(
      "http://example.com/inbox".parse::<Origin>().unwrap_err(),
      OriginError::MalformedAuthority("http://example.com/inbox".to_string())
    );
    assert_eq!(
      "http://example.com:0".parse::<Origin>().unwrap_err(),
      OriginError::InvalidPort("http://example.com:0".to_string())
    );
  }

  #[test]
  fn the_origin_the_server_is_reached_at_is_allowed() {
    assert!(origin_allowed(
      &HeaderValue::from_static("http://localhost:8025"),
      Some(&HeaderValue::from_static("localhost:8025")),
      &[],
    ));
  }

  #[test]
  fn a_host_spelling_out_the_default_port_still_matches() {
    assert!(origin_allowed(
      &HeaderValue::from_static("https://mail.example.com"),
      Some(&HeaderValue::from_static("mail.example.com:443")),
      &[],
    ));
  }

  #[test]
  fn another_page_on_the_same_host_is_refused() {
    assert!(!origin_allowed(
      &HeaderValue::from_static("http://localhost:3001"),
      Some(&HeaderValue::from_static("localhost:8025")),
      &[],
    ));
  }

  #[test]
  fn a_configured_origin_is_allowed_whatever_the_host_says() {
    assert!(origin_allowed(
      &HeaderValue::from_static("https://mail.example.com"),
      Some(&HeaderValue::from_static("127.0.0.1:8025")),
      &[origin("https://mail.example.com")],
    ));
  }

  #[test]
  fn an_unparsable_origin_is_refused() {
    assert!(!origin_allowed(
      &HeaderValue::from_static("null"),
      Some(&HeaderValue::from_static("localhost:8025")),
      &[],
    ));
  }

  #[test]
  fn a_handshake_without_a_host_header_is_refused() {
    assert!(!origin_allowed(
      &HeaderValue::from_static("http://localhost:8025"),
      None,
      &[],
    ));
  }
}
