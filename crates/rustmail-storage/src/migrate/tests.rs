use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

use super::*;
use crate::prepared::AttachmentStorage;
use crate::{MessageRepository, PreparedMessage, connect_options, initialize_database};

#[path = "../../tests/common/legacy_v0_7_0.rs"]
mod legacy;
mod restore;

const MESSAGES: usize = 12;
const SMALL_BATCH: i64 = 2;
const CANCEL_AFTER_BATCHES: usize = 3;
const TINY_TARGET_PAGES: u64 = 64;
const LARGE_BODY_BYTES: usize = 2 * 1024 * 1024;
const RESTRICTED_MODE: u32 = 0o640;
const MARKER: &str = "kept-from-the-first-run";
const BASE64_0_TO_31: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const DASHED_BASE64: &str = "QUJDREVG-R0hJ";
const QP_BINARY: &str = "=00=01=02abc";

struct TempDir(PathBuf);
impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn scratch() -> (TempDir, PathBuf) {
  let dir = std::env::temp_dir().join(format!("rustmail-migrate-{}", Ulid::new()));
  std::fs::create_dir_all(&dir).unwrap();
  let db = dir.join("rustmail.db");
  (TempDir(dir), db)
}

fn never() -> bool {
  false
}

/// `len` hex digits from a fixed-seed generator: text zstd cannot shrink
/// much, so a copy of it fills pages as the source does.
fn incompressible_text(len: usize) -> String {
  let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
  (0..len)
    .map(|_| {
      state ^= state << 13;
      state ^= state >> 7;
      state ^= state << 17;
      char::from_digit((state % 16) as u32, 16).unwrap_or('0')
    })
    .collect()
}

fn multipart(subject: &str, parts: &[(&str, &str)]) -> Vec<u8> {
  let mut raw = format!(
    "From: sender@test.com\r\nTo: rcpt@test.com\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
     Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
     --B\r\nContent-Type: text/plain\r\n\r\nBody of {subject}\r\n"
  );
  for (headers, body) in parts {
    raw.push_str(&format!("--B\r\n{headers}\r\n\r\n{body}\r\n"));
  }
  raw.push_str("--B--\r\n");
  raw.into_bytes()
}

fn sample_mail(index: usize) -> Vec<u8> {
  let subject = format!("mail {index}");
  match index % 4 {
    0 => {
      format!("From: a@test.com\r\nTo: b@test.com\r\nSubject: {subject}\r\n\r\nplain body {index}")
        .into_bytes()
    }
    1 => multipart(
      &subject,
      &[(
        "Content-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"a.png\"",
        BASE64_0_TO_31,
      )],
    ),
    2 => multipart(
      &subject,
      &[
        (
          "Content-Type: text/csv\r\nContent-Disposition: attachment; filename=\"a.csv\"",
          "a,b\r\n1,2",
        ),
        (
          "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: quoted-printable\r\nContent-Disposition: attachment; filename=\"a.bin\"",
          QP_BINARY,
        ),
      ],
    ),
    _ => multipart(
      &subject,
      &[(
        "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"dash.bin\"",
        DASHED_BASE64,
      )],
    ),
  }
}

/// Writes `count` messages into a new legacy database at `db`, with a mix
/// of flags and tags, and returns their summaries in arrival order.
async fn legacy_database(db: &Path, count: usize) -> Vec<legacy::LegacySummary> {
  let pool = legacy::create_legacy_database(db).await;
  let mut stored = Vec::new();
  for index in 0..count {
    stored.push(add_legacy_message(&pool, index).await);
  }
  pool.close().await;
  stored
}

async fn add_legacy_message(pool: &SqlitePool, index: usize) -> legacy::LegacySummary {
  let summary = legacy::insert_as_v0_7_0(
    pool,
    "sender@test.com",
    &["rcpt@test.com".to_string()],
    &sample_mail(index),
    &format!("2026-09-23T09:{:02}:{:02}Z", index / 60, index % 60),
  )
  .await
  .unwrap();
  let tags: Vec<String> = match index % 3 {
    0 => Vec::new(),
    1 => vec!["work".to_string()],
    _ => vec!["work".to_string(), "urgent".to_string(), "work".to_string()],
  };
  legacy::update_as_v0_7_0(
    pool,
    &summary.id,
    Some(index.is_multiple_of(2)),
    Some(index.is_multiple_of(5)),
    (!tags.is_empty()).then_some(tags.as_slice()),
  )
  .await
  .unwrap();
  summary
}

async fn add_to_legacy(db: &Path, index: usize) {
  let pool = legacy::open_plain(db).await;
  add_legacy_message(&pool, index).await;
  pool.close().await;
}

/// A message row as a legacy file or a migrated one holds it.
#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct MessageRow {
  seq: i64,
  id: String,
  sender: String,
  recipients: String,
  subject: Option<String>,
  text_body: Option<String>,
  html_body: Option<String>,
  raw: Vec<u8>,
  size: i64,
  has_attachments: bool,
  is_read: bool,
  is_starred: bool,
  tags: String,
  created_at: String,
}

/// An attachment as a legacy file or a migrated one serves it.
#[derive(Debug, PartialEq, Eq)]
struct AttachmentRow {
  message_id: String,
  id: String,
  filename: Option<String>,
  content_type: Option<String>,
  content_id: Option<String>,
  size: Option<i64>,
  content: Vec<u8>,
}

async fn legacy_snapshot(db: &Path) -> (Vec<MessageRow>, Vec<AttachmentRow>) {
  let pool = legacy::open_plain(db).await;
  let messages = sqlx::query_as::<_, MessageRow>(
    "SELECT rowid AS seq, id, sender, recipients, subject, text_body, html_body, raw, size, has_attachments, is_read, is_starred, tags, created_at FROM messages ORDER BY rowid",
  )
  .fetch_all(&pool)
  .await
  .unwrap();
  let attachments = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<i64>, Vec<u8>)>(
    "SELECT m.id, a.id, a.filename, a.content_type, a.content_id, a.size, a.content FROM messages m JOIN attachments a ON a.message_id = m.id ORDER BY m.rowid, a.rowid",
  )
  .fetch_all(&pool)
  .await
  .unwrap()
  .into_iter()
  .map(|(message_id, id, filename, content_type, content_id, size, content)| AttachmentRow {
    message_id,
    id,
    filename,
    content_type,
    content_id,
    size,
    content,
  })
  .collect();
  pool.close().await;
  (messages, attachments)
}

async fn open_migrated(db: &Path) -> MessageRepository {
  let pool = SqlitePoolOptions::new()
    .connect_with(connect_options(&legacy::file_url(db)).unwrap())
    .await
    .unwrap();
  initialize_database(&pool).await.unwrap();
  MessageRepository::new(pool)
}

/// The rows of a migrated database as the repository serves them: bodies
/// parsed and the source decompressed.
async fn migrated_snapshot(db: &Path) -> (Vec<MessageRow>, Vec<AttachmentRow>) {
  let repo = open_migrated(db).await;
  let pool = legacy::open_plain(db).await;
  let stored: Vec<(i64, String)> = sqlx::query_as("SELECT seq, id FROM messages ORDER BY seq")
    .fetch_all(&pool)
    .await
    .unwrap();
  pool.close().await;
  let mut messages = Vec::new();
  for (seq, id) in stored {
    let message = repo.get(&id).await.unwrap();
    messages.push(MessageRow {
      seq,
      raw: repo.get_raw(&id).await.unwrap(),
      id: message.id,
      sender: message.sender,
      recipients: message.recipients,
      subject: message.subject,
      text_body: message.text_body,
      html_body: message.html_body,
      size: message.size,
      has_attachments: message.has_attachments,
      is_read: message.is_read,
      is_starred: message.is_starred,
      tags: message.tags,
      created_at: message.created_at,
    });
  }
  let mut attachments = Vec::new();
  for message in &messages {
    for summary in repo.get_attachments(&message.id).await.unwrap() {
      let served = repo.get_attachment(&message.id, &summary.id).await.unwrap();
      attachments.push(AttachmentRow {
        message_id: served.message_id,
        id: served.id,
        filename: served.filename,
        content_type: served.content_type,
        content_id: served.content_id,
        size: served.size,
        content: served.content,
      });
    }
  }
  repo.close().await;
  (messages, attachments)
}

async fn scalar(db: &Path, sql: &str) -> i64 {
  let pool = legacy::open_plain(db).await;
  let value: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
  pool.close().await;
  value
}

async fn text(db: &Path, sql: &str) -> Option<String> {
  let pool = legacy::open_plain(db).await;
  let value: Option<String> = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
  pool.close().await;
  value
}

async fn migrate(db: &Path) -> Result<Preparation, StorageError> {
  prepare_with(db, &MigrationOptions::default(), &never).await
}

fn small_batches() -> MigrationOptions {
  MigrationOptions {
    batch_messages: SMALL_BATCH,
    ..MigrationOptions::default()
  }
}

/// Leaves a finished, verified copy beside a legacy `db`, by migrating while
/// another connection keeps the database open, which refuses the swap.
async fn complete_copy_beside(db: &Path) {
  let holder = legacy::open_plain(db).await;
  sqlx::query("SELECT count(*) FROM messages")
    .execute(&holder)
    .await
    .unwrap();
  let error = migrate(db).await.unwrap_err();
  assert!(
    matches!(
      &error,
      StorageError::MigrationRefused(MigrationRefusal::FileInUse { file, .. })
        if *file == sidecar(db, WAL_SUFFIX)
    ),
    "the swap should be refused while the database is open elsewhere, got {error:?}"
  );
  holder.close().await;
  assert_eq!(
    scalar(&sidecar(db, MIGRATING_SUFFIX), "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
}

async fn mark_copy(db: &Path) {
  let pool = legacy::open_plain(&sidecar(db, MIGRATING_SUFFIX)).await;
  sqlx::query("UPDATE migration_origin SET completed_at = ?1")
    .bind(MARKER)
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
}

/// Stores the bare fingerprint of `source` in the origin of `copy`, as
/// rustmail 0.10.0 stored it, and returns it.
async fn store_bare_fingerprint(copy: &Path, source: &Path) -> String {
  let bare = fingerprint_of(source, SourceSchema::Legacy).await.unwrap();
  let pool = legacy::open_plain(copy).await;
  sqlx::query("UPDATE migration_origin SET source_fingerprint = ?1")
    .bind(&bare)
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  bare
}

async fn stored_fingerprint(copy: &Path) -> Option<String> {
  text(copy, "SELECT source_fingerprint FROM migration_origin").await
}

fn assert_no_sidecars(db: &Path) {
  for suffix in [WAL_SUFFIX, SHM_SUFFIX, JOURNAL_SUFFIX] {
    assert!(
      !sidecar(db, suffix).exists(),
      "{suffix} was left beside {}",
      db.display()
    );
  }
}

#[tokio::test]
async fn a_legacy_database_migrates_with_every_row_preserved() {
  let (_dir, db) = scratch();
  legacy_database(&db, MESSAGES).await;
  let legacy_bytes = std::fs::read(&db).unwrap();
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("a legacy database should be migrated");
  };

  assert_eq!(report.messages, MESSAGES as u64);
  assert_eq!(report.attachment_mismatches, 0);
  assert_eq!(
    report.attachments_located + report.attachments_inline,
    legacy_attachments.len() as u64
  );
  assert!(report.attachments_located > 0 && report.attachments_inline > 0);
  assert_eq!(report.backup_path, sidecar(&db, LEGACY_BACKUP_SUFFIX));
  assert_eq!(std::fs::read(&report.backup_path).unwrap(), legacy_bytes);
  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert!(
    text(&db, "SELECT completed_at FROM migration_origin")
      .await
      .is_some()
  );

  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
}

#[tokio::test]
async fn rustmail_0_7_0_fails_on_a_migrated_file_before_writing() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  migrate(&db).await.unwrap();
  let migrated_bytes = std::fs::read(&db).unwrap();

  let pool = legacy::open_plain(&db).await;
  let error = legacy::initialize_as_v0_7_0(&pool)
    .await
    .expect_err("rustmail 0.7.0 must not open a migrated file");
  pool.close().await;

  assert!(
    error.to_string().contains("no such column: message_id"),
    "got {error}"
  );
  assert_eq!(std::fs::read(&db).unwrap(), migrated_bytes);
}

#[tokio::test]
async fn a_migrated_database_keeps_ordering_search_and_the_tag_index() {
  let (_dir, db) = scratch();
  let stored = legacy_database(&db, MESSAGES).await;
  migrate(&db).await.unwrap();

  let repo = open_migrated(&db).await;
  let newest_first: Vec<String> = stored
    .iter()
    .rev()
    .map(|summary| summary.id.clone())
    .collect();
  let listed: Vec<String> = repo
    .list(i64::MAX, 0)
    .await
    .unwrap()
    .into_iter()
    .map(|summary| summary.id)
    .collect();
  assert_eq!(listed, newest_first);

  let cursor = repo.cursor(&stored[MESSAGES / 2].id).await.unwrap();
  let older: Vec<String> = repo
    .list_page(
      &crate::MessageFilter::default(),
      crate::PageStart::Before(cursor),
      i64::MAX,
    )
    .await
    .unwrap()
    .into_iter()
    .map(|summary| summary.id)
    .collect();
  assert_eq!(older, newest_first[MESSAGES - MESSAGES / 2..]);

  let found = repo.search("mail 5", i64::MAX, 0).await.unwrap();
  assert!(found.iter().any(|summary| summary.id == stored[5].id));
  repo.close().await;

  assert_eq!(
    scalar(&db, "SELECT count(*) FROM message_tags").await,
    scalar(
      &db,
      "SELECT count(*) FROM (SELECT DISTINCT m.seq, t.value FROM messages m, json_each(m.tags) t)"
    )
    .await
  );
  assert!(scalar(&db, "SELECT count(*) FROM message_tags").await > 0);
}

#[tokio::test]
async fn a_legacy_attachment_that_does_not_match_its_raw_part_stays_inline_with_its_legacy_bytes() {
  let (_dir, db) = scratch();
  let stored = legacy_database(&db, 2).await;
  let pool = legacy::open_plain(&db).await;
  sqlx::query("UPDATE attachments SET content = x'2a', size = 1 WHERE id = ?1")
    .bind(&stored[1].attachment_ids[0])
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("a legacy database should be migrated");
  };

  assert_eq!(report.attachment_mismatches, 1);
  assert_eq!(report.attachments_located, 0);
  let repo = open_migrated(&db).await;
  let served = repo
    .get_attachment(&stored[1].id, &stored[1].attachment_ids[0])
    .await
    .unwrap();
  assert_eq!(served.content, b"*");
  repo.close().await;
}

#[tokio::test]
async fn a_second_process_gets_migration_locked_naming_the_lock_file() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  let legacy_bytes = std::fs::read(&db).unwrap();
  let paths = Paths::new(&db);
  let held = acquire_lock(&paths).await.unwrap();

  let error = migrate(&db).await.unwrap_err();

  let StorageError::MigrationLocked { ref lock_path } = error else {
    panic!("expected MigrationLocked, got {error:?}");
  };
  assert_eq!(*lock_path, sidecar(&db, LOCK_SUFFIX));
  assert!(error.to_string().contains(&lock_path.display().to_string()));
  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  drop(held);
  assert!(matches!(
    migrate(&db).await.unwrap(),
    Preparation::Ready(Some(_))
  ));
}

#[tokio::test]
async fn a_fresh_database_needs_nothing_even_under_the_lock() {
  let (_dir, db) = scratch();
  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));
  assert!(!db.exists());
  assert!(sidecar(&db, LOCK_SUFFIX).exists());

  std::fs::File::create(&db).unwrap();
  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));
}

#[tokio::test]
async fn a_migration_cancelled_at_batch_k_resumes_where_it_stopped() {
  let (_dir, db) = scratch();
  legacy_database(&db, MESSAGES).await;
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;
  let asked = AtomicUsize::new(0);
  let stop_after_k = || asked.fetch_add(1, Ordering::SeqCst) >= CANCEL_AFTER_BATCHES;

  let paused = prepare_with(&db, &small_batches(), &stop_after_k)
    .await
    .unwrap();

  let copied = CANCEL_AFTER_BATCHES as u64 * SMALL_BATCH as u64;
  assert_eq!(
    paused,
    Preparation::Paused {
      migrated: copied,
      total: MESSAGES as u64
    }
  );
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  assert_eq!(scalar(&copy, "PRAGMA user_version").await, 0);
  assert_eq!(
    scalar(&copy, "SELECT count(*) FROM messages").await,
    copied as i64
  );
  assert_eq!(
    scalar(&copy, "SELECT last_source_seq FROM migration_origin").await,
    copied as i64
  );
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
  assert_no_sidecars(&db);

  let Preparation::Ready(Some(report)) = prepare_with(&db, &small_batches(), &never).await.unwrap()
  else {
    panic!("the resumed migration should complete");
  };
  assert_eq!(report.messages, MESSAGES as u64);
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
}

#[tokio::test]
async fn a_copy_stores_a_fingerprint_rustmail_0_9_does_not_match() {
  let (_dir, db) = scratch();
  legacy_database(&db, MESSAGES).await;
  let bare = fingerprint_of(&db, SourceSchema::Legacy).await.unwrap();
  let asked = AtomicUsize::new(0);
  let stop_after_one = || asked.fetch_add(1, Ordering::SeqCst) >= 1;

  prepare_with(&db, &small_batches(), &stop_after_one)
    .await
    .unwrap();

  let stored = stored_fingerprint(&sidecar(&db, MIGRATING_SUFFIX)).await;
  assert_eq!(stored, Some(format!("schema2:{bare}")));
}

#[tokio::test]
async fn a_copy_paused_by_0_10_0_is_qualified_and_resumed_where_it_stopped() {
  let (_dir, db) = scratch();
  legacy_database(&db, MESSAGES).await;
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;
  let asked = AtomicUsize::new(0);
  let stop_after_k = || asked.fetch_add(1, Ordering::SeqCst) >= CANCEL_AFTER_BATCHES;
  prepare_with(&db, &small_batches(), &stop_after_k)
    .await
    .unwrap();
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  let bare = store_bare_fingerprint(&copy, &db).await;

  let paused = prepare_with(&db, &small_batches(), &|| true).await.unwrap();

  assert_eq!(
    paused,
    Preparation::Paused {
      migrated: CANCEL_AFTER_BATCHES as u64 * SMALL_BATCH as u64,
      total: MESSAGES as u64
    }
  );
  assert_eq!(
    stored_fingerprint(&copy).await,
    Some(format!("schema2:{bare}"))
  );
  let Preparation::Ready(Some(_)) = prepare_with(&db, &small_batches(), &never).await.unwrap()
  else {
    panic!("the resumed migration should complete");
  };
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
}

#[tokio::test]
async fn an_incomplete_copy_of_a_changed_database_is_rebuilt() {
  let (_dir, db) = scratch();
  legacy_database(&db, MESSAGES).await;
  let asked = AtomicUsize::new(0);
  let stop_after_one = || asked.fetch_add(1, Ordering::SeqCst) >= 1;
  prepare_with(&db, &small_batches(), &stop_after_one)
    .await
    .unwrap();
  add_to_legacy(&db, MESSAGES).await;
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;

  let Preparation::Ready(Some(report)) = prepare_with(&db, &small_batches(), &never).await.unwrap()
  else {
    panic!("the rebuilt migration should complete");
  };

  assert_eq!(report.messages, MESSAGES as u64 + 1);
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
}

#[tokio::test]
async fn an_empty_copy_without_an_origin_is_rebuilt() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  std::fs::File::create(sidecar(&db, MIGRATING_SUFFIX)).unwrap();

  assert!(matches!(
    migrate(&db).await.unwrap(),
    Preparation::Ready(Some(_))
  ));
  assert_eq!(scalar(&db, "SELECT count(*) FROM messages").await, 2);
}

#[tokio::test]
async fn the_swap_is_refused_while_the_database_is_open_elsewhere() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  let legacy_bytes = std::fs::read(&db).unwrap();

  complete_copy_beside(&db).await;

  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert!(!sidecar(&db, LEGACY_BACKUP_SUFFIX).exists());
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
}

#[tokio::test]
async fn the_swap_is_refused_while_the_copy_is_open_elsewhere() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  complete_copy_beside(&db).await;
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  let holder = legacy::open_plain(&copy).await;
  sqlx::query("SELECT count(*) FROM messages")
    .execute(&holder)
    .await
    .unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::MigrationRefused(MigrationRefusal::FileInUse { file, .. })
        if *file == sidecar(&copy, WAL_SUFFIX)
    ),
    "expected the copy's -wal to be named, got {error:?}"
  );
  holder.close().await;
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
}

#[tokio::test]
async fn a_complete_copy_of_an_unchanged_database_is_swapped_without_rebuilding() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  let legacy_bytes = std::fs::read(&db).unwrap();
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
  assert_eq!(
    std::fs::read(sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap(),
    legacy_bytes
  );
}

#[tokio::test]
async fn a_complete_copy_of_a_database_changed_after_completion_is_rebuilt() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  complete_copy_beside(&db).await;
  mark_copy(&db).await;
  add_to_legacy(&db, 3).await;
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("the rebuilt migration should complete");
  };

  assert_eq!(report.messages, 4);
  assert_ne!(
    text(&db, "SELECT completed_at FROM migration_origin")
      .await
      .as_deref(),
    Some(MARKER)
  );
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
}

#[tokio::test]
async fn a_swap_interrupted_between_its_renames_is_finished() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  let (legacy_messages, _) = legacy_snapshot(&db).await;
  complete_copy_beside(&db).await;
  std::fs::rename(&db, sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap();

  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));

  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  let (messages, _) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
}

#[tokio::test]
async fn a_complete_copy_left_by_0_10_0_is_swapped_without_rebuilding() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  complete_copy_beside(&db).await;
  mark_copy(&db).await;
  let bare = store_bare_fingerprint(&sidecar(&db, MIGRATING_SUFFIX), &db).await;

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
  assert_eq!(
    stored_fingerprint(&db).await,
    Some(format!("schema2:{bare}"))
  );
}

#[tokio::test]
async fn a_swap_0_10_0_was_interrupted_in_is_finished() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  let (legacy_messages, _) = legacy_snapshot(&db).await;
  complete_copy_beside(&db).await;
  store_bare_fingerprint(&sidecar(&db, MIGRATING_SUFFIX), &db).await;
  std::fs::rename(&db, sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap();

  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));

  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
  let (messages, _) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
}

#[tokio::test]
async fn an_interrupted_swap_whose_backup_changed_is_refused() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  complete_copy_beside(&db).await;
  let backup = sidecar(&db, LEGACY_BACKUP_SUFFIX);
  std::fs::rename(&db, &backup).unwrap();
  add_to_legacy(&backup, 3).await;

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::BackupChanged { .. })
    ),
    "got {error:?}"
  );
  assert!(!db.exists());
  assert!(sidecar(&db, MIGRATING_SUFFIX).exists());
}

#[tokio::test]
async fn a_legacy_database_beside_an_existing_backup_is_refused_as_ambiguous() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  std::fs::copy(&db, sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap();
  let legacy_bytes = std::fs::read(&db).unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::AmbiguousBackup { .. })
    ),
    "got {error:?}"
  );
  assert!(error.to_string().contains(".schema0.bak"));
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert!(!sidecar(&db, MIGRATING_SUFFIX).exists());
}

#[tokio::test]
async fn a_stray_copy_beside_a_current_database_is_refused() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  migrate(&db).await.unwrap();
  std::fs::File::create(sidecar(&db, MIGRATING_SUFFIX)).unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::StrayMigrationCopy { .. })
    ),
    "got {error:?}"
  );
}

#[tokio::test]
async fn a_current_database_with_its_backup_opens() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  migrate(&db).await.unwrap();

  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));
}

#[tokio::test]
async fn a_backup_without_a_database_or_a_copy_is_refused() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  std::fs::rename(&db, sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::UnknownState { .. })
    ),
    "got {error:?}"
  );
  assert!(!db.exists());
}

#[tokio::test]
async fn running_out_of_space_leaves_the_legacy_database_byte_identical() {
  let (_dir, db) = scratch();
  let pool = legacy::create_legacy_database(&db).await;
  let large = format!(
    "From: a@test.com\r\nTo: b@test.com\r\nSubject: large\r\n\r\n{}",
    incompressible_text(LARGE_BODY_BYTES)
  );
  for _ in 0..2 {
    legacy::insert_as_v0_7_0(
      &pool,
      "a@test.com",
      &[],
      large.as_bytes(),
      "2026-09-23T09:00:00Z",
    )
    .await
    .unwrap();
  }
  pool.close().await;
  let legacy_bytes = std::fs::read(&db).unwrap();
  let capped = MigrationOptions {
    target_max_pages: Some(TINY_TARGET_PAGES),
    ..MigrationOptions::default()
  };

  let error = prepare_with(&db, &capped, &never).await.unwrap_err();

  let StorageError::MigrationDiskFull { needed_bytes, .. } = error else {
    panic!("expected MigrationDiskFull, got {error:?}");
  };
  assert!(needed_bytes > 0);
  assert!(
    error
      .to_string()
      .contains("free space and restart to resume")
  );
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert_no_sidecars(&db);
  assert!(!sidecar(&db, LEGACY_BACKUP_SUFFIX).exists());

  assert!(matches!(
    migrate(&db).await.unwrap(),
    Preparation::Ready(Some(_))
  ));
  assert_eq!(scalar(&db, "SELECT count(*) FROM messages").await, 2);
}

#[cfg(unix)]
#[tokio::test]
async fn the_database_file_mode_is_preserved() {
  use std::os::unix::fs::PermissionsExt;
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  std::fs::set_permissions(&db, std::fs::Permissions::from_mode(RESTRICTED_MODE)).unwrap();

  migrate(&db).await.unwrap();

  for file in [
    db.clone(),
    sidecar(&db, LEGACY_BACKUP_SUFFIX),
    sidecar(&db, LOCK_SUFFIX),
  ] {
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & MODE_BITS;
    assert_eq!(
      mode,
      RESTRICTED_MODE,
      "{} lost the database's mode",
      file.display()
    );
  }
}

const PREVIOUS_SCHEMA: i64 = 1;
/// `migration_origin` as rustmail 0.8 and 0.9 create it in their copy.
const PREVIOUS_ORIGIN_DDL: &str = "CREATE TABLE migration_origin (id INTEGER PRIMARY KEY CHECK (id = 1), source_fingerprint TEXT NOT NULL, last_source_seq INTEGER NOT NULL, attachment_mismatches INTEGER NOT NULL DEFAULT 0, completed_at TEXT)";

/// Writes `count` messages into a new schema-1 database at `db`, laid out as
/// rustmail 0.8 and 0.9 store them, with a mix of flags and tags, and
/// returns the rows and attachments a migration must preserve.
async fn previous_database(db: &Path, count: usize) -> (Vec<MessageRow>, Vec<AttachmentRow>) {
  let pool = legacy::open_plain(db).await;
  for statement in crate::schema::SCHEMA_1_DDL {
    sqlx::query(statement).execute(&pool).await.unwrap();
  }
  sqlx::query(&format!("PRAGMA user_version = {PREVIOUS_SCHEMA}"))
    .execute(&pool)
    .await
    .unwrap();
  let mut messages = Vec::new();
  let mut attachments = Vec::new();
  for index in 0..count {
    let raw = sample_mail(index);
    let prepared = PreparedMessage::parse(
      "sender@test.com".to_string(),
      &["rcpt@test.com".to_string()],
      raw.clone(),
    )
    .unwrap();
    let bodies = crate::content::Bodies::parse(&raw);
    let tags = match index % 3 {
      0 => "[]",
      1 => r#"["work"]"#,
      _ => r#"["work","urgent","work"]"#,
    };
    let row = MessageRow {
      seq: index as i64 + 1,
      id: Ulid::new().to_string(),
      sender: prepared.sender.clone(),
      recipients: prepared.recipients_json.clone(),
      subject: prepared.subject.clone(),
      text_body: bodies.text,
      html_body: bodies.html,
      raw: raw.clone(),
      size: raw.len() as i64,
      has_attachments: prepared.has_attachments,
      is_read: index.is_multiple_of(2),
      is_starred: index.is_multiple_of(5),
      tags: tags.to_string(),
      created_at: format!("2026-10-01T09:{:02}:{:02}Z", index / 60, index % 60),
    };
    sqlx::query(
      "INSERT INTO messages (seq, id, sender, recipients, subject, size, has_attachments, is_read, is_starred, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(row.seq)
    .bind(&row.id)
    .bind(&row.sender)
    .bind(&row.recipients)
    .bind(&row.subject)
    .bind(row.size)
    .bind(row.has_attachments)
    .bind(row.is_read)
    .bind(row.is_starred)
    .bind(&row.tags)
    .bind(&row.created_at)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
      "INSERT INTO message_content (seq, text_body, html_body, raw) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(row.seq)
    .bind(&row.text_body)
    .bind(&row.html_body)
    .bind(&row.raw)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO messages_fts (rowid, subject, text_body, sender, recipients) VALUES (?1, ?2, ?3, ?4, ?5)")
      .bind(row.seq)
      .bind(&row.subject)
      .bind(&row.text_body)
      .bind(&row.sender)
      .bind(&row.recipients)
      .execute(&pool)
      .await
      .unwrap();
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    let decoded: Vec<Vec<u8>> = crate::prepared::stored_part_refs(&parsed)
      .map(|part| crate::prepared::part_contents(part).to_vec())
      .collect();
    for (attachment, content) in prepared.attachments.iter().zip(decoded) {
      let id = Ulid::new().to_string();
      let (locator, inline) = match &attachment.storage {
        AttachmentStorage::Located(locator) => (Some(*locator), None),
        AttachmentStorage::Inline(content) => (None, Some(content.clone())),
      };
      sqlx::query(
        "INSERT INTO attachments (id, message_seq, filename, content_type, content_id, size, raw_offset, raw_len, transfer_encoding, content) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
      )
      .bind(&id)
      .bind(row.seq)
      .bind(&attachment.filename)
      .bind(&attachment.content_type)
      .bind(&attachment.content_id)
      .bind(attachment.size as i64)
      .bind(locator.map(|locator| locator.offset as i64))
      .bind(locator.map(|locator| locator.len as i64))
      .bind(locator.map(|locator| locator.encoding.code()))
      .bind(inline)
      .execute(&pool)
      .await
      .unwrap();
      attachments.push(AttachmentRow {
        message_id: row.id.clone(),
        id,
        filename: attachment.filename.clone(),
        content_type: attachment.content_type.clone(),
        content_id: attachment.content_id.clone(),
        size: Some(attachment.size as i64),
        content,
      });
    }
    messages.push(row);
  }
  pool.close().await;
  (messages, attachments)
}

#[tokio::test]
async fn a_schema_1_database_migrates_with_every_row_preserved() {
  let (_dir, db) = scratch();
  let (expected_messages, expected_attachments) = previous_database(&db, MESSAGES).await;
  let previous_bytes = std::fs::read(&db).unwrap();

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("a schema-1 database should be migrated");
  };

  assert_eq!(report.messages, MESSAGES as u64);
  assert_eq!(report.attachment_mismatches, 0);
  assert_eq!(
    report.attachments_located + report.attachments_inline,
    expected_attachments.len() as u64
  );
  assert_eq!(report.backup_path, sidecar(&db, PREVIOUS_BACKUP_SUFFIX));
  assert_eq!(std::fs::read(&report.backup_path).unwrap(), previous_bytes);
  assert!(!sidecar(&db, LEGACY_BACKUP_SUFFIX).exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(
    scalar(&db, "SELECT source_schema FROM migration_origin").await,
    PREVIOUS_SCHEMA
  );
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, expected_messages);
  assert_eq!(attachments, expected_attachments);

  let repo = open_migrated(&db).await;
  let found = repo.search("mail 5", i64::MAX, 0).await.unwrap();
  assert!(
    found
      .iter()
      .any(|summary| summary.id == expected_messages[5].id)
  );
  repo.close().await;
  assert_eq!(
    scalar(&db, "SELECT count(*) FROM message_tags").await,
    scalar(
      &sidecar(&db, PREVIOUS_BACKUP_SUFFIX),
      "SELECT count(*) FROM message_tags"
    )
    .await
  );
}

#[tokio::test]
async fn a_schema_1_database_beside_its_backup_is_refused_as_ambiguous() {
  let (_dir, db) = scratch();
  previous_database(&db, 2).await;
  std::fs::copy(&db, sidecar(&db, PREVIOUS_BACKUP_SUFFIX)).unwrap();
  let previous_bytes = std::fs::read(&db).unwrap();

  let error = migrate(&db).await.unwrap_err();

  assert!(
    matches!(
      error,
      StorageError::MigrationRefused(MigrationRefusal::AmbiguousBackup { schema: 1, .. })
    ),
    "got {error:?}"
  );
  assert!(error.to_string().contains(".schema1.bak"));
  assert_eq!(std::fs::read(&db).unwrap(), previous_bytes);
}

#[tokio::test]
async fn a_copy_left_complete_by_0_9_is_swapped_in_and_then_migrated_on() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  let mut source = Source::lock(&db).await.unwrap();
  let layout = SourceLayout::probe(&mut source.conn, "main", SourceSchema::Legacy)
    .await
    .unwrap();
  let print = source.fingerprint(&layout).await.unwrap();
  source.release().await.unwrap();
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  let (expected_messages, _) = previous_database(&copy, 3).await;
  let pool = legacy::open_plain(&copy).await;
  sqlx::query(PREVIOUS_ORIGIN_DDL)
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query("INSERT INTO migration_origin (id, source_fingerprint, last_source_seq, completed_at) VALUES (1, ?1, 3, ?2)")
    .bind(&print.fingerprint)
    .bind(MARKER)
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  let legacy_bytes = std::fs::read(&db).unwrap();
  std::fs::rename(&db, sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap();
  let copy_bytes = std::fs::read(&copy).unwrap();

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("the finished schema-1 copy should be swapped in and migrated");
  };

  assert_eq!(report.backup_path, sidecar(&db, PREVIOUS_BACKUP_SUFFIX));
  assert_eq!(std::fs::read(&report.backup_path).unwrap(), copy_bytes);
  assert_eq!(
    std::fs::read(sidecar(&db, LEGACY_BACKUP_SUFFIX)).unwrap(),
    legacy_bytes
  );
  assert!(!copy.exists());
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  let (messages, _) = migrated_snapshot(&db).await;
  assert_eq!(messages, expected_messages);
}

#[tokio::test]
async fn an_incomplete_copy_left_by_0_9_is_rebuilt() {
  let (_dir, db) = scratch();
  legacy_database(&db, 3).await;
  let (legacy_messages, _) = legacy_snapshot(&db).await;
  let mut source = Source::lock(&db).await.unwrap();
  let layout = SourceLayout::probe(&mut source.conn, "main", SourceSchema::Legacy)
    .await
    .unwrap();
  let print = source.fingerprint(&layout).await.unwrap();
  source.release().await.unwrap();
  let copy = sidecar(&db, MIGRATING_SUFFIX);
  let pool = legacy::open_plain(&copy).await;
  for statement in crate::schema::SCHEMA_1_DDL {
    sqlx::query(statement).execute(&pool).await.unwrap();
  }
  sqlx::query(PREVIOUS_ORIGIN_DDL)
    .execute(&pool)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO migration_origin (id, source_fingerprint, last_source_seq) VALUES (1, ?1, 1)",
  )
  .bind(&print.fingerprint)
  .execute(&pool)
  .await
  .unwrap();
  pool.close().await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("the migration should complete");
  };

  assert_eq!(report.messages, 3);
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  let (messages, _) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
}
