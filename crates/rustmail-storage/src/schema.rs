use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

use crate::StorageError;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_SIZE_KIB: &str = "-64000";
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
/// # Errors
///
/// Returns [`StorageError::Database`] if any SQL statement fails.
pub async fn initialize_database(pool: &SqlitePool) -> Result<(), StorageError> {
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
        CREATE INDEX IF NOT EXISTS idx_messages_created_at
        ON messages(created_at)
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

}
