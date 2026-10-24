//! Building the current-schema copy of an older database, batch by batch.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mail_parser::MessageParser;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Connection, SqliteConnection};
use time::OffsetDateTime;
use tracing::info;

use super::MigrationOptions;
use super::fingerprint::{SourceLayout, SourceSchema};
use crate::StorageError;
use crate::content::{compress, decompress};
use crate::locator::{Locator, locate};
use crate::prepared::{AttachmentStorage, stored_part_refs};
use crate::repo::format_iso8601;
use crate::schema::{SCHEMA_DDL, SCHEMA_VERSION, tuned};

/// Schema name the source database is attached under on the copy's connection.
const SOURCE_SCHEMA: &str = "src";
/// How often the copy logs its progress.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

/// Records where the copy came from and how far it got.
///
/// Kept in the finished file, so the pre-swap check, a later start and
/// `restore-backup` can always tell which database a copy was made from.
/// Copies made by rustmail 0.8 and 0.9 lack `source_schema` and
/// `target_schema`; they were all built from schema 0 into schema 1.
/// `source_fingerprint` holds the [`qualified_fingerprint`], or the bare one
/// in copies made by 0.8, 0.9 and 0.10.0.
const MIGRATION_ORIGIN_DDL: &str = r#"
  CREATE TABLE migration_origin (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),
    source_fingerprint    TEXT NOT NULL,
    last_source_seq       INTEGER NOT NULL,
    attachment_mismatches INTEGER NOT NULL DEFAULT 0,
    completed_at          TEXT,
    source_schema         INTEGER NOT NULL,
    target_schema         INTEGER NOT NULL
  )
"#;
/// `source_schema` of a copy whose origin row predates the column.
const ORIGIN_DEFAULT_SOURCE_SCHEMA: i64 = 0;
/// `target_schema` of a copy whose origin row predates the column.
const ORIGIN_DEFAULT_TARGET_SCHEMA: i64 = 1;

/// What a copy records about its origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Origin {
  /// Fingerprint of the database the copy is made from.
  pub(crate) fingerprint: String,
  /// `seq` of the last source message copied.
  pub(crate) last_source_seq: i64,
  /// `user_version` of the database the copy is made from.
  pub(crate) source_schema: i64,
  /// `user_version` the copy is built at.
  pub(crate) target_schema: i64,
}

impl Origin {
  /// Whether the copy was made from the database whose fingerprint is
  /// `fingerprint`, stored in either form.
  pub(crate) fn made_from(&self, fingerprint: &str) -> bool {
    self.fingerprint == fingerprint
      || self.fingerprint == qualified_fingerprint(fingerprint, self.target_schema)
  }
}

/// `fingerprint` as a copy built at `target_schema` stores it.
///
/// Rustmail 0.8, 0.9 and 0.10.0 compare the stored value with the bare
/// fingerprint of the database beside the copy, so they never take a copy
/// that stores this form for one made from it: they rebuild it instead of
/// resuming it or swapping it in.
pub(crate) fn qualified_fingerprint(fingerprint: &str, target_schema: i64) -> String {
  format!("schema{target_schema}:{fingerprint}")
}

/// Rewrites the bare `fingerprint` in the origin of the copy at `path` to its
/// qualified form, so older rustmail versions stop taking the copy for one
/// of their own.
pub(crate) async fn qualify_origin(path: &Path, fingerprint: &str) -> Result<(), StorageError> {
  let mut conn = SqliteConnection::connect_with(&super::existing_file(path)).await?;
  let updated = sqlx::query(
    "UPDATE migration_origin SET source_fingerprint = ?1 WHERE id = 1 AND source_fingerprint = ?2",
  )
  .bind(qualified_fingerprint(fingerprint, SCHEMA_VERSION))
  .bind(fingerprint)
  .execute(&mut conn)
  .await;
  conn.close().await?;
  updated?;
  Ok(())
}

/// Reads the origin row of the copy open on `conn`, if it has one.
pub(crate) async fn read_origin(
  conn: &mut SqliteConnection,
) -> Result<Option<Origin>, StorageError> {
  let has_table: bool = sqlx::query_scalar(
    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'migration_origin')",
  )
  .fetch_one(&mut *conn)
  .await?;
  if !has_table {
    return Ok(None);
  }
  let has_schemas: bool = sqlx::query_scalar(
    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('migration_origin') WHERE name = 'source_schema')",
  )
  .fetch_one(&mut *conn)
  .await?;
  let columns = if has_schemas {
    "source_schema, target_schema"
  } else {
    "?1, ?2"
  };
  let row: Option<(String, i64, i64, i64)> = sqlx::query_as(&format!(
    "SELECT source_fingerprint, last_source_seq, {columns} FROM migration_origin WHERE id = 1"
  ))
  .bind(ORIGIN_DEFAULT_SOURCE_SCHEMA)
  .bind(ORIGIN_DEFAULT_TARGET_SCHEMA)
  .fetch_optional(&mut *conn)
  .await?;
  Ok(row.map(
    |(fingerprint, last_source_seq, source_schema, target_schema)| Origin {
      fingerprint,
      last_source_seq,
      source_schema,
      target_schema,
    },
  ))
}

/// Attachment counts of a finished copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AttachmentCounts {
  pub(crate) located: u64,
  pub(crate) inline: u64,
  pub(crate) mismatches: u64,
}

/// Whether the batch loop ran to the end of the source table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyOutcome {
  /// Every source message is in the copy.
  Finished,
  /// Stopped between batches; `migrated` of `total` messages are copied.
  Paused { migrated: u64, total: u64 },
}

/// The copy being built, open with the source database attached.
pub(crate) struct Target {
  conn: SqliteConnection,
  path: PathBuf,
  source: PathBuf,
}

fn target_options(path: &Path, options: &MigrationOptions) -> SqliteConnectOptions {
  let tuned = tuned(
    SqliteConnectOptions::new()
      .filename(path)
      .create_if_missing(false),
  )
  .journal_mode(SqliteJournalMode::Wal);
  match options.target_max_pages {
    Some(pages) => tuned.pragma("max_page_count", pages.to_string()),
    None => tuned,
  }
}

impl Target {
  /// Creates the current schema and the origin row in the empty file at
  /// `path`, without setting `user_version`.
  pub(crate) async fn create(
    path: &Path,
    source: &Path,
    source_schema: SourceSchema,
    fingerprint: &str,
    options: &MigrationOptions,
  ) -> Result<Self, StorageError> {
    let mut conn = SqliteConnection::connect_with(&target_options(path, options)).await?;
    let mut txn = conn.begin_with("BEGIN IMMEDIATE").await?;
    for statement in SCHEMA_DDL {
      sqlx::query(statement).execute(&mut *txn).await?;
    }
    sqlx::query(MIGRATION_ORIGIN_DDL).execute(&mut *txn).await?;
    sqlx::query(
      "INSERT INTO migration_origin (id, source_fingerprint, last_source_seq, source_schema, target_schema) VALUES (1, ?1, 0, ?2, ?3)",
    )
    .bind(qualified_fingerprint(fingerprint, SCHEMA_VERSION))
    .bind(source_schema.version())
    .bind(SCHEMA_VERSION)
    .execute(&mut *txn)
    .await?;
    txn.commit().await?;
    Self::attach(conn, path, source).await
  }

  /// Reopens an incomplete copy at `path` to resume it.
  pub(crate) async fn open(
    path: &Path,
    source: &Path,
    options: &MigrationOptions,
  ) -> Result<Self, StorageError> {
    let conn = SqliteConnection::connect_with(&target_options(path, options)).await?;
    Self::attach(conn, path, source).await
  }

  async fn attach(
    mut conn: SqliteConnection,
    path: &Path,
    source: &Path,
  ) -> Result<Self, StorageError> {
    sqlx::query(&format!("ATTACH DATABASE ?1 AS {SOURCE_SCHEMA}"))
      .bind(source.to_string_lossy().into_owned())
      .execute(&mut conn)
      .await?;
    Ok(Self {
      conn,
      path: path.to_path_buf(),
      source: source.to_path_buf(),
    })
  }

  /// Closes the copy's connection, checkpointing its WAL.
  pub(crate) async fn close(self) -> Result<(), StorageError> {
    self.conn.close().await?;
    Ok(())
  }

  /// How the attached source, in `schema`, is laid out.
  pub(crate) async fn source_layout(
    &mut self,
    schema: SourceSchema,
  ) -> Result<SourceLayout, StorageError> {
    SourceLayout::probe(&mut self.conn, SOURCE_SCHEMA, schema).await
  }

  async fn scalar(&mut self, sql: &str) -> Result<i64, StorageError> {
    Ok(sqlx::query_scalar(sql).fetch_one(&mut self.conn).await?)
  }

  /// Copies every source message after the origin's `last_source_seq`, one
  /// committed batch at a time, asking `should_stop` before each batch.
  pub(crate) async fn copy(
    &mut self,
    layout: &SourceLayout,
    options: &MigrationOptions,
    total: u64,
    migration_id: &str,
    should_stop: &(dyn Fn() -> bool + Send + Sync),
  ) -> Result<CopyOutcome, StorageError> {
    let mut last_seq: i64 =
      sqlx::query_scalar("SELECT last_source_seq FROM migration_origin WHERE id = 1")
        .fetch_one(&mut self.conn)
        .await?;
    let mut migrated =
      u64::try_from(self.scalar("SELECT count(*) FROM messages").await?).unwrap_or(0);
    let mut progress = Progress::new(migrated);
    loop {
      if should_stop() {
        return Ok(CopyOutcome::Paused { migrated, total });
      }
      let Some(batch) = self.next_batch(layout, last_seq, options).await? else {
        return Ok(CopyOutcome::Finished);
      };
      self.copy_batch(layout, last_seq, &batch).await?;
      last_seq = batch.last_seq;
      migrated += batch.messages;
      progress.report(migration_id, migrated, total);
    }
  }

  async fn next_batch(
    &mut self,
    layout: &SourceLayout,
    after: i64,
    options: &MigrationOptions,
  ) -> Result<Option<Batch>, StorageError> {
    let sizes = match layout {
      SourceLayout::Legacy(_) => format!(
        "SELECT rowid, length(raw) FROM {SOURCE_SCHEMA}.messages WHERE rowid > ?1 ORDER BY rowid LIMIT ?2"
      ),
      SourceLayout::Previous => format!(
        "SELECT seq, size FROM {SOURCE_SCHEMA}.messages WHERE seq > ?1 ORDER BY seq LIMIT ?2"
      ),
    };
    let rows: Vec<(i64, i64)> = sqlx::query_as(&sizes)
      .bind(after)
      .bind(options.batch_messages)
      .fetch_all(&mut self.conn)
      .await?;
    let mut batch: Option<Batch> = None;
    for (rowid, raw_len) in rows {
      match &mut batch {
        None => {
          batch = Some(Batch {
            last_seq: rowid,
            messages: 1,
            raw_bytes: raw_len,
          });
        }
        Some(open) if open.raw_bytes.saturating_add(raw_len) <= options.batch_raw_bytes => {
          open.last_seq = rowid;
          open.messages += 1;
          open.raw_bytes += raw_len;
        }
        Some(_) => break,
      }
    }
    Ok(batch)
  }

  async fn copy_batch(
    &mut self,
    layout: &SourceLayout,
    after: i64,
    batch: &Batch,
  ) -> Result<(), StorageError> {
    let planned = match layout {
      SourceLayout::Legacy(columns) => {
        self
          .plan_attachments(columns.has_attachments_table, after, batch.last_seq)
          .await?
      }
      SourceLayout::Previous => Vec::new(),
    };
    let mismatches = planned
      .iter()
      .filter(|attachment| attachment.mismatch)
      .count();
    let content = self
      .compressed_content(layout, after, batch.last_seq)
      .await?;
    let (messages_sql, fts_sql) = match layout {
      SourceLayout::Legacy(columns) => (
        format!(
          "INSERT INTO messages (seq, id, sender, recipients, subject, size, has_attachments, is_read, is_starred, tags, created_at) \
           SELECT rowid, id, sender, recipients, subject, length(raw), has_attachments, is_read, {}, {}, created_at \
           FROM {SOURCE_SCHEMA}.messages WHERE rowid > ?1 AND rowid <= ?2 ORDER BY rowid",
          columns.is_starred, columns.tags
        ),
        format!(
          "INSERT INTO messages_fts (rowid, subject, text_body, sender, recipients) \
           SELECT rowid, subject, text_body, sender, recipients FROM {SOURCE_SCHEMA}.messages \
           WHERE rowid > ?1 AND rowid <= ?2 ORDER BY rowid"
        ),
      ),
      SourceLayout::Previous => (
        format!(
          "INSERT INTO messages (seq, id, sender, recipients, subject, size, has_attachments, is_read, is_starred, tags, created_at) \
           SELECT m.seq, m.id, m.sender, m.recipients, m.subject, length(c.raw), m.has_attachments, m.is_read, m.is_starred, m.tags, m.created_at \
           FROM {SOURCE_SCHEMA}.messages m JOIN {SOURCE_SCHEMA}.message_content c ON c.seq = m.seq \
           WHERE m.seq > ?1 AND m.seq <= ?2 ORDER BY m.seq"
        ),
        format!(
          "INSERT INTO messages_fts (rowid, subject, text_body, sender, recipients) \
           SELECT m.seq, m.subject, c.text_body, m.sender, m.recipients \
           FROM {SOURCE_SCHEMA}.messages m JOIN {SOURCE_SCHEMA}.message_content c ON c.seq = m.seq \
           WHERE m.seq > ?1 AND m.seq <= ?2 ORDER BY m.seq"
        ),
      ),
    };
    let mut txn = self.conn.begin().await?;
    sqlx::query(&messages_sql)
      .bind(after)
      .bind(batch.last_seq)
      .execute(&mut *txn)
      .await?;
    for (seq, frame) in &content {
      sqlx::query("INSERT INTO message_content (seq, raw) VALUES (?1, ?2)")
        .bind(seq)
        .bind(frame)
        .execute(&mut *txn)
        .await?;
    }
    sqlx::query(&fts_sql)
      .bind(after)
      .bind(batch.last_seq)
      .execute(&mut *txn)
      .await?;
    if let SourceLayout::Previous = layout {
      sqlx::query(&format!(
        "INSERT INTO attachments (id, message_seq, filename, content_type, content_id, size, raw_offset, raw_len, transfer_encoding, content) \
         SELECT id, message_seq, filename, content_type, content_id, size, raw_offset, raw_len, transfer_encoding, content \
         FROM {SOURCE_SCHEMA}.attachments WHERE message_seq > ?1 AND message_seq <= ?2 ORDER BY rowid"
      ))
      .bind(after)
      .bind(batch.last_seq)
      .execute(&mut *txn)
      .await?;
    }
    for attachment in &planned {
      let (locator, content) = match &attachment.storage {
        AttachmentStorage::Located(locator) => (Some(locator), None),
        AttachmentStorage::Inline(content) => (None, Some(content)),
      };
      sqlx::query(
        "INSERT INTO attachments (id, message_seq, filename, content_type, content_id, size, raw_offset, raw_len, transfer_encoding, content) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
      )
      .bind(&attachment.meta.id)
      .bind(attachment.seq)
      .bind(&attachment.meta.filename)
      .bind(&attachment.meta.content_type)
      .bind(&attachment.meta.content_id)
      .bind(attachment.meta.size)
      .bind(locator.map(|locator| locator.offset as i64))
      .bind(locator.map(|locator| locator.len as i64))
      .bind(locator.map(|locator| locator.encoding.code()))
      .bind(content)
      .execute(&mut *txn)
      .await?;
    }
    sqlx::query(
      "UPDATE migration_origin SET last_source_seq = ?1, attachment_mismatches = attachment_mismatches + ?2 WHERE id = 1",
    )
    .bind(batch.last_seq)
    .bind(mismatches as i64)
    .execute(&mut *txn)
    .await?;
    txn.commit().await?;
    Ok(())
  }

  /// The batch's raw sources, each compressed and proven to decompress back
  /// to itself, in `seq` order.
  async fn compressed_content(
    &mut self,
    layout: &SourceLayout,
    after: i64,
    last_seq: i64,
  ) -> Result<Vec<(i64, Vec<u8>)>, StorageError> {
    let sources = match layout {
      SourceLayout::Legacy(_) => format!(
        "SELECT rowid, raw FROM {SOURCE_SCHEMA}.messages WHERE rowid > ?1 AND rowid <= ?2 ORDER BY rowid"
      ),
      SourceLayout::Previous => format!(
        "SELECT seq, raw FROM {SOURCE_SCHEMA}.message_content WHERE seq > ?1 AND seq <= ?2 ORDER BY seq"
      ),
    };
    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(&sources)
      .bind(after)
      .bind(last_seq)
      .fetch_all(&mut self.conn)
      .await?;
    let compressed = in_parallel(rows, |(seq, raw)| {
      let frame = compress(&raw)?;
      let round_trip = decompress(&frame, raw.len() as i64, &seq.to_string())?;
      Ok::<_, StorageError>((seq, frame, round_trip == raw))
    })
    .await?
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    match compressed.iter().find(|(_, _, intact)| !intact) {
      Some((seq, _, _)) => Err(self.verify_failed_at(
        "compressed source",
        format!("message {seq} decompresses to its source"),
        "different bytes".to_string(),
      )),
      None => Ok(
        compressed
          .into_iter()
          .map(|(seq, frame, _)| (seq, frame))
          .collect(),
      ),
    }
  }

  /// Decides how each legacy attachment of the batch is stored, parsing the
  /// raw sources on the blocking pool.
  async fn plan_attachments(
    &mut self,
    has_attachments_table: bool,
    after: i64,
    last_seq: i64,
  ) -> Result<Vec<PlannedAttachment>, StorageError> {
    if !has_attachments_table {
      return Ok(Vec::new());
    }
    let rows: Vec<LegacyAttachmentRow> = sqlx::query_as(&format!(
      "SELECT m.rowid AS seq, a.id, a.filename, a.content_type, a.content_id, a.size, a.content \
       FROM {SOURCE_SCHEMA}.messages m JOIN {SOURCE_SCHEMA}.attachments a ON a.message_id = m.id \
       WHERE m.rowid > ?1 AND m.rowid <= ?2 ORDER BY m.rowid, a.rowid"
    ))
    .bind(after)
    .bind(last_seq)
    .fetch_all(&mut self.conn)
    .await?;
    let mut messages: Vec<LegacyMessage> = Vec::new();
    for row in rows {
      let (seq, attachment) = row.split();
      match messages.last_mut() {
        Some(message) if message.seq == seq => message.attachments.push(attachment),
        _ => messages.push(LegacyMessage {
          seq,
          raw: Vec::new(),
          attachments: vec![attachment],
        }),
      }
    }
    for message in &mut messages {
      message.raw = sqlx::query_scalar(&format!(
        "SELECT raw FROM {SOURCE_SCHEMA}.messages WHERE rowid = ?1"
      ))
      .bind(message.seq)
      .fetch_one(&mut self.conn)
      .await?;
    }
    let mut planned: Vec<PlannedAttachment> = in_parallel(messages, plan_message)
      .await?
      .into_iter()
      .flatten()
      .collect();
    planned.sort_by_key(|attachment| attachment.seq);
    Ok(planned)
  }

  /// Checks the finished copy against the attached source database.
  pub(crate) async fn verify(
    &mut self,
    layout: &SourceLayout,
    database: &Path,
  ) -> Result<AttachmentCounts, StorageError> {
    let (legacy_attachments, legacy_tags) = match layout {
      SourceLayout::Legacy(columns) => (
        if columns.has_attachments_table {
          format!(
            "SELECT count(*) FROM {SOURCE_SCHEMA}.messages m JOIN {SOURCE_SCHEMA}.attachments a ON a.message_id = m.id"
          )
        } else {
          "SELECT 0".to_string()
        },
        format!(
          "SELECT count(*) FROM (SELECT DISTINCT s.seq, t.value FROM \
           (SELECT rowid AS seq, {} AS tags FROM {SOURCE_SCHEMA}.messages) s, json_each(s.tags) t)",
          columns.tags
        ),
      ),
      SourceLayout::Previous => (
        format!("SELECT count(*) FROM {SOURCE_SCHEMA}.attachments"),
        format!("SELECT count(*) FROM {SOURCE_SCHEMA}.message_tags"),
      ),
    };
    let legacy_messages = format!("SELECT count(*) FROM {SOURCE_SCHEMA}.messages");
    let source_raw_bytes = match layout {
      SourceLayout::Legacy(_) => {
        format!("SELECT coalesce(sum(length(raw)), 0) FROM {SOURCE_SCHEMA}.messages")
      }
      SourceLayout::Previous => {
        format!("SELECT coalesce(sum(length(raw)), 0) FROM {SOURCE_SCHEMA}.message_content")
      }
    };
    let checks: [(&'static str, &str, &str); 7] = [
      (
        "message count",
        &legacy_messages,
        "SELECT count(*) FROM messages",
      ),
      (
        "content count",
        &legacy_messages,
        "SELECT count(*) FROM message_content",
      ),
      (
        "raw bytes",
        &source_raw_bytes,
        "SELECT coalesce(sum(size), 0) FROM messages",
      ),
      (
        "attachment count",
        &legacy_attachments,
        "SELECT count(*) FROM attachments",
      ),
      (
        "tag index",
        &legacy_tags,
        "SELECT count(*) FROM message_tags",
      ),
      (
        "full-text document count",
        "SELECT count(*) FROM messages",
        "SELECT count(*) FROM messages_fts_docsize",
      ),
      (
        "full-text document ids",
        "SELECT 0",
        "SELECT count(*) FROM messages m WHERE NOT EXISTS (SELECT 1 FROM messages_fts_docsize d WHERE d.id = m.seq)",
      ),
    ];
    for (check, expected_sql, found_sql) in checks {
      let expected = self.scalar(expected_sql).await?;
      let found = self.scalar(found_sql).await?;
      if expected != found {
        return Err(self.verify_failed(database, check, expected.to_string(), found.to_string()));
      }
    }
    if let Err(error) =
      sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)")
        .execute(&mut self.conn)
        .await
    {
      return Err(self.verify_failed(
        database,
        "full-text integrity",
        "ok".to_string(),
        error.to_string(),
      ));
    }
    let violations = sqlx::query("PRAGMA main.foreign_key_check")
      .fetch_all(&mut self.conn)
      .await?
      .len();
    if violations != 0 {
      return Err(self.verify_failed(
        database,
        "foreign key",
        "0 violations".to_string(),
        format!("{violations} violations"),
      ));
    }
    let quick_check: String = sqlx::query_scalar("PRAGMA main.quick_check")
      .fetch_one(&mut self.conn)
      .await?;
    if quick_check != "ok" {
      return Err(self.verify_failed(database, "quick_check", "ok".to_string(), quick_check));
    }
    let located = self
      .scalar("SELECT count(*) FROM attachments WHERE raw_offset IS NOT NULL")
      .await?;
    let inline = self
      .scalar("SELECT count(*) FROM attachments WHERE raw_offset IS NULL")
      .await?;
    let mismatches = self
      .scalar("SELECT attachment_mismatches FROM migration_origin WHERE id = 1")
      .await?;
    Ok(AttachmentCounts {
      located: u64::try_from(located).unwrap_or(0),
      inline: u64::try_from(inline).unwrap_or(0),
      mismatches: u64::try_from(mismatches).unwrap_or(0),
    })
  }

  fn verify_failed(
    &self,
    database: &Path,
    check: &'static str,
    expected: String,
    found: String,
  ) -> StorageError {
    StorageError::MigrationVerifyFailed {
      database: database.to_path_buf(),
      target: self.path.clone(),
      check,
      expected,
      found,
    }
  }

  /// [`Self::verify_failed`] for the source attached to this copy.
  fn verify_failed_at(&self, check: &'static str, expected: String, found: String) -> StorageError {
    StorageError::MigrationVerifyFailed {
      database: self.source.clone(),
      target: self.path.clone(),
      check,
      expected,
      found,
    }
  }

  /// Marks the copy complete and closes it: `user_version` is the last write.
  pub(crate) async fn finalize(mut self) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('optimize')")
      .execute(&mut self.conn)
      .await?;
    let completed_at = format_iso8601(OffsetDateTime::now_utc());
    let mut txn = self.conn.begin().await?;
    sqlx::query("UPDATE migration_origin SET completed_at = ?1 WHERE id = 1")
      .bind(&completed_at)
      .execute(&mut *txn)
      .await?;
    sqlx::query(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
      .execute(&mut *txn)
      .await?;
    txn.commit().await?;
    sqlx::query(&format!("DETACH DATABASE {SOURCE_SCHEMA}"))
      .execute(&mut self.conn)
      .await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
      .execute(&mut self.conn)
      .await?;
    self.close().await
  }
}

/// A run of legacy messages copied in one transaction.
struct Batch {
  last_seq: i64,
  messages: u64,
  raw_bytes: i64,
}

#[derive(sqlx::FromRow)]
struct LegacyAttachmentRow {
  seq: i64,
  id: String,
  filename: Option<String>,
  content_type: Option<String>,
  content_id: Option<String>,
  size: Option<i64>,
  content: Vec<u8>,
}

impl LegacyAttachmentRow {
  fn split(self) -> (i64, LegacyAttachment) {
    (
      self.seq,
      LegacyAttachment {
        meta: AttachmentMeta {
          id: self.id,
          filename: self.filename,
          content_type: self.content_type,
          content_id: self.content_id,
          size: self.size,
        },
        content: self.content,
      },
    )
  }
}

/// The columns an attachment keeps unchanged.
struct AttachmentMeta {
  id: String,
  filename: Option<String>,
  content_type: Option<String>,
  content_id: Option<String>,
  size: Option<i64>,
}

struct LegacyAttachment {
  meta: AttachmentMeta,
  content: Vec<u8>,
}

struct LegacyMessage {
  seq: i64,
  raw: Vec<u8>,
  attachments: Vec<LegacyAttachment>,
}

/// A legacy attachment as the copy stores it.
struct PlannedAttachment {
  seq: i64,
  meta: AttachmentMeta,
  storage: AttachmentStorage,
  /// Ingest would not have stored this attachment as it was stored: the
  /// part counts differ, or the located bytes do not decode to the legacy
  /// contents. The legacy contents are kept inline.
  mismatch: bool,
}

/// Runs `work` on every item across the blocking pool, one share per core,
/// and returns the results in input order.
async fn in_parallel<T, R>(items: Vec<T>, work: fn(T) -> R) -> Result<Vec<R>, StorageError>
where
  T: Send + 'static,
  R: Send + 'static,
{
  if items.is_empty() {
    return Ok(Vec::new());
  }
  let workers = std::thread::available_parallelism()
    .map_or(1, NonZeroUsize::get)
    .min(items.len());
  let mut shares: Vec<Vec<(usize, T)>> = (0..workers).map(|_| Vec::new()).collect();
  for (index, item) in items.into_iter().enumerate() {
    shares[index % workers].push((index, item));
  }
  let handles: Vec<_> = shares
    .into_iter()
    .map(|share| {
      tokio::task::spawn_blocking(move || {
        share
          .into_iter()
          .map(|(index, item)| (index, work(item)))
          .collect::<Vec<_>>()
      })
    })
    .collect();
  let mut results = Vec::new();
  for handle in handles {
    results.extend(handle.await.map_err(StorageError::MigrationTaskAborted)?);
  }
  results.sort_by_key(|(index, _)| *index);
  Ok(results.into_iter().map(|(_, result)| result).collect())
}

/// Pairs a message's legacy attachment rows with the parts ingest stores
/// today, and locates each row whose part the ingest eligibility accepts
/// and whose located bytes decode to exactly the legacy contents.
///
/// The legacy contents are the truth: any doubt keeps them inline.
fn plan_message(message: LegacyMessage) -> Vec<PlannedAttachment> {
  let raw = message.raw;
  let located: Vec<Option<Locator>> = MessageParser::default()
    .parse(&raw)
    .map(|parsed| {
      stored_part_refs(&parsed)
        .map(|part| locate(&raw, part))
        .collect()
    })
    .unwrap_or_default();
  let parts_match = located.len() == message.attachments.len();
  message
    .attachments
    .into_iter()
    .enumerate()
    .map(|(index, attachment)| {
      let candidate = located
        .get(index)
        .copied()
        .flatten()
        .filter(|_| parts_match);
      let verified = candidate.filter(|locator| reproduces(&raw, locator, &attachment));
      PlannedAttachment {
        seq: message.seq,
        meta: attachment.meta,
        mismatch: !parts_match || candidate.is_some() != verified.is_some(),
        storage: match verified {
          Some(locator) => AttachmentStorage::Located(locator),
          None => AttachmentStorage::Inline(attachment.content),
        },
      }
    })
    .collect()
}

/// Whether serving `locator` from `raw` yields the legacy attachment byte
/// for byte, at the size it records.
fn reproduces(raw: &[u8], locator: &Locator, attachment: &LegacyAttachment) -> bool {
  raw
    .get(locator.offset..locator.offset.saturating_add(locator.len))
    .and_then(|body| locator.encoding.decode(body))
    .is_some_and(|decoded| {
      decoded == attachment.content && attachment.meta.size == i64::try_from(decoded.len()).ok()
    })
}

/// Logs the copy's progress at most every [`PROGRESS_INTERVAL`].
struct Progress {
  started: Instant,
  last_report: Instant,
  migrated_at_start: u64,
}

impl Progress {
  fn new(migrated_at_start: u64) -> Self {
    let now = Instant::now();
    Self {
      started: now,
      last_report: now,
      migrated_at_start,
    }
  }

  fn report(&mut self, migration_id: &str, migrated: u64, total: u64) {
    if self.last_report.elapsed() < PROGRESS_INTERVAL {
      return;
    }
    self.last_report = Instant::now();
    let elapsed = self.started.elapsed().as_secs_f64();
    let rate_per_s = if elapsed > 0.0 {
      migrated.saturating_sub(self.migrated_at_start) as f64 / elapsed
    } else {
      0.0
    };
    let eta_s = if rate_per_s > 0.0 {
      total.saturating_sub(migrated) as f64 / rate_per_s
    } else {
      0.0
    };
    info!(
      event = "storage_migration",
      phase = "copy",
      migration_id,
      migrated,
      total,
      rate_per_s = rate_per_s.round(),
      eta_s = eta_s.round(),
      "Storage migration in progress"
    );
  }
}
