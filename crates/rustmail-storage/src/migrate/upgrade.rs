//! Building the current-schema copy of a schema-2 database.
//!
//! Schema 3 only adds columns to schema 2, so the copy is the source file as
//! `VACUUM INTO` writes it, verified against the source and then altered.
//! The origin row, the new columns and `user_version` are written in one
//! transaction, so a copy is either a finished schema-3 file or one a later
//! start rebuilds.

use std::path::Path;

use sqlx::{AssertSqlSafe, Connection, SqliteConnection};
use time::OffsetDateTime;

use super::build::{AttachmentCounts, MIGRATION_ORIGIN_DDL, qualified_fingerprint, target_options};
use super::fingerprint::{SourceLayout, SourcePrint, SourceSchema, fingerprint};
use super::{MigrationOptions, existing_file};
use crate::StorageError;
use crate::repo::format_iso8601;
use crate::schema::{RELEASE_COLUMNS_DDL, SCHEMA_VERSION};

/// What `PRAGMA quick_check` answers for an intact file.
const QUICK_CHECK_OK: &str = "ok";

/// Writes the schema-2 database at `source` into the empty file at `copy`,
/// checks the copy holds what `print` describes, and turns it into schema 3.
///
/// # Errors
///
/// Returns [`StorageError::MigrationVerifyFailed`] if the copy does not match
/// the source, and [`StorageError::Database`] if SQLite fails, including a
/// full disk.
pub(crate) async fn build(
  copy: &Path,
  source: &Path,
  print: &SourcePrint,
  options: &MigrationOptions,
) -> Result<AttachmentCounts, StorageError> {
  vacuum_into(source, copy).await?;
  let mut conn = SqliteConnection::connect_with(&target_options(copy, options)).await?;
  let result = async {
    verify(&mut conn, copy, source, print).await?;
    finalize(&mut conn, print).await?;
    counts(&mut conn).await
  }
  .await;
  let closed = conn.close().await;
  let counts = result?;
  closed?;
  Ok(counts)
}

/// Writes a consistent snapshot of `source` into the empty file at `copy`.
async fn vacuum_into(source: &Path, copy: &Path) -> Result<(), StorageError> {
  let mut conn = SqliteConnection::connect_with(&existing_file(source)).await?;
  let result = sqlx::query("VACUUM INTO ?1")
    .bind(copy.to_string_lossy().into_owned())
    .execute(&mut conn)
    .await;
  conn.close().await?;
  result?;
  Ok(())
}

async fn verify(
  conn: &mut SqliteConnection,
  copy: &Path,
  source: &Path,
  print: &SourcePrint,
) -> Result<(), StorageError> {
  let failed =
    |check: &'static str, expected: String, found: String| StorageError::MigrationVerifyFailed {
      database: source.to_path_buf(),
      target: copy.to_path_buf(),
      check,
      expected,
      found,
    };
  let layout = SourceLayout::probe(conn, "main", SourceSchema::Compressed).await?;
  let copied = fingerprint(conn, &layout).await?;
  if copied.fingerprint != print.fingerprint {
    return Err(failed(
      "fingerprint",
      print.fingerprint.clone(),
      copied.fingerprint,
    ));
  }
  let quick_check: String = sqlx::query_scalar("PRAGMA main.quick_check")
    .fetch_one(&mut *conn)
    .await?;
  if quick_check != QUICK_CHECK_OK {
    return Err(failed(
      "quick_check",
      QUICK_CHECK_OK.to_string(),
      quick_check,
    ));
  }
  if let Err(error) =
    sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)")
      .execute(&mut *conn)
      .await
  {
    return Err(failed(
      "full-text integrity",
      QUICK_CHECK_OK.to_string(),
      error.to_string(),
    ));
  }
  Ok(())
}

/// Replaces the origin the source may carry from its own migration with this
/// one, adds the release columns and sets `user_version`, in one transaction.
async fn finalize(conn: &mut SqliteConnection, print: &SourcePrint) -> Result<(), StorageError> {
  let completed_at = format_iso8601(OffsetDateTime::now_utc());
  let mut txn = conn.begin_with("BEGIN IMMEDIATE").await?;
  sqlx::query("DROP TABLE IF EXISTS migration_origin")
    .execute(&mut *txn)
    .await?;
  sqlx::query(MIGRATION_ORIGIN_DDL).execute(&mut *txn).await?;
  sqlx::query(
    "INSERT INTO migration_origin (id, source_fingerprint, last_source_seq, completed_at, source_schema, target_schema) \
     VALUES (1, ?1, (SELECT coalesce(max(seq), 0) FROM messages), ?2, ?3, ?4)",
  )
  .bind(qualified_fingerprint(&print.fingerprint, SCHEMA_VERSION))
  .bind(&completed_at)
  .bind(SourceSchema::Compressed.version())
  .bind(SCHEMA_VERSION)
  .execute(&mut *txn)
  .await?;
  for statement in RELEASE_COLUMNS_DDL {
    sqlx::query(*statement).execute(&mut *txn).await?;
  }
  sqlx::query(AssertSqlSafe(format!(
    "PRAGMA user_version = {SCHEMA_VERSION}"
  )))
  .execute(&mut *txn)
  .await?;
  txn.commit().await?;
  sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
    .execute(&mut *conn)
    .await?;
  Ok(())
}

async fn counts(conn: &mut SqliteConnection) -> Result<AttachmentCounts, StorageError> {
  let (located, inline): (i64, i64) = sqlx::query_as(
    "SELECT (SELECT count(*) FROM attachments WHERE raw_offset IS NOT NULL), \
            (SELECT count(*) FROM attachments WHERE raw_offset IS NULL)",
  )
  .fetch_one(&mut *conn)
  .await?;
  Ok(AttachmentCounts {
    located: u64::try_from(located).unwrap_or(0),
    inline: u64::try_from(inline).unwrap_or(0),
    mismatches: 0,
  })
}
