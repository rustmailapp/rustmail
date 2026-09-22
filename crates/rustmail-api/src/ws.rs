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
const WS_SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a connection is pinged, how long it may stay silent, and how long
/// one send may block before the connection is given up.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WsTimings {
  pub(crate) ping_interval: Duration,
  pub(crate) idle_timeout: Duration,
  pub(crate) send_timeout: Duration,
}

impl Default for WsTimings {
  fn default() -> Self {
    Self {
      ping_interval: WS_PING_INTERVAL,
      idle_timeout: WS_IDLE_TIMEOUT,
      send_timeout: WS_SEND_TIMEOUT,
    }
  }
}

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
/// deadline. Only frames from the peer refresh it, so reaching
/// [`WS_IDLE_TIMEOUT`] means the peer stopped answering, however much mail is
/// flowing the other way. A send that blocks past [`WS_SEND_TIMEOUT`] closes
/// the connection too, so a peer that stopped reading cannot hold its slot.
///
/// A client too slow to keep up with the broadcast channel is disconnected
/// rather than served the surviving events. Its incremental view of the inbox
/// is already wrong at that point, and only a full refetch can repair it, so
/// closing hands the job to the reconnect path that already resyncs.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
  let mut rx = state.ws_tx.subscribe();
  debug!("WebSocket client connected");

  let timings = state.ws_timings;
  let idle = tokio::time::sleep(timings.idle_timeout);
  tokio::pin!(idle);

  let mut ping = tokio::time::interval(timings.ping_interval);
  ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

  loop {
    tokio::select! {
        _ = ping.tick() => {
            if !send_within(&mut socket, Message::Ping(Bytes::new()), timings.send_timeout).await {
                break;
            }
        }
        event = rx.recv() => {
            match event {
                Ok(frame) => {
                    if !send_within(&mut socket, Message::Text(frame.text()), timings.send_timeout).await {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!(missed = n, "WebSocket client fell behind, closing so it resyncs");
                    send_within(&mut socket, Message::Close(None), timings.send_timeout).await;
                    break;
                }
                Err(RecvError::Closed) => break,
            }
        }
        msg = socket.recv() => {
            match msg {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {
                    idle.as_mut().reset(tokio::time::Instant::now() + timings.idle_timeout);
                }
                Some(Err(_)) => break,
            }
        }
        _ = &mut idle => {
            debug!("WebSocket peer stopped answering pings, closing");
            send_within(&mut socket, Message::Close(None), timings.send_timeout).await;
            break;
        }
    }
  }

  debug!("WebSocket client disconnected");
}

/// Sends `message`, reporting whether it went out within `limit`.
///
/// A peer that stopped reading fills its TCP window and would otherwise park
/// the send, and with it the whole connection, until the kernel gives up.
async fn send_within(socket: &mut WebSocket, message: Message, limit: Duration) -> bool {
  match tokio::time::timeout(limit, socket.send(message)).await {
    Ok(Ok(())) => true,
    Ok(Err(error)) => {
      debug!(%error, "WebSocket send failed, closing");
      false
    }
    Err(_) => {
      warn!(timeout = ?limit, "WebSocket send timed out, closing");
      false
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{WS_IDLE_TIMEOUT, WS_PING_INTERVAL, WS_SEND_TIMEOUT, WsTimings};
  use crate::{AppState, WsEvent, WsFrame, router};
  use rustmail_storage::{MessageRepository, initialize_database};
  use std::net::SocketAddr;
  use std::time::Duration;
  use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
  use tokio::net::TcpStream;
  use tokio::sync::broadcast;

  const TEST_TIMINGS: WsTimings = WsTimings {
    ping_interval: Duration::from_millis(50),
    idle_timeout: Duration::from_millis(300),
    send_timeout: Duration::from_millis(300),
  };
  const EVENT_CADENCE: Duration = Duration::from_millis(20);
  const VERDICT_DEADLINE: Duration = Duration::from_secs(5);
  const BROADCAST_CAPACITY: usize = 256;
  const MAX_CONNECTIONS: usize = 50;
  const LARGE_TAG_BYTES: usize = 1 << 20;
  const PERMIT_POLL: Duration = Duration::from_millis(20);
  const WS_TEXT_OPCODE: u8 = 0x1;
  const WS_CLOSE_OPCODE: u8 = 0x8;
  const WS_PING_OPCODE: u8 = 0x9;
  const WS_EXTENDED_LENGTH_MARKER: u8 = 126;
  const WS_HUGE_LENGTH_MARKER: u8 = 127;

  #[test]
  fn a_responsive_peer_always_beats_the_idle_deadline() {
    assert!(
      WS_PING_INTERVAL < WS_IDLE_TIMEOUT,
      "pings must be more frequent than the idle deadline, otherwise a live \
       client is closed before it can answer: ping={WS_PING_INTERVAL:?} \
       idle={WS_IDLE_TIMEOUT:?}"
    );
  }

  #[test]
  fn a_send_gives_up_before_the_idle_deadline() {
    assert!(
      WS_SEND_TIMEOUT < WS_IDLE_TIMEOUT,
      "a blocked send must not outlive a silent peer: send={WS_SEND_TIMEOUT:?} \
       idle={WS_IDLE_TIMEOUT:?}"
    );
  }

  async fn serve(timings: WsTimings) -> (SocketAddr, AppState, broadcast::Sender<WsFrame>) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let (ws_tx, _) = broadcast::channel::<WsFrame>(BROADCAST_CAPACITY);
    let mut state = AppState::new(MessageRepository::new(pool), ws_tx.clone(), None, None);
    state.ws_timings = timings;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move {
      axum::serve(listener, app).await.unwrap();
    });

    (addr, state, ws_tx)
  }

  async fn handshake(addr: SocketAddr) -> BufReader<TcpStream> {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
      .write_all(
        format!(
          "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        )
        .as_bytes(),
      )
      .await
      .unwrap();

    let mut reader = BufReader::new(stream);
    loop {
      let mut line = String::new();
      reader.read_line(&mut line).await.unwrap();
      assert!(!line.is_empty(), "connection closed during WS handshake");
      if line == "\r\n" {
        return reader;
      }
    }
  }

  /// Reads one unmasked server frame, or `None` once the server hung up.
  async fn read_frame(reader: &mut BufReader<TcpStream>) -> Option<(u8, Vec<u8>)> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).await.ok()?;
    let opcode = header[0] & 0x0f;

    let len = match header[1] & 0x7f {
      WS_EXTENDED_LENGTH_MARKER => {
        let mut extended = [0u8; 2];
        reader.read_exact(&mut extended).await.ok()?;
        usize::from(u16::from_be_bytes(extended))
      }
      WS_HUGE_LENGTH_MARKER => {
        let mut extended = [0u8; 8];
        reader.read_exact(&mut extended).await.ok()?;
        usize::try_from(u64::from_be_bytes(extended)).ok()?
      }
      len => usize::from(len),
    };

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await.ok()?;
    Some((opcode, payload))
  }

  #[tokio::test]
  async fn a_peer_that_never_answers_pings_is_closed_while_events_flow() {
    let (addr, _state, ws_tx) = serve(TEST_TIMINGS).await;
    let mut reader = handshake(addr).await;
    assert_eq!(
      read_frame(&mut reader).await.map(|f| f.0),
      Some(WS_PING_OPCODE)
    );

    let publisher = tokio::spawn(async move {
      let mut cadence = tokio::time::interval(EVENT_CADENCE);
      loop {
        cadence.tick().await;
        let _ = ws_tx.send(WsFrame::encode(&WsEvent::MessagesClear).unwrap());
      }
    });

    let mut text_frames = 0usize;
    let closed = tokio::time::timeout(VERDICT_DEADLINE, async {
      loop {
        match read_frame(&mut reader).await {
          None => break,
          Some((WS_CLOSE_OPCODE, _)) => break,
          Some((WS_TEXT_OPCODE, _)) => text_frames += 1,
          Some(_) => {}
        }
      }
    })
    .await;
    publisher.abort();

    assert!(
      closed.is_ok(),
      "a peer that never pongs must be closed even while events keep flowing"
    );
    assert!(
      text_frames > 0,
      "events must reach the peer until it is closed"
    );
  }

  #[tokio::test]
  async fn a_peer_that_stops_reading_releases_its_connection_slot() {
    let (addr, state, ws_tx) = serve(TEST_TIMINGS).await;
    let mut reader = handshake(addr).await;
    assert_eq!(
      read_frame(&mut reader).await.map(|f| f.0),
      Some(WS_PING_OPCODE)
    );

    let large_tag = "x".repeat(LARGE_TAG_BYTES);
    for i in 0..BROADCAST_CAPACITY / 4 {
      let event = WsEvent::MessageTags {
        id: i.to_string(),
        tags: vec![large_tag.clone()],
      };
      ws_tx.send(WsFrame::encode(&event).unwrap()).unwrap();
    }

    let released = tokio::time::timeout(VERDICT_DEADLINE, async {
      while state.ws_semaphore.available_permits() < MAX_CONNECTIONS {
        tokio::time::sleep(PERMIT_POLL).await;
      }
    })
    .await;
    drop(reader);

    assert!(
      released.is_ok(),
      "a send blocked on a peer that stopped reading must time out and free the slot"
    );
  }

  async fn next_text(reader: &mut BufReader<TcpStream>) -> Vec<u8> {
    loop {
      match tokio::time::timeout(VERDICT_DEADLINE, read_frame(reader))
        .await
        .expect("no text frame arrived")
      {
        Some((WS_TEXT_OPCODE, payload)) => return payload,
        Some((WS_PING_OPCODE, _)) => {}
        other => panic!("expected a text frame, got {other:?}"),
      }
    }
  }

  #[tokio::test]
  async fn every_subscriber_receives_the_bytes_a_per_client_encode_produced() {
    let (addr, state, _ws_tx) = serve(WsTimings::default()).await;
    let mut first = handshake(addr).await;
    let mut second = handshake(addr).await;
    assert_eq!(
      read_frame(&mut first).await.map(|f| f.0),
      Some(WS_PING_OPCODE)
    );
    assert_eq!(
      read_frame(&mut second).await.map(|f| f.0),
      Some(WS_PING_OPCODE)
    );

    let summary = state
      .repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        b"From: a@t.com\r\nTo: b@t.com\r\nSubject: Wire\r\n\r\nbody",
      )
      .await
      .unwrap();
    let event = WsEvent::MessageNew(summary);
    let expected = serde_json::to_vec(&event).unwrap();
    state.broadcast(event);

    assert_eq!(next_text(&mut first).await, expected);
    assert_eq!(next_text(&mut second).await, expected);
  }
}
