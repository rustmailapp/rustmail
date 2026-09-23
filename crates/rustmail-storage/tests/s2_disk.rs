//! Disk use of the S2 workload: mail whose size is almost all one base64
//! binary attachment, as the phase-3 bench's `--kind medium` sends it
//! (150 000 random bytes, 76-column CRLF base64, 16 distinct blobs).
//!
//! With attachments served from the raw message, the store holds each such
//! mail about once, so the file stays within [`MAX_DISK_TO_RAW_RATIO`] of the
//! raw bytes captured. The ratio is measured on a file-backed store after a
//! truncating checkpoint, over the main file plus any WAL left, and printed;
//! run `cargo test -p rustmail-storage --test s2_disk -- --nocapture` to see
//! it.

use rustmail_storage::{MessageRepository, connect_options, initialize_database};
use sqlx::sqlite::SqlitePoolOptions;
use ulid::Ulid;

const MESSAGES: usize = 200;
const DISTINCT_BLOBS: usize = 16;
const ATTACHMENT_BYTES: usize = 150_000;
const BASE64_LINE: usize = 76;
const MAX_DISK_TO_RAW_RATIO: f64 = 1.05;
const BASE64_ALPHABET: &[u8; 64] =
  b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
  let mut state = seed.max(1);
  (0..len)
    .map(|_| {
      state ^= state << 13;
      state ^= state >> 7;
      state ^= state << 17;
      state.to_le_bytes()[0]
    })
    .collect()
}

fn base64_lines(bytes: &[u8]) -> String {
  let mut encoded = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let word = chunk.iter().enumerate().fold(0u32, |word, (i, &byte)| {
      word | u32::from(byte) << (16 - 8 * i)
    });
    for i in 0..4 {
      encoded.push(if i <= chunk.len() {
        BASE64_ALPHABET[(word >> (18 - 6 * i) & 0x3f) as usize]
      } else {
        b'='
      });
    }
  }
  encoded
    .chunks(BASE64_LINE)
    .map(|line| std::str::from_utf8(line).unwrap())
    .collect::<Vec<_>>()
    .join("\r\n")
}

fn medium_message(i: usize, blob: &str) -> Vec<u8> {
  format!(
    "From: reports@bench.test\r\nTo: user{}@bench.test\r\nSubject: Report {i} quarterly\r\n\
     Message-ID: <bench-m-{i}@bench.test>\r\nDate: Tue, 22 Sep 2026 12:00:00 +0000\r\n\
     MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"MIX\"\r\n\r\n\
     --MIX\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nReport {i} attached.\r\n\
     --MIX\r\nContent-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"report-{i}.bin\"\r\n\
     Content-Transfer-Encoding: base64\r\n\r\n{blob}\r\n--MIX--\r\n",
    i % 997
  )
  .into_bytes()
}

#[tokio::test]
async fn attachment_heavy_mail_is_stored_about_once() {
  let dir = TempDir(std::env::temp_dir().join(format!("rustmail-s2-disk-{}", Ulid::new())));
  std::fs::create_dir_all(&dir.0).unwrap();
  let db_path = dir.0.join("s2.db");
  let pool = SqlitePoolOptions::new()
    .connect_with(connect_options(&format!("sqlite://{}?mode=rwc", db_path.display())).unwrap())
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool.clone());

  let blobs: Vec<String> = (0..DISTINCT_BLOBS)
    .map(|seed| base64_lines(&random_bytes(seed as u64 + 1, ATTACHMENT_BYTES)))
    .collect();
  let mut raw_bytes = 0;
  for i in 0..MESSAGES {
    let raw = medium_message(i, &blobs[i % DISTINCT_BLOBS]);
    raw_bytes += raw.len();
    repo
      .insert("reports@bench.test", &["user@bench.test".to_string()], &raw)
      .await
      .unwrap();
  }

  let located: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE raw_offset IS NOT NULL")
      .fetch_one(&pool)
      .await
      .unwrap();
  assert_eq!(
    located, MESSAGES as i64,
    "every S2 attachment should be located"
  );

  sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
    .execute(&pool)
    .await
    .unwrap();
  let file_bytes = |suffix: &str| {
    std::fs::metadata(dir.0.join(format!("s2.db{suffix}"))).map_or(0, |meta| meta.len())
  };
  let disk_bytes = file_bytes("") + file_bytes("-wal");
  let ratio = disk_bytes as f64 / raw_bytes as f64;
  println!("S2 disk: {disk_bytes} bytes for {raw_bytes} raw bytes, ratio {ratio:.3}");
  assert!(
    ratio <= MAX_DISK_TO_RAW_RATIO,
    "S2 disk is {ratio:.3}x raw, above {MAX_DISK_TO_RAW_RATIO}x"
  );
  pool.close().await;
}
