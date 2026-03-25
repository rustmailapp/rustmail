use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent};
use futures_util::StreamExt;
use tokio::sync::mpsc;

pub enum Event {
  Key(KeyEvent),
  Tick,
  WsMessage(String),
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
