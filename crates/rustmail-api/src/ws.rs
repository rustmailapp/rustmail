use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::origin::origin_allowed;
use crate::state::AppState;

const WS_PING_INTERVAL: Duration = Duration::from_secs(30);
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Upgrades a request to a WebSocket carrying [`WsEvent`](crate::WsEvent)s.
///
/// A handshake sent by a browser page is refused unless it comes from the
/// origin RustMail is reached at or from a configured one: the events name
/// senders, recipients and subjects, and nothing else stops a page the victim
/// happens to have open from subscribing. A handshake with no `Origin` at all
/// is a non-browser client — the TUI, `websocat`, CI — and is allowed, since a
/// browser will not let a page omit the header.
pub async fn ws_handler(
  ws: WebSocketUpgrade,
  State(state): State<AppState>,
  headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
  if let Some(origin) = headers.get(ORIGIN)
    && !origin_allowed(origin, headers.get(HOST), &state.allowed_origins)
  {
    warn!(
      origin = %String::from_utf8_lossy(origin.as_bytes()),
      "WebSocket handshake rejected: origin not allowed"
    );
    return Err(StatusCode::FORBIDDEN);
  }

  let permit = state
    .ws_semaphore
    .clone()
    .try_acquire_owned()
    .map_err(|_| {
      warn!("WebSocket connection rejected: max connections reached");
      StatusCode::SERVICE_UNAVAILABLE
    })?;

  Ok(ws.on_upgrade(move |socket| async move {
    handle_socket(socket, state).await;
    drop(permit);
  }))
}

/// Streams [`WsEvent`](crate::WsEvent)s to one client until it goes away.
///
/// A ping on connect and then every [`WS_PING_INTERVAL`] keeps a quiet-but-live
/// connection open: browsers answer with a pong, which refreshes the idle
/// deadline. Reaching [`WS_IDLE_TIMEOUT`] therefore means the peer stopped
/// answering, not merely that no mail arrived.
///
/// A client too slow to keep up with the broadcast channel is disconnected
/// rather than served the surviving events. Its incremental view of the inbox
/// is already wrong at that point, and only a full refetch can repair it, so
/// closing hands the job to the reconnect path that already resyncs.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
  let mut rx = state.ws_tx.subscribe();
  debug!("WebSocket client connected");

  let idle = tokio::time::sleep(WS_IDLE_TIMEOUT);
  tokio::pin!(idle);

  let mut ping = tokio::time::interval(WS_PING_INTERVAL);
  ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

  loop {
    tokio::select! {
        _ = ping.tick() => {
            if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                break;
            }
        }
        event = rx.recv() => {
            idle.as_mut().reset(tokio::time::Instant::now() + WS_IDLE_TIMEOUT);
            match event {
                Ok(event) => {
                    let json = match serde_json::to_string(&event) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!(missed = n, "WebSocket client fell behind, closing so it resyncs");
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                Err(RecvError::Closed) => break,
            }
        }
        msg = socket.recv() => {
            match msg {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {
                    idle.as_mut().reset(tokio::time::Instant::now() + WS_IDLE_TIMEOUT);
                }
                Some(Err(_)) => break,
            }
        }
        _ = &mut idle => {
            debug!("WebSocket peer stopped answering pings, closing");
            let _ = socket.send(Message::Close(None)).await;
            break;
        }
    }
  }

  debug!("WebSocket client disconnected");
}

#[cfg(test)]
mod tests {
  use super::{WS_IDLE_TIMEOUT, WS_PING_INTERVAL};

  #[test]
  fn a_responsive_peer_always_beats_the_idle_deadline() {
    assert!(
      WS_PING_INTERVAL < WS_IDLE_TIMEOUT,
      "pings must be more frequent than the idle deadline, otherwise a live \
       client is closed before it can answer: ping={WS_PING_INTERVAL:?} \
       idle={WS_IDLE_TIMEOUT:?}"
    );
  }
}
