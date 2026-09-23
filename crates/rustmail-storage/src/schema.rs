use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

use crate::StorageError;

pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Schema version this binary reads and writes, kept in `PRAGMA user_version`.
///
/// The current layout predates versioning, so it is schema 0: SQLite's
/// default, which every file created so far already carries. A file with a
/// higher version was written by a newer rustmail and is refused.
pub const SCHEMA_VERSION: i64 = 0;
/// Name the database is reported under when SQLite has no file path for it.
const IN_MEMORY_DATABASE_NAME: &str = "in-memory database";
/// Page cache of every connection.
///
/// Reads are served from the memory map, so a connection's own cache holds
/// little the OS page cache does not already have, and a larger one bought
/// no ingest throughput. It does stay resident: the dedicated writer keeps
/// its cache for the life of the process, and each reader keeps whatever
/// browsing a large mailbox filled it with.
const CACHE_SIZE_KIB: &str = "-8000";
const MMAP_SIZE_BYTES: &str = "268435456";
/// WAL pages that may accumulate before a commit also checkpoints.
///
/// SQLite's default is 1000 pages, roughly 4 MiB. A captured message writes
/// its raw bytes plus every decoded attachment, so a single mail with
/// attachments can fill that on its own and make almost every commit pay for
/// a checkpoint: two `fsync` calls and a copy of the WAL back into the
/// database, on the same connection that is trying to store the next message.
/// Raising the threshold batches that work into rarer, larger checkpoints.
const WAL_AUTOCHECKPOINT_PAGES: &str = "4000";

/// Builds connection options for `db_url` with RustMail's SQLite tuning.
///
/// Every pragma here is per-connection, so it must be applied when a
/// connection is opened rather than once against the pool: a pragma issued
/// through the pool reaches whichever single connection happened to serve it
/// and leaves the rest of the pool on SQLite's defaults.
///
/// `journal_mode` is deliberately not set here. WAL is recorded in the
/// database file itself, so [`initialize_database`] sets it once at startup;
/// switching it needs an exclusive lock that `busy_timeout` cannot wait on.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if `db_url` is not a valid SQLite URL.
pub fn connect_options(db_url: &str) -> Result<SqliteConnectOptions, StorageError> {
  Ok(
    SqliteConnectOptions::from_str(db_url)?
      .busy_timeout(BUSY_TIMEOUT)
      .foreign_keys(true)
      .pragma("synchronous", "NORMAL")
      .pragma("cache_size", CACHE_SIZE_KIB)
      .pragma("mmap_size", MMAP_SIZE_BYTES)
      .pragma("temp_store", "MEMORY")
      .pragma("wal_autocheckpoint", WAL_AUTOCHECKPOINT_PAGES),
  )
}

/// Creates the database schema if it does not already exist.
///
/// Sets up the `messages` table, `attachments` table, FTS5 virtual table,
/// WAL journal mode, and foreign key enforcement.
///
/// Starred messages get a partial index. Without it the starred filter reads
/// every row, each one past its raw blob to reach `is_starred`; with it, only
/// a user's star adds an entry, so capturing mail pays nothing for it.
///
/// Before any of that, the file's `user_version` is checked against
/// [`SCHEMA_VERSION`], so a database from a newer rustmail is refused without
/// a single write.
///
/// # Errors
///
/// Returns [`StorageError::NewerSchema`] if the database was written by a
/// newer schema, and [`StorageError::Database`] if any SQL statement fails.
pub async fn initialize_database(pool: &SqlitePool) -> Result<(), StorageError> {
  ensure_supported_schema(pool).await?;

  sqlx::query(
    r#"
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
        "#,
  )
  .execute(pool)
  .await?;

  add_column_if_missing(
    pool,
    "messages",
    "is_starred",
    "is_starred INTEGER NOT NULL DEFAULT 0",
  )
  .await?;
  add_column_if_missing(pool, "messages", "tags", "tags TEXT NOT NULL DEFAULT '[]'").await?;

  sqlx::query(
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
  )
  .execute(pool)
  .await?;

  sqlx::query(
    r#"
        CREATE INDEX IF NOT EXISTS idx_attachments_content_id
        ON attachments(message_id, content_id)
        WHERE content_id IS NOT NULL
        "#,
  )
  .execute(pool)
  .await?;

  sqlx::query(
    r#"
        CREATE INDEX IF NOT EXISTS idx_attachments_message_id
        ON attachments(message_id)
        "#,
  )
  .execute(pool)
  .await?;

  sqlx::query(
    r#"
        CREATE INDEX IF NOT EXISTS idx_messages_created_at
        ON messages(created_at)
        "#,
  )
  .execute(pool)
  .await?;

  sqlx::query(
    r#"
        CREATE INDEX IF NOT EXISTS idx_messages_starred
        ON messages(is_starred)
        WHERE is_starred = 1
        "#,
  )
  .execute(pool)
  .await?;

  sqlx::query(
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
  )
  .execute(pool)
  .await?;

  sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;

  Ok(())
}

async fn ensure_supported_schema(pool: &SqlitePool) -> Result<(), StorageError> {
  let found: i64 = sqlx::query_scalar("PRAGMA user_version")
    .fetch_one(pool)
    .await?;
  if found <= SCHEMA_VERSION {
    return Ok(());
  }
  let file: String =
    sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name = 'main'")
      .fetch_one(pool)
      .await?;
  let database = if file.is_empty() {
    IN_MEMORY_DATABASE_NAME.to_string()
  } else {
    file
  };
  Err(StorageError::NewerSchema {
    database,
    found,
    supported: SCHEMA_VERSION,
  })
}

async fn add_column_if_missing(
  pool: &SqlitePool,
  table: &str,
  column: &str,
  definition: &str,
) -> Result<(), StorageError> {
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
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use sqlx::sqlite::SqlitePoolOptions;
  use ulid::Ulid;

  const POOLED_CONNECTIONS: u32 = 4;
  const SYNCHRONOUS_NORMAL: i64 = 1;
  const TEMP_STORE_MEMORY: i64 = 2;
  const FOREIGN_KEYS_ON: i64 = 1;

  struct TempDir(std::path::PathBuf);
  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[tokio::test]
  async fn tuning_pragmas_reach_every_pooled_connection() {
    let dir = std::env::temp_dir().join(format!("rustmail-pragma-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = TempDir(dir.clone());
    let url = format!("sqlite://{}?mode=rwc", dir.join("test.db").display());

    let pool = SqlitePoolOptions::new()
      .min_connections(POOLED_CONNECTIONS)
      .max_connections(POOLED_CONNECTIONS)
      .connect_with(connect_options(&url).unwrap())
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();

    // Hold every connection at once so each assertion lands on a distinct one.
    let mut held = Vec::new();
    for _ in 0..POOLED_CONNECTIONS {
      held.push(pool.acquire().await.unwrap());
    }

    for (index, conn) in held.iter_mut().enumerate() {
      let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        synchronous, SYNCHRONOUS_NORMAL,
        "connection {index} did not get synchronous=NORMAL"
      );

      let temp_store: i64 = sqlx::query_scalar("PRAGMA temp_store")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        temp_store, TEMP_STORE_MEMORY,
        "connection {index} did not get temp_store=MEMORY"
      );

      let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        foreign_keys, FOREIGN_KEYS_ON,
        "connection {index} did not get foreign_keys=ON"
      );

      let cache_size: i64 = sqlx::query_scalar("PRAGMA cache_size")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        cache_size.to_string(),
        CACHE_SIZE_KIB,
        "connection {index} did not get the tuned cache_size"
      );

      let mmap_size: i64 = sqlx::query_scalar("PRAGMA mmap_size")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        mmap_size.to_string(),
        MMAP_SIZE_BYTES,
        "connection {index} did not get the tuned mmap_size"
      );

      let autocheckpoint: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        autocheckpoint.to_string(),
        WAL_AUTOCHECKPOINT_PAGES,
        "connection {index} fell back to SQLite's default checkpoint threshold, \
         which makes almost every large-message commit checkpoint the WAL"
      );

      let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(&mut **conn)
        .await
        .unwrap();
      assert_eq!(
        busy_timeout as u128,
        BUSY_TIMEOUT.as_millis(),
        "connection {index} did not get the configured busy_timeout"
      );
    }
  }

  const NEWER_SCHEMA: i64 = 1;

  fn scratch_database(prefix: &str) -> (TempDir, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rustmail-{prefix}-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rustmail.db");
    (TempDir(dir), path)
  }

  fn file_url(path: &std::path::Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
  }

  async fn open_tuned(path: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
      .connect_with(connect_options(&file_url(path)).unwrap())
      .await
      .unwrap()
  }

  async fn run_on_plain_file(path: &std::path::Path, statements: &[&str]) {
    let pool = SqlitePoolOptions::new()
      .max_connections(1)
      .connect(&file_url(path))
      .await
      .unwrap();
    for statement in statements {
      sqlx::query(statement).execute(&pool).await.unwrap();
    }
    pool.close().await;
  }

  async fn user_version(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("PRAGMA user_version")
      .fetch_one(pool)
      .await
      .unwrap()
  }

  fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
  }

  #[tokio::test]
  async fn a_database_from_a_newer_schema_is_refused_untouched() {
    let (_guard, path) = scratch_database("newer-schema");
    run_on_plain_file(
      &path,
      &[
        "CREATE TABLE message_content (seq INTEGER PRIMARY KEY, raw BLOB NOT NULL)",
        &format!("PRAGMA user_version = {NEWER_SCHEMA}"),
      ],
    )
    .await;
    let bytes_before = std::fs::read(&path).unwrap();
    let modified_before = std::fs::metadata(&path).unwrap().modified().unwrap();

    let pool = open_tuned(&path).await;
    let error = initialize_database(&pool).await.unwrap_err();
    pool.close().await;

    let StorageError::NewerSchema {
      ref database,
      found,
      supported,
    } = error
    else {
      panic!("expected NewerSchema, got {error:?}");
    };
    assert_eq!(found, NEWER_SCHEMA);
    assert_eq!(supported, SCHEMA_VERSION);
    assert!(
      database.ends_with("rustmail.db"),
      "the error should name the file, got {database}"
    );
    assert_eq!(
      error.to_string(),
      format!(
        "{database} is schema 1, written by a newer rustmail; \
         this binary supports schema 0. Upgrade rustmail."
      )
    );

    assert_eq!(
      std::fs::read(&path).unwrap(),
      bytes_before,
      "a refused database must be left byte-identical"
    );
    assert_eq!(
      std::fs::metadata(&path).unwrap().modified().unwrap(),
      modified_before
    );
    for suffix in ["-wal", "-shm", "-journal"] {
      assert!(
        !sidecar(&path, suffix).exists(),
        "refusing the database left a {suffix} file behind"
      );
    }
  }

  #[tokio::test]
  async fn a_fresh_database_stays_at_the_current_schema() {
    let (_guard, path) = scratch_database("fresh-schema");

    let pool = open_tuned(&path).await;
    initialize_database(&pool).await.unwrap();

    assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
      .fetch_one(&pool)
      .await
      .unwrap();
    assert_eq!(stored, 0);
  }

  #[tokio::test]
  async fn a_legacy_database_opens_as_before() {
    let (_guard, path) = scratch_database("legacy-schema");
    run_on_plain_file(
      &path,
      &[
        "CREATE TABLE messages (
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
            created_at      TEXT NOT NULL
        )",
        "INSERT INTO messages (id, sender, recipients, raw, size, created_at)
         VALUES ('legacy', 'a@test.com', '[]', x'00', 1, '2026-01-01T00:00:00Z')",
      ],
    )
    .await;

    let pool = open_tuned(&path).await;
    initialize_database(&pool).await.unwrap();

    assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
    let (id, is_starred, tags): (String, i64, String) =
      sqlx::query_as("SELECT id, is_starred, tags FROM messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
      (id.as_str(), is_starred, tags.as_str()),
      ("legacy", 0, "[]")
    );
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
      .fetch_one(&pool)
      .await
      .unwrap();
    assert_eq!(journal_mode, "wal");
  }

  #[tokio::test]
  async fn attachments_are_indexed_by_message() {
    let pool = SqlitePoolOptions::new()
      .connect_with(connect_options("sqlite::memory:").unwrap())
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();

    // Without this index every attachment listing, and every cascade from a
    // deleted message, scans the whole attachments table.
    let rows: Vec<(i64, i64, i64, String)> =
      sqlx::query_as("EXPLAIN QUERY PLAN SELECT id FROM attachments WHERE message_id = 'x'")
        .fetch_all(&pool)
        .await
        .unwrap();

    let plan = rows
      .into_iter()
      .map(|(_, _, _, detail)| detail)
      .collect::<Vec<_>>()
      .join(" ");
    assert!(
      plan.contains("idx_attachments_message_id"),
      "attachment lookup by message is not using its index: {plan}"
    );
  }

  #[tokio::test]
  async fn starred_messages_are_listed_from_their_own_index() {
    let pool = SqlitePoolOptions::new()
      .connect_with(connect_options("sqlite::memory:").unwrap())
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();

    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(
      "EXPLAIN QUERY PLAN SELECT m.id FROM messages m WHERE 1=1 AND m.is_starred = 1 AND m.rowid < 10 ORDER BY m.rowid DESC LIMIT 50",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let plan = rows
      .into_iter()
      .map(|(_, _, _, detail)| detail)
      .collect::<Vec<_>>()
      .join(" ");
    assert!(
      plan.contains("idx_messages_starred"),
      "the starred listing scans every message: {plan}"
    );
  }
}
