//! A rustmail from before schema versioning must fail on a schema-1 file
//! before it writes anything to it.
//!
//! Such a binary never reads `user_version`; it runs its own
//! `CREATE ... IF NOT EXISTS` statements against whatever file it is given.
//! [`initialize_as_v0_7_0`] replays those statements, vendored verbatim from
//! `v0.7.0:crates/rustmail-storage/src/schema.rs`, which is also the last tag
//! before phase 4.

use std::path::{Path, PathBuf};

use rustmail_storage::{MessageRepository, SCHEMA_VERSION, connect_options, initialize_database};
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use ulid::Ulid;

const V0_7_0_MESSAGES: &str = r#"
        CREATE TABLE IF NOT EXISTS messages (
            id              TEXT PRIMARY KEY,
            sender          TEXT NOT NULL,
            recipients      TEXT NOT NULL,
            subject         TEXT,
            text_body       TEXT,
            html_body       TEXT,
            raw             BLOB NOT NULL,
            size            INTEGER NOT NULL,
            has_attachments INTEGER NOT NULL DEFAULT 0,
            is_read         INTEGER NOT NULL DEFAULT 0,
            is_starred      INTEGER NOT NULL DEFAULT 0,
            tags            TEXT NOT NULL DEFAULT '[]',
            created_at      TEXT NOT NULL
        )
        "#;
const V0_7_0_ADDED_COLUMNS: &[(&str, &str, &str)] = &[
  (
    "messages",
    "is_starred",
    "is_starred INTEGER NOT NULL DEFAULT 0",
  ),
  ("messages", "tags", "tags TEXT NOT NULL DEFAULT '[]'"),
];
const V0_7_0_AFTER_COLUMNS: &[&str] = &[
  r#"
        CREATE TABLE IF NOT EXISTS attachments (
            id           TEXT PRIMARY KEY,
            message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            filename     TEXT,
            content_type TEXT,
            content_id   TEXT,
            size         INTEGER,
            content      BLOB NOT NULL
        )
        "#,
  r#"
        CREATE INDEX IF NOT EXISTS idx_attachments_content_id
        ON attachments(message_id, content_id)
        WHERE content_id IS NOT NULL
        "#,
  r#"
        CREATE INDEX IF NOT EXISTS idx_attachments_message_id
        ON attachments(message_id)
        "#,
  r#"
        CREATE INDEX IF NOT EXISTS idx_messages_created_at
        ON messages(created_at)
        "#,
  r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            subject,
            text_body,
            sender,
            recipients,
            content='messages',
            content_rowid='rowid'
        )
        "#,
  "PRAGMA journal_mode=WAL",
];

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

fn file_url(path: &Path) -> String {
  format!("sqlite://{}?mode=rwc", path.display())
}

async fn initialize_as_v0_7_0(pool: &SqlitePool) -> Result<(), sqlx::Error> {
  sqlx::query(V0_7_0_MESSAGES).execute(pool).await?;
  for (table, column, definition) in V0_7_0_ADDED_COLUMNS {
    let exists: Option<(String,)> =
      sqlx::query_as("SELECT name FROM pragma_table_info(?) WHERE name = ?")
        .bind(table)
        .bind(column)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
      sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {definition}"))
        .execute(pool)
        .await?;
    }
  }
  for statement in V0_7_0_AFTER_COLUMNS {
    sqlx::query(statement).execute(pool).await?;
  }
  Ok(())
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
