use sqlx::SqlitePool;

use crate::StorageError;

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
  sqlx::query("PRAGMA synchronous=NORMAL")
    .execute(pool)
    .await?;
  sqlx::query("PRAGMA foreign_keys=ON").execute(pool).await?;
  sqlx::query("PRAGMA busy_timeout=5000")
    .execute(pool)
    .await?;
  sqlx::query("PRAGMA cache_size=-64000")
    .execute(pool)
    .await?;
  sqlx::query("PRAGMA mmap_size=268435456")
    .execute(pool)
    .await?;
  sqlx::query("PRAGMA temp_store=MEMORY")
    .execute(pool)
    .await?;

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
