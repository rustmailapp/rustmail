use anyhow::Result;
use futures_util::StreamExt;
use ratatui::crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

/// Bound on the in-flight event queue. Sized for several seconds of live
/// WebSocket traffic at the UI's draw cadence, so a burst cannot grow memory
/// without limit.
///
/// A producer that can outrun the drain loop (WebSocket frames) uses
/// `try_send` and drops the frame on overflow, forwarding a single
/// [`Event::WsOverflow`] marker in its place so the app can resync once it
/// catches up. Every other producer (keyboard, mouse, ticks) uses a
/// blocking send, which backpressures the producer instead of dropping
/// input.
pub const EVENT_QUEUE_CAPACITY: usize = 1024;

pub enum Event {
  Key(KeyEvent),
  Mouse(MouseEvent),
  Resize,
  Tick,
  WsMessage(String),
  WsStatus(bool),
  WsOverflow,
}

pub struct EventHandler {
  rx: mpsc::Receiver<Event>,
}

impl EventHandler {
  pub async fn next(&mut self) -> Result<Event> {
    self
      .rx
      .recv()
      .await
      .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
  }

  /// Returns the next already-queued event without waiting, or `None` once
  /// the queue is drained.
  pub fn try_next(&mut self) -> Option<Event> {
    self.rx.try_recv().ok()
  }
}

pub fn create_event_handler() -> (EventHandler, mpsc::Sender<Event>) {
  let (tx, rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);

  let event_tx = tx.clone();
  tokio::spawn(async move {
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(100));

    loop {
      tokio::select! {
        maybe_event = reader.next() => {
          let event = match maybe_event {
            Some(Ok(CrosstermEvent::Key(key))) => Event::Key(key),
            Some(Ok(CrosstermEvent::Mouse(mouse))) => Event::Mouse(mouse),
            Some(Ok(CrosstermEvent::Resize(_, _))) => Event::Resize,
            Some(Err(_)) | None => break,
            _ => continue,
          };
          if event_tx.send(event).await.is_err() {
            break;
          }
        }
        _ = tick_interval.tick() => {
          if event_tx.send(Event::Tick).await.is_err() {
            break;
          }
        }
      }
    }
  });

  (EventHandler { rx }, tx)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn try_next_drains_the_queue_without_blocking() {
    let (tx, rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
    let mut handler = EventHandler { rx };
    tx.send(Event::Tick).await.unwrap();
    tx.send(Event::Resize).await.unwrap();

    assert!(matches!(handler.try_next(), Some(Event::Tick)));
    assert!(matches!(handler.try_next(), Some(Event::Resize)));
    assert!(handler.try_next().is_none());
  }

  #[tokio::test]
  async fn channel_rejects_sends_past_its_capacity() {
    let (tx, _rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
    for _ in 0..EVENT_QUEUE_CAPACITY {
      tx.try_send(Event::Tick)
        .expect("capacity should not be exceeded yet");
    }
    assert!(tx.try_send(Event::Tick).is_err());
  }
}
