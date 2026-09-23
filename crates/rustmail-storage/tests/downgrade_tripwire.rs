//! A rustmail from before schema versioning must fail on a schema-1 file
//! before it writes anything to it.
//!
//! Such a binary never reads `user_version`; it runs its own
//! `CREATE ... IF NOT EXISTS` statements against whatever file it is given.
//! [`initialize_as_v0_7_0`] replays those statements, vendored verbatim from
//! `v0.7.0:crates/rustmail-storage/src/schema.rs`, which is also the last tag
//! before phase 4.

#[path = "common/legacy_v0_7_0.rs"]
mod legacy_v0_7_0;

use std::path::{Path, PathBuf};

use legacy_v0_7_0::{file_url, initialize_as_v0_7_0};
use rustmail_storage::{MessageRepository, SCHEMA_VERSION, connect_options, initialize_database};
use sqlx::sqlite::SqlitePoolOptions;
use ulid::Ulid;

struct TempDir(PathBuf);
impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
  let mut name = path.as_os_str().to_owned();
  name.push(suffix);
  PathBuf::from(name)
}

async fn schema_1_file_with_one_message(path: &Path) -> String {
  let pool = SqlitePoolOptions::new()
    .max_connections(1)
    .connect_with(connect_options(&file_url(path)).unwrap())
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let repo = MessageRepository::new(pool);
  let summary = repo
    .insert(
      "a@test.com",
      &["b@test.com".to_string()],
      b"Subject: kept\r\n\r\nbody",
    )
    .await
    .unwrap();
  repo.close().await;
  summary.id
}

#[tokio::test]
async fn rustmail_0_7_0_fails_on_a_schema_1_file_before_writing() {
  let dir = std::env::temp_dir().join(format!("rustmail-tripwire-{}", Ulid::new()));
  std::fs::create_dir_all(&dir).unwrap();
  let _guard = TempDir(dir.clone());
  let path = dir.join("rustmail.db");
  let id = schema_1_file_with_one_message(&path).await;
  let bytes_before = std::fs::read(&path).unwrap();
  assert_eq!(
    std::fs::metadata(sidecar(&path, "-wal")).map_or(0, |meta| meta.len()),
    0,
    "the snapshot must hold every committed page"
  );

  let legacy = SqlitePoolOptions::new()
    .max_connections(1)
    .connect(&file_url(&path))
    .await
    .unwrap();
  let error = initialize_as_v0_7_0(&legacy)
    .await
    .expect_err("rustmail 0.7.0 must not open a schema-1 file");
  legacy.close().await;

  assert!(
    error.to_string().contains("no such column: message_id"),
    "the tripwire should be the attachments index, got {error}"
  );
  assert_eq!(
    std::fs::read(&path).unwrap(),
    bytes_before,
    "rustmail 0.7.0 must not write to a schema-1 file"
  );

  let pool = SqlitePoolOptions::new()
    .connect_with(connect_options(&file_url(&path)).unwrap())
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  let version: i64 = sqlx::query_scalar("PRAGMA user_version")
    .fetch_one(&pool)
    .await
    .unwrap();
  assert_eq!(version, SCHEMA_VERSION);
  let repo = MessageRepository::new(pool);
  assert_eq!(
    repo.get(&id).await.unwrap().subject.as_deref(),
    Some("kept")
  );
}
