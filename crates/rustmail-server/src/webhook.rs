use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{Mutex, mpsc};
use tracing::warn;

/// Concurrent webhook requests in flight.
pub const WEBHOOK_WORKERS: usize = 10;

/// Notifications that may wait for a free worker before new ones are dropped.
pub const WEBHOOK_QUEUE_CAPACITY: usize = 1024;

const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded fan-out of webhook notifications to a fixed pool of workers.
///
/// A slow or unreachable webhook fills the queue instead of piling up tasks;
/// once it is full, new notifications are dropped and counted.
pub struct WebhookQueue<T> {
  tx: mpsc::Sender<T>,
  dropped_total: u64,
}

impl<T: Serialize + Send + 'static> WebhookQueue<T> {
  /// Starts `workers` tasks posting to `url`. They stop once the queue is
  /// dropped and drained.
  pub fn spawn(client: reqwest::Client, url: String, workers: usize, capacity: usize) -> Self {
    let (tx, rx) = mpsc::channel(capacity);
    let rx = Arc::new(Mutex::new(rx));
    let url: Arc<str> = url.into();
    for _ in 0..workers {
      tokio::spawn(run_worker(client.clone(), url.clone(), rx.clone()));
    }
    Self {
      tx,
      dropped_total: 0,
    }
  }

  /// Queues every payload that fits and returns how many were dropped.
  pub fn enqueue_batch(&mut self, payloads: Vec<T>) -> usize {
    let mut dropped = 0;
    for payload in payloads {
      if self.tx.try_send(payload).is_err() {
        dropped += 1;
      }
    }
    if dropped > 0 {
      self.dropped_total += dropped as u64;
      warn!(
        dropped,
        dropped_total = self.dropped_total,
        capacity = self.tx.max_capacity(),
        "Webhook queue full, notifications dropped"
      );
    }
    dropped
  }
}

async fn run_worker<T: Serialize>(
  client: reqwest::Client,
  url: Arc<str>,
  rx: Arc<Mutex<mpsc::Receiver<T>>>,
) {
  loop {
    let Some(payload) = rx.lock().await.recv().await else {
      return;
    };
    if let Err(e) = client
      .post(&*url)
      .json(&payload)
      .timeout(WEBHOOK_TIMEOUT)
      .send()
      .await
    {
      warn!(error = %e, "Webhook delivery failed");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::{Json, Router, extract::State, routing::post};
  use tokio::io::AsyncReadExt;
  use tokio::net::{TcpListener, TcpStream};

  const WAIT: Duration = Duration::from_secs(5);

  async fn stalling_webhook() -> (String, mpsc::UnboundedReceiver<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
      while let Ok((mut socket, _)) = listener.accept().await {
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;
        if tx.send(socket).is_err() {
          return;
        }
      }
    });
    (url, rx)
  }

  async fn recording_webhook() -> (String, mpsc::UnboundedReceiver<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    let app = Router::new()
      .route(
        "/hook",
        post(
          |State(tx): State<mpsc::UnboundedSender<serde_json::Value>>,
           Json(body): Json<serde_json::Value>| async move {
            let _ = tx.send(body);
          },
        ),
      )
      .with_state(tx);
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, rx)
  }

  #[tokio::test]
  async fn a_stalled_webhook_drops_what_does_not_fit_in_the_queue() {
    let (url, mut requests) = stalling_webhook().await;
    let mut queue = WebhookQueue::spawn(reqwest::Client::new(), url, 1, 2);

    assert_eq!(queue.enqueue_batch(vec![0]), 0);
    let _in_flight = tokio::time::timeout(WAIT, requests.recv())
      .await
      .unwrap()
      .unwrap();

    assert_eq!(queue.enqueue_batch(vec![1, 2, 3, 4, 5]), 3);
    assert_eq!(queue.enqueue_batch(vec![6]), 1);
    assert_eq!(queue.dropped_total, 4);
  }

  #[tokio::test]
  async fn a_stalled_webhook_holds_at_most_one_request_per_worker() {
    let (url, mut requests) = stalling_webhook().await;
    let mut queue = WebhookQueue::spawn(reqwest::Client::new(), url, 2, 8);

    assert_eq!(queue.enqueue_batch((0..8).collect()), 0);
    let mut in_flight = Vec::new();
    for _ in 0..2 {
      in_flight.push(
        tokio::time::timeout(WAIT, requests.recv())
          .await
          .unwrap()
          .unwrap(),
      );
    }

    assert!(
      tokio::time::timeout(Duration::from_millis(300), requests.recv())
        .await
        .is_err()
    );
  }

  #[tokio::test]
  async fn every_queued_notification_reaches_a_healthy_webhook() {
    let (url, mut bodies) = recording_webhook().await;
    let mut queue = WebhookQueue::spawn(reqwest::Client::new(), url, 3, 16);

    assert_eq!(queue.enqueue_batch((0..10).collect::<Vec<u32>>()), 0);

    let mut received = Vec::new();
    for _ in 0..10 {
      let body = tokio::time::timeout(WAIT, bodies.recv())
        .await
        .unwrap()
        .unwrap();
      received.push(body.as_u64().unwrap());
    }
    received.sort_unstable();
    assert_eq!(received, (0..10).collect::<Vec<u64>>());
  }
}
