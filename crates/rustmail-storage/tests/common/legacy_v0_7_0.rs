//! The storage layer of rustmail v0.7.0, the last tag before phase 4, for
//! building legacy databases in tests.
//!
//! The DDL and the insert and update statements are vendored verbatim from
//! `v0.7.0:crates/rustmail-storage/src/{schema,repo}.rs`; only the plumbing
//! around them is new. Several test crates include this file, each using the
//! part it needs, so unused items are allowed here.
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;

use mail_parser::{MessageParser, MimeHeaders, PartType};
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

pub const V0_7_0_MESSAGES: &str = r#"
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
pub const V0_7_0_ADDED_COLUMNS: &[(&str, &str, &str)] = &[
  (
    "messages",
    "is_starred",
    "is_starred INTEGER NOT NULL DEFAULT 0",
  ),
  ("messages", "tags", "tags TEXT NOT NULL DEFAULT '[]'"),
];
pub const V0_7_0_AFTER_COLUMNS: &[&str] = &[
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

/// The summary v0.7.0's insert returned for a stored message.
#[derive(Debug, Clone)]
pub struct LegacySummary {
  pub id: String,
  pub sender: String,
  pub recipients: String,
  pub subject: Option<String>,
  pub size: i64,
  pub has_attachments: bool,
  pub created_at: String,
  /// The attachment ids minted, in the order the rows were written.
  pub attachment_ids: Vec<String>,
}

pub fn file_url(path: &Path) -> String {
  format!("sqlite://{}?mode=rwc", path.display())
}

/// Opens `path` on one connection, as a plain SQLite client would.
pub async fn open_plain(path: &Path) -> SqlitePool {
  SqlitePoolOptions::new()
    .max_connections(1)
    .connect(&file_url(path))
    .await
    .unwrap()
}

/// Runs v0.7.0's `initialize_database` against `pool`.
pub async fn initialize_as_v0_7_0(pool: &SqlitePool) -> Result<(), sqlx::Error> {
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

/// Creates a legacy database at `path` and returns a pool on it.
pub async fn create_legacy_database(path: &Path) -> SqlitePool {
  let pool = open_plain(path).await;
  initialize_as_v0_7_0(&pool).await.unwrap();
  pool
}

/// Stores a message exactly as v0.7.0's `MessageRepository::insert` did,
/// with `created_at` given rather than read from the clock.
pub async fn insert_as_v0_7_0(
  pool: &SqlitePool,
  sender: &str,
  recipients: &[String],
  raw: &[u8],
  created_at: &str,
) -> Result<LegacySummary, sqlx::Error> {
  let id = ulid::Ulid::new().to_string();
  let recipients_json = serde_json::to_string(recipients).unwrap_or_default();
  let size = raw.len() as i64;

  let parsed = MessageParser::default().parse(raw);

  let (subject, text_body, html_body, has_attachments) = match &parsed {
    Some(msg) => (
      msg.subject().map(String::from),
      msg.body_text(0).map(|s| s.into_owned()),
      msg.body_html(0).map(|s| s.into_owned()),
      msg.attachment_count() > 0,
    ),
    None => (None, None, None, false),
  };

  let mut txn = pool.begin().await?;

  sqlx::query(
    r#"
      INSERT INTO messages (id, sender, recipients, subject, text_body, html_body, raw, size, has_attachments, is_read, is_starred, tags, created_at)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, '[]', ?10)
      "#,
  )
  .bind(&id)
  .bind(sender)
  .bind(&recipients_json)
  .bind(&subject)
  .bind(&text_body)
  .bind(&html_body)
  .bind(raw)
  .bind(size)
  .bind(has_attachments)
  .bind(created_at)
  .execute(&mut *txn)
  .await?;

  sqlx::query(
    "INSERT INTO messages_fts(rowid, subject, text_body, sender, recipients) SELECT rowid, ?2, ?3, ?4, ?5 FROM messages WHERE id = ?1",
  )
  .bind(&id)
  .bind(&subject)
  .bind(&text_body)
  .bind(sender)
  .bind(&recipients_json)
  .execute(&mut *txn)
  .await?;

  let mut minted = Vec::new();
  if let Some(parsed_msg) = &parsed {
    let attachment_ids: HashSet<u32> = parsed_msg.attachments.iter().copied().collect();

    for (idx, part) in parsed_msg.parts.iter().enumerate() {
      let is_attachment = attachment_ids.contains(&(idx as u32));
      let cid = part.content_id().map(String::from);
      let is_inline_binary = matches!(part.body, PartType::InlineBinary(_));

      if !is_attachment && !is_inline_binary {
        continue;
      }

      let content = part.contents();
      if content.is_empty() {
        continue;
      }

      let att_id = ulid::Ulid::new().to_string();
      let filename = part.attachment_name().map(String::from);
      let content_type =
        part
          .content_type()
          .map(|ct: &mail_parser::ContentType| match ct.subtype() {
            Some(subtype) => format!("{}/{}", ct.ctype(), subtype),
            None => ct.ctype().to_string(),
          });
      let att_size = content.len() as i64;

      sqlx::query(
        r#"
          INSERT INTO attachments (id, message_id, filename, content_type, content_id, size, content)
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
          "#,
      )
      .bind(&att_id)
      .bind(&id)
      .bind(&filename)
      .bind(&content_type)
      .bind(&cid)
      .bind(att_size)
      .bind(content)
      .execute(&mut *txn)
      .await?;
      minted.push(att_id);
    }
  }

  txn.commit().await?;

  Ok(LegacySummary {
    id,
    sender: sender.to_string(),
    recipients: recipients_json,
    subject,
    size,
    has_attachments,
    created_at: created_at.to_string(),
    attachment_ids: minted,
  })
}

/// Sets a message's flags and tags as v0.7.0's `update_message` did.
pub async fn update_as_v0_7_0(
  pool: &SqlitePool,
  id: &str,
  is_read: Option<bool>,
  is_starred: Option<bool>,
  tags: Option<&[String]>,
) -> Result<(), sqlx::Error> {
  let mut txn = pool.begin().await?;
  if let Some(is_read) = is_read {
    sqlx::query("UPDATE messages SET is_read = ?1 WHERE id = ?2")
      .bind(is_read)
      .bind(id)
      .execute(&mut *txn)
      .await?;
  }
  if let Some(is_starred) = is_starred {
    sqlx::query("UPDATE messages SET is_starred = ?1 WHERE id = ?2")
      .bind(is_starred)
      .bind(id)
      .execute(&mut *txn)
      .await?;
  }
  if let Some(tags) = tags {
    let tags_json = serde_json::to_string(tags).unwrap_or_default();
    sqlx::query("UPDATE messages SET tags = ?1 WHERE id = ?2")
      .bind(&tags_json)
      .bind(id)
      .execute(&mut *txn)
      .await?;
  }
  txn.commit().await?;
  Ok(())
}
