use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteAutoVacuum, SqliteConnectOptions};
use sqlx::{Connection, SqliteConnection, SqlitePool};

use crate::StorageError;

pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Schema version this binary reads and writes, kept in `PRAGMA user_version`.
///
/// Schema 1 keeps each message's metadata in `messages` and its bodies and
/// raw source in `message_content`, so listing, counting and searching never
/// read a body. A file with a higher version was written by a newer rustmail
/// and is refused.
pub const SCHEMA_VERSION: i64 = 1;
/// `user_version` of every file written before schema versioning existed.
const LEGACY_SCHEMA_VERSION: i64 = 0;
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

/// Statements that create schema 1 in an empty file, in order.
///
/// `messages` holds only what a listing reads, so a list, count or filter
/// never walks a body's overflow chain. Bodies sit ahead of `raw` in
/// `message_content`, so reading a message's bodies stops before its raw
/// source. `seq` is an explicit `INTEGER PRIMARY KEY`: it is arrival order,
/// the cursor position and the FTS rowid, and `VACUUM` never renumbers it.
///
/// The full-text index keeps its external content, now read through a view
/// joining both tables on their primary keys. `attachments` has no
/// `message_id` and none of the legacy index names, so a rustmail from before
/// schema versioning fails on its first `CREATE INDEX` against this file,
/// before it writes anything.
///
/// `messages.tags` stays the JSON array the API returns, order and duplicates
/// included. `message_tags` indexes it, one row per distinct tag, so the tag
/// filter is a primary-key lookup. Triggers rewrite a message's rows whenever
/// its `tags` change, and the rows follow a deleted message by cascade, so
/// every write path keeps the two equal without touching `message_tags`
/// itself. `idx_messages_unread` holds only unread messages, like the starred
/// index.
const SCHEMA_1_DDL: &[&str] = &[
  r#"
  CREATE TABLE messages (
    seq             INTEGER PRIMARY KEY,
    id              TEXT NOT NULL UNIQUE,
    sender          TEXT NOT NULL,
    recipients      TEXT NOT NULL,
    subject         TEXT,
    size            INTEGER NOT NULL,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    is_read         INTEGER NOT NULL DEFAULT 0,
    is_starred      INTEGER NOT NULL DEFAULT 0,
    tags            TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL
  )
  "#,
  "CREATE INDEX idx_messages_created_at ON messages(created_at)",
  "CREATE INDEX idx_messages_starred ON messages(is_starred) WHERE is_starred = 1",
  "CREATE INDEX idx_messages_unread ON messages(is_read) WHERE is_read = 0",
  r#"
  CREATE TABLE message_content (
    seq       INTEGER PRIMARY KEY REFERENCES messages(seq) ON DELETE CASCADE,
    text_body TEXT,
    html_body TEXT,
    raw       BLOB NOT NULL
  )
  "#,
  r#"
  CREATE VIEW messages_fts_source AS
    SELECT m.seq AS seq, m.subject AS subject, c.text_body AS text_body,
           m.sender AS sender, m.recipients AS recipients
    FROM messages m JOIN message_content c ON c.seq = m.seq
  "#,
  r#"
  CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject,
    text_body,
    sender,
    recipients,
    content='messages_fts_source',
    content_rowid='seq'
  )
  "#,
  r#"
  CREATE TABLE attachments (
    id           TEXT PRIMARY KEY,
    message_seq  INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    filename     TEXT,
    content_type TEXT,
    content_id   TEXT,
    size         INTEGER,
    content      BLOB NOT NULL
  )
  "#,
  "CREATE INDEX idx_attachments_by_message ON attachments(message_seq)",
  r#"
  CREATE INDEX idx_attachments_by_cid ON attachments(message_seq, content_id)
  WHERE content_id IS NOT NULL
  "#,
  r#"
  CREATE TABLE message_tags (
    tag         TEXT NOT NULL,
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    PRIMARY KEY (tag, message_seq)
  ) WITHOUT ROWID
  "#,
  "CREATE INDEX idx_message_tags_by_message ON message_tags(message_seq)",
  r#"
  CREATE TRIGGER message_tags_after_insert AFTER INSERT ON messages
  WHEN NEW.tags <> '[]'
  BEGIN
    INSERT OR IGNORE INTO message_tags(tag, message_seq)
      SELECT value, NEW.seq FROM json_each(NEW.tags);
  END
  "#,
  r#"
  CREATE TRIGGER message_tags_after_update AFTER UPDATE OF tags ON messages
  WHEN NEW.tags IS NOT OLD.tags
  BEGIN
    DELETE FROM message_tags WHERE message_seq = NEW.seq;
    INSERT OR IGNORE INTO message_tags(tag, message_seq)
      SELECT value, NEW.seq FROM json_each(NEW.tags);
  END
  "#,
];

/// Builds connection options for `db_url` with RustMail's SQLite tuning.
///
/// Every pragma here is per-connection, so it must be applied when a
/// connection is opened rather than once against the pool: a pragma issued
/// through the pool reaches whichever single connection happened to serve it
/// and leaves the rest of the pool on SQLite's defaults.
///
/// `auto_vacuum` only takes effect before the first table exists, which is
/// on a fresh file or a fresh in-memory database; on any other file SQLite
/// ignores it without writing. sqlx issues it ahead of every other pragma.
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
      .auto_vacuum(SqliteAutoVacuum::Incremental)
      .busy_timeout(BUSY_TIMEOUT)
      .foreign_keys(true)
      .pragma("synchronous", "NORMAL")
      .pragma("cache_size", CACHE_SIZE_KIB)
      .pragma("mmap_size", MMAP_SIZE_BYTES)
      .pragma("temp_store", "MEMORY")
      .pragma("wal_autocheckpoint", WAL_AUTOCHECKPOINT_PAGES),
  )
}

/// A database this binary can open, as found before anything is written.
enum Layout {
  /// No tables yet: schema 1 is created.
  Empty,
  /// Schema 1, ready to use.
  Current,
}

/// Opens the database at [`SCHEMA_VERSION`], creating schema 1 in an empty
/// file, and switches it to WAL.
///
/// The file's `user_version` and tables are read before anything is written,
/// and every refusal leaves the file untouched:
///
/// | `user_version` | Contents | Result |
/// |---|---|---|
/// | 0 | no tables | schema 1 is created |
/// | 0 | a `messages` table with a `raw` column | [`StorageError::LegacySchema`] |
/// | 0 | anything else | [`StorageError::UnrecognizedSchema`] |
/// | 1 | – | opened |
/// | > 1 | – | [`StorageError::NewerSchema`] |
///
/// Creating schema 1 is one `IMMEDIATE` transaction whose last statement sets
/// `user_version`, so a crash midway leaves an empty file, and a second
/// process racing to create the same file finds the finished schema instead.
///
/// # Errors
///
/// Returns [`StorageError::NewerSchema`], [`StorageError::LegacySchema`] or
/// [`StorageError::UnrecognizedSchema`] if the file cannot be opened at this
/// schema, and [`StorageError::Database`] if any SQL statement fails.
pub async fn initialize_database(pool: &SqlitePool) -> Result<(), StorageError> {
  let mut conn = pool.acquire().await?;
  if let Layout::Empty = inspect_layout(&mut conn).await? {
    create_schema(&mut conn).await?;
  }
  sqlx::query("PRAGMA journal_mode=WAL")
    .execute(&mut *conn)
    .await?;
  Ok(())
}

async fn create_schema(conn: &mut SqliteConnection) -> Result<(), StorageError> {
  let mut txn = conn.begin_with("BEGIN IMMEDIATE").await?;
  if let Layout::Current = inspect_layout(&mut txn).await? {
    return Ok(());
  }
  for statement in SCHEMA_1_DDL {
    sqlx::query(statement).execute(&mut *txn).await?;
  }
  sqlx::query(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
    .execute(&mut *txn)
    .await?;
  txn.commit().await?;
  Ok(())
}

async fn inspect_layout(conn: &mut SqliteConnection) -> Result<Layout, StorageError> {
  let found: i64 = sqlx::query_scalar("PRAGMA user_version")
    .fetch_one(&mut *conn)
    .await?;
  if found == SCHEMA_VERSION {
    return Ok(Layout::Current);
  }
  if found > SCHEMA_VERSION {
    return Err(StorageError::NewerSchema {
      database: database_name(conn).await?,
      found,
      supported: SCHEMA_VERSION,
    });
  }
  let tables: Vec<String> = sqlx::query_scalar(
    r"SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\_%' ESCAPE '\' ORDER BY name",
  )
  .fetch_all(&mut *conn)
  .await?;
  if found == LEGACY_SCHEMA_VERSION && tables.is_empty() {
    return Ok(Layout::Empty);
  }
  let has_legacy_messages: bool = sqlx::query_scalar(
    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name = 'raw')",
  )
  .fetch_one(&mut *conn)
  .await?;
  let database = database_name(conn).await?;
  if found == LEGACY_SCHEMA_VERSION && has_legacy_messages {
    return Err(StorageError::LegacySchema {
      database,
      supported: SCHEMA_VERSION,
    });
  }
  Err(StorageError::UnrecognizedSchema {
    database,
    found,
    tables,
  })
}

async fn database_name(conn: &mut SqliteConnection) -> Result<String, StorageError> {
  let file: String =
    sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name = 'main'")
      .fetch_one(&mut *conn)
      .await?;
  if file.is_empty() {
    return Ok(IN_MEMORY_DATABASE_NAME.to_string());
  }
  Ok(file)
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
  const AUTO_VACUUM_INCREMENTAL: i64 = 2;
  const SQLITE_DEFAULT_PAGE_SIZE: i64 = 4096;
  const IN_MEMORY_URL: &str = "sqlite::memory:";

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

  const NEWER_SCHEMA: i64 = SCHEMA_VERSION + 1;

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

  async fn open_in_memory() -> SqlitePool {
    SqlitePoolOptions::new()
      .connect_with(connect_options(IN_MEMORY_URL).unwrap())
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

  async fn pragma(pool: &SqlitePool, name: &str) -> i64 {
    sqlx::query_scalar(&format!("PRAGMA {name}"))
      .fetch_one(pool)
      .await
      .unwrap()
  }

  async fn user_version(pool: &SqlitePool) -> i64 {
    pragma(pool, "user_version").await
  }

  async fn query_plan(pool: &SqlitePool, sql: &str) -> String {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sql}"))
      .fetch_all(pool)
      .await
      .unwrap();
    rows
      .into_iter()
      .map(|(_, _, _, detail)| detail)
      .collect::<Vec<_>>()
      .join(" | ")
  }

  fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
  }

  async fn assert_refused_untouched(path: &std::path::Path) -> StorageError {
    let bytes_before = std::fs::read(path).unwrap();
    let modified_before = std::fs::metadata(path).unwrap().modified().unwrap();

    let pool = open_tuned(path).await;
    let error = initialize_database(&pool).await.unwrap_err();
    pool.close().await;

    assert_eq!(
      std::fs::read(path).unwrap(),
      bytes_before,
      "a refused database must be left byte-identical"
    );
    assert_eq!(
      std::fs::metadata(path).unwrap().modified().unwrap(),
      modified_before
    );
    for suffix in ["-wal", "-shm", "-journal"] {
      assert!(
        !sidecar(path, suffix).exists(),
        "refusing the database left a {suffix} file behind"
      );
    }
    error
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

    let error = assert_refused_untouched(&path).await;

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
        "{database} is schema 2, written by a newer rustmail; \
         this binary supports schema 1. Upgrade rustmail."
      )
    );
  }

  const LEGACY_MESSAGES_DDL: &str = "CREATE TABLE messages (
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
  )";

  #[tokio::test]
  async fn a_legacy_database_is_refused_untouched() {
    let (_guard, path) = scratch_database("legacy-schema");
    run_on_plain_file(
      &path,
      &[
        LEGACY_MESSAGES_DDL,
        "INSERT INTO messages (id, sender, recipients, raw, size, created_at)
         VALUES ('legacy', 'a@test.com', '[]', x'00', 1, '2026-01-01T00:00:00Z')",
      ],
    )
    .await;

    let error = assert_refused_untouched(&path).await;

    let StorageError::LegacySchema {
      ref database,
      supported,
    } = error
    else {
      panic!("expected LegacySchema, got {error:?}");
    };
    assert_eq!(supported, SCHEMA_VERSION);
    assert!(
      database.ends_with("rustmail.db"),
      "the error should name the file, got {database}"
    );
  }

  #[tokio::test]
  async fn a_database_with_unexpected_tables_is_refused_naming_them() {
    let (_guard, path) = scratch_database("foreign-schema");
    run_on_plain_file(
      &path,
      &[
        "CREATE TABLE invoices (id INTEGER PRIMARY KEY)",
        "CREATE TABLE customers (id INTEGER PRIMARY KEY)",
      ],
    )
    .await;

    let error = assert_refused_untouched(&path).await;

    let StorageError::UnrecognizedSchema {
      ref database,
      found,
      ref tables,
    } = error
    else {
      panic!("expected UnrecognizedSchema, got {error:?}");
    };
    assert_eq!(found, LEGACY_SCHEMA_VERSION);
    assert_eq!(tables, &["customers", "invoices"]);
    assert_eq!(
      error.to_string(),
      format!(
        "{database} is not a rustmail database: schema 0 with unexpected tables \
         (customers, invoices). Point rustmail at a new database file, or at one it created."
      )
    );
  }

  #[tokio::test]
  async fn a_fresh_file_is_created_at_the_current_schema() {
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
  async fn a_fresh_file_gets_incremental_auto_vacuum_and_wal() {
    let (_guard, path) = scratch_database("fresh-pragmas");

    let pool = open_tuned(&path).await;
    initialize_database(&pool).await.unwrap();

    assert_eq!(pragma(&pool, "auto_vacuum").await, AUTO_VACUUM_INCREMENTAL);
    assert_eq!(pragma(&pool, "page_size").await, SQLITE_DEFAULT_PAGE_SIZE);
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
      .fetch_one(&pool)
      .await
      .unwrap();
    assert_eq!(journal_mode, "wal");
  }

  #[tokio::test]
  async fn a_fresh_in_memory_database_gets_incremental_auto_vacuum() {
    let pool = open_in_memory().await;
    initialize_database(&pool).await.unwrap();

    assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
    assert_eq!(pragma(&pool, "auto_vacuum").await, AUTO_VACUUM_INCREMENTAL);
    assert_eq!(pragma(&pool, "page_size").await, SQLITE_DEFAULT_PAGE_SIZE);
  }

  #[tokio::test]
  async fn a_schema_1_file_reopens_without_change() {
    let (_guard, path) = scratch_database("reopen-schema");
    let pool = open_tuned(&path).await;
    initialize_database(&pool).await.unwrap();
    sqlx::query(
      "INSERT INTO messages (id, sender, recipients, size, created_at)
       VALUES ('kept', 'a@test.com', '[]', 1, '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let pool = open_tuned(&path).await;
    initialize_database(&pool).await.unwrap();

    assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM messages")
      .fetch_all(&pool)
      .await
      .unwrap();
    assert_eq!(ids, ["kept"]);
  }

  #[tokio::test]
  async fn attachments_are_indexed_by_message() {
    let pool = open_in_memory().await;
    initialize_database(&pool).await.unwrap();

    let plan = query_plan(&pool, "SELECT id FROM attachments WHERE message_seq = 1").await;
    assert!(
      plan.contains("idx_attachments_by_message"),
      "attachment lookup by message is not using its index: {plan}"
    );
  }

  #[tokio::test]
  async fn the_fts_content_view_resolves_a_row_by_both_primary_keys() {
    let pool = open_in_memory().await;
    initialize_database(&pool).await.unwrap();

    let plan = query_plan(
      &pool,
      "SELECT seq, subject, text_body, sender, recipients FROM messages_fts_source WHERE seq = 1",
    )
    .await;
    assert_eq!(
      plan,
      "SEARCH m USING INTEGER PRIMARY KEY (rowid=?) | SEARCH c USING INTEGER PRIMARY KEY (rowid=?)",
      "FTS5 reads its content through this lookup on every delete"
    );
  }

  #[tokio::test]
  async fn starred_messages_are_listed_from_their_own_index() {
    let pool = open_in_memory().await;
    initialize_database(&pool).await.unwrap();

    let plan = query_plan(
      &pool,
      "SELECT m.id FROM messages m WHERE 1=1 AND m.is_starred = 1 AND m.seq < 10 ORDER BY m.seq DESC LIMIT 50",
    )
    .await;
    assert!(
      plan.contains("idx_messages_starred"),
      "the starred listing scans every message: {plan}"
    );
  }
}
