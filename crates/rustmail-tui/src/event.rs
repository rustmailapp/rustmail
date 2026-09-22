use anyhow::Result;
use futures_util::StreamExt;
use ratatui::crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

pub enum Event {
  Key(KeyEvent),
  Mouse(MouseEvent),
  Resize,
  Tick,
  WsMessage(String),
  WsStatus(bool),
}

pub struct EventHandler {
  rx: mpsc::UnboundedReceiver<Event>,
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

pub fn create_event_handler() -> (EventHandler, mpsc::UnboundedSender<Event>) {
  let (tx, rx) = mpsc::unbounded_channel();

  let event_tx = tx.clone();
  tokio::spawn(async move {
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(100));

    loop {
      tokio::select! {
        maybe_event = reader.next() => {
          match maybe_event {
            Some(Ok(CrosstermEvent::Key(key))) => {
              let _ = event_tx.send(Event::Key(key));
            }
            Some(Ok(CrosstermEvent::Mouse(mouse))) => {
              let _ = event_tx.send(Event::Mouse(mouse));
            }
            Some(Ok(CrosstermEvent::Resize(_, _))) => {
              let _ = event_tx.send(Event::Resize);
            }
            Some(Err(_)) | None => break,
            _ => {}
          }
        }
        _ = tick_interval.tick() => {
          let _ = event_tx.send(Event::Tick);
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
    let (tx, rx) = mpsc::unbounded_channel();
    let mut handler = EventHandler { rx };
    tx.send(Event::Tick).unwrap();
    tx.send(Event::Resize).unwrap();

    assert!(matches!(handler.try_next(), Some(Event::Tick)));
    assert!(matches!(handler.try_next(), Some(Event::Resize)));
    assert!(handler.try_next().is_none());
  }
}
