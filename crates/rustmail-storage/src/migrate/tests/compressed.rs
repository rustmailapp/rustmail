use super::*;
use crate::schema::{COMPRESSED_SCHEMA_VERSION, RELEASE_COLUMNS_DDL, SCHEMA_DDL};
use crate::{ReleaseOutcome, restore_backup};

const COMPRESSED_BACKUP: &str = ".schema2.bak";
const SEARCHED_SUBJECT: &str = "mail 3";

/// A message row and its stored source as a schema-2 or schema-3 file holds
/// them, columns both share.
type StoredMessage = (
  i64,
  String,
  String,
  String,
  Option<String>,
  i64,
  bool,
  bool,
  bool,
  String,
  String,
  Vec<u8>,
);
/// An attachment row as a schema-2 or schema-3 file holds it.
type StoredAttachment = (
  String,
  i64,
  Option<String>,
  Option<i64>,
  Option<i64>,
  Option<i64>,
  Option<Vec<u8>>,
);

/// Writes `count` messages into a new schema-2 database at `db`, laid out as
/// rustmail 0.10 stores them, with a mix of flags and tags, and leaves no
/// `-wal` beside it.
async fn compressed_database(db: &Path, count: usize) {
  let pool = SqlitePoolOptions::new()
    .max_connections(1)
    .connect_with(connect_options(&legacy::file_url(db)).unwrap())
    .await
    .unwrap();
  for statement in SCHEMA_DDL
    .iter()
    .filter(|ddl| !RELEASE_COLUMNS_DDL.contains(ddl))
  {
    sqlx::query(*statement).execute(&pool).await.unwrap();
  }
  for pragma in [
    format!("PRAGMA user_version = {COMPRESSED_SCHEMA_VERSION}"),
    "PRAGMA journal_mode = WAL".to_string(),
  ] {
    sqlx::query(AssertSqlSafe(pragma))
      .execute(&pool)
      .await
      .unwrap();
  }
  let repo = MessageRepository::new(pool);
  for index in 0..count {
    let summary = repo
      .insert(
        "sender@test.com",
        &["rcpt@test.com".to_string()],
        &sample_mail(index),
      )
      .await
      .unwrap();
    let tags: Vec<String> = match index % 3 {
      0 => Vec::new(),
      _ => vec!["work".to_string(), "urgent".to_string()],
    };
    repo
      .update_message(
        &summary.id,
        Some(index.is_multiple_of(2)),
        Some(index.is_multiple_of(5)),
        Some(&tags),
      )
      .await
      .unwrap();
  }
  repo.close().await;
  checkpoint_wal(db).await;
}

async fn checkpoint_wal(db: &Path) {
  let pool = legacy::open_plain(db).await;
  sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  assert_no_sidecars(db);
}

/// Every row both schemas hold, read without the repository.
async fn stored_rows(db: &Path) -> (Vec<StoredMessage>, Vec<StoredAttachment>, i64) {
  let pool = legacy::open_plain(db).await;
  let messages = sqlx::query_as(
    "SELECT m.seq, m.id, m.sender, m.recipients, m.subject, m.size, m.has_attachments, m.is_read, m.is_starred, m.tags, m.created_at, c.raw \
     FROM messages m JOIN message_content c ON c.seq = m.seq ORDER BY m.seq",
  )
  .fetch_all(&pool)
  .await
  .unwrap();
  let attachments = sqlx::query_as(
    "SELECT id, message_seq, content_type, raw_offset, raw_len, transfer_encoding, content FROM attachments ORDER BY rowid",
  )
  .fetch_all(&pool)
  .await
  .unwrap();
  let tag_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM message_tags")
    .fetch_one(&pool)
    .await
    .unwrap();
  pool.close().await;
  (messages, attachments, tag_rows)
}

async fn release_columns(db: &Path) -> Vec<String> {
  let pool = legacy::open_plain(db).await;
  let columns: Vec<String> = sqlx::query_scalar(
    "SELECT name FROM pragma_table_info('messages') WHERE name IN ('released_at', 'release_outcome') ORDER BY cid",
  )
  .fetch_all(&pool)
  .await
  .unwrap();
  pool.close().await;
  columns
}

#[tokio::test]
async fn a_schema_2_database_migrates_to_schema_3_with_every_row_kept_and_a_backup() {
  let (_dir, db) = scratch();
  compressed_database(&db, MESSAGES).await;
  let bytes_before = std::fs::read(&db).unwrap();
  let rows_before = stored_rows(&db).await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("a schema-2 database should be migrated");
  };

  let backup = sidecar(&db, COMPRESSED_BACKUP);
  assert_eq!(report.backup_path, backup);
  assert_eq!(report.messages, MESSAGES as u64);
  assert_eq!(
    std::fs::read(&backup).unwrap(),
    bytes_before,
    "the backup must be the schema-2 file as it was"
  );
  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(
    scalar(&backup, "PRAGMA user_version").await,
    COMPRESSED_SCHEMA_VERSION
  );
  assert_eq!(stored_rows(&db).await, rows_before);
  assert_eq!(
    release_columns(&db).await,
    ["released_at", "release_outcome"]
  );
  assert_eq!(
    scalar(
      &db,
      "SELECT count(*) FROM messages WHERE released_at IS NOT NULL OR release_outcome IS NOT NULL"
    )
    .await,
    0
  );
  assert_eq!(
    text(&db, "SELECT source_fingerprint FROM migration_origin")
      .await
      .map(|stored| stored.starts_with("schema3:")),
    Some(true)
  );

  let repo = open_migrated(&db).await;
  assert_eq!(repo.search_count(SEARCHED_SUBJECT).await.unwrap(), 1);
  let id = rows_before.0[0].1.clone();
  let release = repo
    .record_release(&id, ReleaseOutcome::Sent)
    .await
    .unwrap();
  assert_eq!(release.release_outcome, ReleaseOutcome::Sent);
  repo.close().await;
}

#[tokio::test]
async fn a_schema_2_snapshot_left_before_its_release_columns_is_rebuilt() {
  let (_dir, db) = scratch();
  compressed_database(&db, MESSAGES).await;
  let rows_before = stored_rows(&db).await;
  let pool = legacy::open_plain(&db).await;
  sqlx::query("VACUUM INTO ?1")
    .bind(
      sidecar(&db, MIGRATING_SUFFIX)
        .to_string_lossy()
        .into_owned(),
    )
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  checkpoint_wal(&db).await;

  assert!(matches!(
    migrate(&db).await.unwrap(),
    Preparation::Ready(Some(_))
  ));

  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(stored_rows(&db).await, rows_before);
  assert_eq!(
    release_columns(&db).await,
    ["released_at", "release_outcome"]
  );
}

#[tokio::test]
async fn a_finished_schema_3_copy_of_an_unchanged_schema_2_database_is_swapped_without_rebuilding()
{
  let (_dir, db) = scratch();
  compressed_database(&db, MESSAGES).await;
  let rows_before = stored_rows(&db).await;
  complete_copy_beside(&db).await;
  mark_copy(&db).await;

  assert!(matches!(
    migrate(&db).await.unwrap(),
    Preparation::Ready(Some(_))
  ));

  assert_eq!(
    text(&db, "SELECT completed_at FROM migration_origin")
      .await
      .as_deref(),
    Some(MARKER)
  );
  assert_eq!(stored_rows(&db).await, rows_before);
  assert!(sidecar(&db, COMPRESSED_BACKUP).exists());
}

#[tokio::test]
async fn a_schema_2_swap_interrupted_between_its_renames_is_finished() {
  let (_dir, db) = scratch();
  compressed_database(&db, MESSAGES).await;
  let rows_before = stored_rows(&db).await;
  complete_copy_beside(&db).await;
  std::fs::rename(&db, sidecar(&db, COMPRESSED_BACKUP)).unwrap();

  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));

  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(stored_rows(&db).await, rows_before);
}

#[tokio::test]
async fn a_schema_2_database_beside_its_backup_is_refused_as_ambiguous() {
  let (_dir, db) = scratch();
  compressed_database(&db, 2).await;
  std::fs::copy(&db, sidecar(&db, COMPRESSED_BACKUP)).unwrap();
  let bytes_before = std::fs::read(&db).unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::AmbiguousBackup { schema: 2, .. })
    ),
    "got {error:?}"
  );
  assert_eq!(std::fs::read(&db).unwrap(), bytes_before);
}

#[tokio::test]
async fn a_database_migrated_from_schema_2_restores_its_schema_2_backup() {
  let (_dir, db) = scratch();
  compressed_database(&db, MESSAGES).await;
  let bytes_before = std::fs::read(&db).unwrap();
  migrate(&db).await.unwrap();

  let report = restore_backup(&db).await.unwrap();

  assert_eq!(report.restored_from, sidecar(&db, COMPRESSED_BACKUP));
  assert_eq!(std::fs::read(&db).unwrap(), bytes_before);
  assert!(
    report.kept_as.to_string_lossy().contains(".schema3-"),
    "the migrated file should be kept as schema 3: {}",
    report.kept_as.display()
  );
  assert_eq!(
    scalar(&report.kept_as, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
}

/// Turns the schema-3 `copy`, migrated from the schema-1 `source`, into the
/// finished schema-2 copy rustmail 0.10 makes.
async fn as_copy_left_by_0_10(copy: &Path, source: &Path) {
  let bare = fingerprint_of(source, SourceSchema::Previous)
    .await
    .unwrap();
  let pool = legacy::open_plain(copy).await;
  for statement in [
    "ALTER TABLE messages DROP COLUMN release_outcome".to_string(),
    "ALTER TABLE messages DROP COLUMN released_at".to_string(),
    format!(
      "UPDATE migration_origin SET target_schema = {COMPRESSED_SCHEMA_VERSION}, source_fingerprint = 'schema{COMPRESSED_SCHEMA_VERSION}:{bare}'"
    ),
    format!("PRAGMA user_version = {COMPRESSED_SCHEMA_VERSION}"),
  ] {
    sqlx::query(AssertSqlSafe(statement))
      .execute(&pool)
      .await
      .unwrap();
  }
  pool.close().await;
  checkpoint_wal(copy).await;
}

#[tokio::test]
async fn a_swap_0_10_was_interrupted_in_is_finished_and_migrated_on_to_schema_3() {
  let (_dir, db) = scratch();
  let (expected_messages, expected_attachments) = previous_database(&db, MESSAGES).await;
  migrate(&db).await.unwrap();
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  std::fs::rename(&db, &copy).unwrap();
  as_copy_left_by_0_10(&copy, &sidecar(&db, PREVIOUS_BACKUP_SUFFIX)).await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("the finished swap should be migrated on");
  };

  assert_eq!(report.backup_path, sidecar(&db, COMPRESSED_BACKUP));
  assert!(sidecar(&db, PREVIOUS_BACKUP_SUFFIX).exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, expected_messages);
  assert_eq!(attachments, expected_attachments);
}
