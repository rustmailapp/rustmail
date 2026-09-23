//! A minimal WebSocket client over a real socket: enough of RFC 6455 to
//! complete the handshake and read the unmasked frames a server sends.
//!
//! Hand-rolled so the goldens need no client crate, and so they see the
//! frames exactly as they cross the wire.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const READ_TIMEOUT: Duration = Duration::from_secs(10);
const OPCODE_MASK: u8 = 0x0f;
const LENGTH_MASK: u8 = 0x7f;
const MASKED_BIT: u8 = 0x80;
const LENGTH_U16: u8 = 126;
const LENGTH_U64: u8 = 127;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xa;
const SWITCHING_PROTOCOLS: &str = "HTTP/1.1 101";

/// A connected client that has seen the server's first ping, which the
/// server sends only once it has subscribed to the broadcast channel.
pub struct WsClient {
  stream: BufReader<TcpStream>,
}

impl WsClient {
  pub async fn connect(addr: SocketAddr) -> Self {
    let mut stream = BufReader::new(TcpStream::connect(addr).await.unwrap());
    let request = format!(
      "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream
      .get_mut()
      .write_all(request.as_bytes())
      .await
      .unwrap();

    let mut status = String::new();
    within(stream.read_line(&mut status)).await.unwrap();
    assert!(
      status.starts_with(SWITCHING_PROTOCOLS),
      "handshake refused: {status:?}"
    );
    loop {
      let mut line = String::new();
      within(stream.read_line(&mut line)).await.unwrap();
      if line == "\r\n" {
        break;
      }
    }

    let mut client = Self { stream };
    let (opcode, _) = client.frame().await;
    assert_eq!(opcode, OPCODE_PING, "the server's first frame is its ping");
    client
  }

  /// The next text frame, skipping control frames.
  pub async fn next_text(&mut self) -> String {
    loop {
      let (opcode, payload) = self.frame().await;
      match opcode {
        OPCODE_TEXT => return String::from_utf8(payload).expect("text frames are UTF-8"),
        OPCODE_PING | OPCODE_PONG => continue,
        OPCODE_CLOSE => panic!("the server closed the WebSocket"),
        other => panic!("unexpected WebSocket opcode {other:#x}"),
      }
    }
  }

  async fn frame(&mut self) -> (u8, Vec<u8>) {
    let mut head = [0u8; 2];
    within(self.stream.read_exact(&mut head)).await.unwrap();
    assert_eq!(head[1] & MASKED_BIT, 0, "server frames are never masked");
    let len = match head[1] & LENGTH_MASK {
      LENGTH_U16 => {
        let mut len = [0u8; 2];
        within(self.stream.read_exact(&mut len)).await.unwrap();
        usize::from(u16::from_be_bytes(len))
      }
      LENGTH_U64 => {
        let mut len = [0u8; 8];
        within(self.stream.read_exact(&mut len)).await.unwrap();
        usize::try_from(u64::from_be_bytes(len)).unwrap()
      }
      short => usize::from(short),
    };
    let mut payload = vec![0u8; len];
    within(self.stream.read_exact(&mut payload)).await.unwrap();
    (head[0] & OPCODE_MASK, payload)
  }
}

async fn within<T>(future: impl Future<Output = T>) -> T {
  tokio::time::timeout(READ_TIMEOUT, future)
    .await
    .expect("the WebSocket server went silent")
}
