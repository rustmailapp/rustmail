use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::time::Duration;
use tracing::{debug, warn};

use crate::state::AppState;

const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn ws_handler(
  ws: WebSocketUpgrade,
  State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
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

async fn handle_socket(mut socket: WebSocket, state: AppState) {
  let mut rx = state.ws_tx.subscribe();
  debug!("WebSocket client connected");

  let idle = tokio::time::sleep(WS_IDLE_TIMEOUT);
  tokio::pin!(idle);

  loop {
    tokio::select! {
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
                Err(_) => break,
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
            debug!("WebSocket idle timeout, closing");
            let _ = socket.send(Message::Close(None)).await;
            break;
        }
    }
  }

  debug!("WebSocket client disconnected");
}
