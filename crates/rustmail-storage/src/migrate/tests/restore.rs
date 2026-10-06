use super::*;
use crate::{RestoreRefusal, RestoreReport, restore_backup};

const POST_MIGRATION_SUBJECT: &str = "after the migration";

/// Migrates a legacy database of `count` messages, then stores one more
/// message in the migrated file, and returns that message's id.
async fn migrated_with_new_mail(db: &Path, count: usize) -> String {
  legacy_database(db, count).await;
  migrate(db).await.unwrap();
  let repo = open_migrated(db).await;
  let raw = format!(
    "From: late@test.com\r\nTo: rcpt@test.com\r\nSubject: {POST_MIGRATION_SUBJECT}\r\n\r\nnew"
  );
  let summary = repo
    .insert(
      "late@test.com",
      &["rcpt@test.com".to_string()],
      raw.as_bytes(),
    )
    .await
    .unwrap();
  repo.close().await;
  summary.id
}

fn kept_files(db: &Path) -> Vec<PathBuf> {
  let prefix = format!("{}.schema1-", db.file_name().unwrap().to_string_lossy());
  let mut kept: Vec<PathBuf> = std::fs::read_dir(db.parent().unwrap())
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| {
      path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
    })
    .collect();
  kept.sort();
  kept
}

/// The schema version and message ids of the database at `db`, which a
/// refused restore must leave as they were.
async fn contents(db: &Path) -> (i64, Vec<String>) {
  let pool = legacy::open_plain(db).await;
  let version: i64 = sqlx::query_scalar("PRAGMA user_version")
    .fetch_one(&pool)
    .await
    .unwrap();
  let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
    .fetch_all(&pool)
    .await
    .unwrap();
  pool.close().await;
  (version, ids)
}

async fn assert_untouched(db: &Path, db_contents: &(i64, Vec<String>), backup_bytes: &[u8]) {
  assert_eq!(&contents(db).await, db_contents);
  assert_eq!(
    std::fs::read(sidecar(db, BACKUP_SUFFIX)).unwrap(),
    backup_bytes
  );
  assert!(kept_files(db).is_empty());
}

#[tokio::test]
async fn a_restored_backup_opens_under_v0_7_0_with_the_pre_migration_mailbox() {
  let (_dir, db) = scratch();
  let late_id = migrated_with_new_mail(&db, MESSAGES).await;
  let backup = sidecar(&db, BACKUP_SUFFIX);
  let legacy_bytes = std::fs::read(&backup).unwrap();
  let before = legacy_snapshot(&backup).await;

  let report = restore_backup(&db).await.unwrap();

  assert_eq!(
    report,
    RestoreReport {
      database: db.clone(),
      restored_from: sidecar(&db, BACKUP_SUFFIX),
      kept_as: kept_files(&db).pop().unwrap(),
    }
  );
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert!(!sidecar(&db, BACKUP_SUFFIX).exists());
  let pool = legacy::open_plain(&db).await;
  legacy::initialize_as_v0_7_0(&pool).await.unwrap();
  pool.close().await;
  assert_eq!(legacy_snapshot(&db).await, before);
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
  assert_no_sidecars(&db);

  let kept = &report.kept_as;
  assert_eq!(
    scalar(kept, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(
    scalar(kept, "SELECT count(*) FROM messages").await,
    MESSAGES as i64 + 1
  );
  assert_eq!(
    text(
      kept,
      &format!("SELECT subject FROM messages WHERE id = '{}'", late_id)
    )
    .await,
    Some(POST_MIGRATION_SUBJECT.to_string())
  );
  assert_no_sidecars(kept);
}

#[tokio::test]
async fn a_restored_backup_migrates_again_beside_the_kept_schema_1_file() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, MESSAGES).await;
  let before_restore = restore_backup(&db).await.unwrap();
  let kept_bytes = std::fs::read(&before_restore.kept_as).unwrap();
  let (legacy_messages, legacy_attachments) = legacy_snapshot(&db).await;

  let Preparation::Ready(Some(report)) = migrate(&db).await.unwrap() else {
    panic!("the restored database should be migrated again");
  };

  assert_eq!(report.messages, MESSAGES as u64);
  assert_eq!(report.attachment_mismatches, 0);
  let (messages, attachments) = migrated_snapshot(&db).await;
  assert_eq!(messages, legacy_messages);
  assert_eq!(attachments, legacy_attachments);
  assert_eq!(std::fs::read(&before_restore.kept_as).unwrap(), kept_bytes);
  assert_eq!(kept_files(&db), vec![before_restore.kept_as]);
  assert_eq!(migrate(&db).await.unwrap(), Preparation::Ready(None));
}

#[tokio::test]
async fn a_restore_is_refused_while_another_connection_has_the_database_open() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let holder = legacy::open_plain(&db).await;
  sqlx::query("SELECT count(*) FROM messages")
    .execute(&holder)
    .await
    .unwrap();
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::FileInUse { file, .. })
        if *file == sidecar(&db, WAL_SUFFIX)
    ),
    "expected the database's -wal to be named, got {error:?}"
  );
  assert!(error.to_string().contains("run restore-backup again"));
  holder.close().await;
  assert_eq!(
    scalar(&db, "PRAGMA user_version").await,
    crate::SCHEMA_VERSION
  );
  assert_eq!(
    std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap(),
    backup_bytes
  );
  assert!(kept_files(&db).is_empty());
}

#[tokio::test]
async fn a_restore_is_refused_while_the_migration_lock_is_held() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let db_contents = contents(&db).await;
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let held = acquire_lock(&Paths::new(&db)).await.unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(&error, StorageError::MigrationLocked { lock_path } if *lock_path == sidecar(&db, LOCK_SUFFIX)),
    "got {error:?}"
  );
  drop(held);
  assert_untouched(&db, &db_contents, &backup_bytes).await;
}

#[tokio::test]
async fn a_restore_is_refused_without_a_backup() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  std::fs::remove_file(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let db_contents = contents(&db).await;

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::NoBackup { backup, .. })
        if *backup == sidecar(&db, BACKUP_SUFFIX)
    ),
    "got {error:?}"
  );
  assert_eq!(contents(&db).await, db_contents);
  assert!(kept_files(&db).is_empty());
}

#[tokio::test]
async fn a_restore_is_refused_when_the_backup_is_not_a_schema_0_database() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let db_contents = contents(&db).await;
  std::fs::copy(&db, sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::BackupInvalid { backup, .. })
        if *backup == sidecar(&db, BACKUP_SUFFIX)
    ),
    "got {error:?}"
  );
  assert_untouched(&db, &db_contents, &backup_bytes).await;
}

#[tokio::test]
async fn a_restore_is_refused_when_the_backup_is_not_sqlite() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let db_contents = contents(&db).await;
  let garbage = vec![b'x'; 8192];
  std::fs::write(sidecar(&db, BACKUP_SUFFIX), &garbage).unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::BackupInvalid { .. })
    ),
    "got {error:?}"
  );
  assert_untouched(&db, &db_contents, &garbage).await;
}

#[tokio::test]
async fn a_restore_is_refused_while_the_backup_has_a_journal_beside_it() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let db_contents = contents(&db).await;
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let journal = sidecar(&sidecar(&db, BACKUP_SUFFIX), JOURNAL_SUFFIX);
  std::fs::write(&journal, b"hot").unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::FileInUse { file, .. }) if *file == journal
    ),
    "got {error:?}"
  );
  assert_untouched(&db, &db_contents, &backup_bytes).await;
}

#[tokio::test]
async fn a_restore_of_a_database_that_was_never_migrated_is_refused() {
  let (_dir, db) = scratch();
  legacy_database(&db, 2).await;
  let db_contents = contents(&db).await;

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::NotMigrated {
        database_state: "schema 0",
        ..
      })
    ),
    "got {error:?}"
  );
  assert_eq!(contents(&db).await, db_contents);
}

#[tokio::test]
async fn a_restore_is_refused_beside_a_stray_migration_copy() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let db_contents = contents(&db).await;
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  std::fs::write(sidecar(&db, MIGRATING_SUFFIX), b"").unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::MigrationCopyPresent { .. })
    ),
    "got {error:?}"
  );
  assert_untouched(&db, &db_contents, &backup_bytes).await;
}

const INTERRUPTED_KEPT_SUFFIX: &str = ".schema1-20260101T000000Z";
const OLDER_KEPT_SUFFIX: &str = ".schema1-20250101T000000Z";
const SECOND_CYCLE_KEPT_SUFFIX: &str = ".schema1-99991231T235959Z";

/// Copies the database's write-ahead log into it and closes it, as a restore
/// does before its first rename.
async fn checkpoint(db: &Path) {
  let pool = legacy::open_plain(db).await;
  sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
    .execute(&pool)
    .await
    .unwrap();
  pool.close().await;
  assert_no_sidecars(db);
}

/// Migrates a legacy database and moves the schema-1 file to a kept name by
/// hand, as a restore that stopped after its first rename leaves it, and
/// returns the kept path.
async fn interrupted_after_first_rename(db: &Path) -> PathBuf {
  migrated_with_new_mail(db, 2).await;
  checkpoint(db).await;
  let kept = sidecar(db, INTERRUPTED_KEPT_SUFFIX);
  std::fs::rename(db, &kept).unwrap();
  kept
}

/// Asserts a refused resume left the database missing and the backup and
/// kept files as they were.
fn assert_resume_untouched(db: &Path, backup_bytes: &[u8], kept: &[(PathBuf, Vec<u8>)]) {
  assert!(!db.exists());
  assert_eq!(
    std::fs::read(sidecar(db, BACKUP_SUFFIX)).unwrap(),
    backup_bytes
  );
  for (path, bytes) in kept {
    assert_eq!(&std::fs::read(path).unwrap(), bytes);
  }
}

#[tokio::test]
async fn a_restore_interrupted_between_its_renames_is_finished() {
  let (_dir, db) = scratch();
  let kept = interrupted_after_first_rename(&db).await;
  let backup = sidecar(&db, BACKUP_SUFFIX);
  let legacy_bytes = std::fs::read(&backup).unwrap();
  let kept_bytes = std::fs::read(&kept).unwrap();
  let before = legacy_snapshot(&backup).await;

  let report = restore_backup(&db).await.unwrap();

  assert_eq!(
    report,
    RestoreReport {
      database: db.clone(),
      restored_from: backup.clone(),
      kept_as: kept.clone(),
    }
  );
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert!(!backup.exists());
  assert_eq!(std::fs::read(&kept).unwrap(), kept_bytes);
  assert_eq!(kept_files(&db), vec![kept.clone()]);
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
  let pool = legacy::open_plain(&db).await;
  legacy::initialize_as_v0_7_0(&pool).await.unwrap();
  pool.close().await;
  assert_eq!(legacy_snapshot(&db).await, before);
  assert_no_sidecars(&db);
  assert_no_sidecars(&kept);
}

#[tokio::test]
async fn an_interrupted_second_restore_is_finished_beside_the_first_kept_file() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  let first_kept = restore_backup(&db).await.unwrap().kept_as;
  let Preparation::Ready(Some(_)) = migrate(&db).await.unwrap() else {
    panic!("the restored database should be migrated again");
  };
  checkpoint(&db).await;
  let second_kept = sidecar(&db, SECOND_CYCLE_KEPT_SUFFIX);
  std::fs::rename(&db, &second_kept).unwrap();
  let backup = sidecar(&db, BACKUP_SUFFIX);
  let legacy_bytes = std::fs::read(&backup).unwrap();
  let first_kept_bytes = std::fs::read(&first_kept).unwrap();
  let second_kept_bytes = std::fs::read(&second_kept).unwrap();

  let report = restore_backup(&db).await.unwrap();

  assert_eq!(report.kept_as, second_kept);
  assert_eq!(std::fs::read(&db).unwrap(), legacy_bytes);
  assert!(!backup.exists());
  assert_eq!(scalar(&db, "PRAGMA user_version").await, 0);
  assert_eq!(std::fs::read(&first_kept).unwrap(), first_kept_bytes);
  assert_eq!(std::fs::read(&second_kept).unwrap(), second_kept_bytes);
  assert_eq!(kept_files(&db), vec![first_kept, second_kept]);
}

#[tokio::test]
async fn an_interrupted_restore_whose_newest_kept_file_is_not_schema_1_is_refused() {
  let (_dir, db) = scratch();
  let kept = interrupted_after_first_rename(&db).await;
  let older = sidecar(&db, OLDER_KEPT_SUFFIX);
  std::fs::copy(&kept, &older).unwrap();
  std::fs::copy(sidecar(&db, BACKUP_SUFFIX), &kept).unwrap();
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let kept_contents = vec![
    (kept.clone(), std::fs::read(&kept).unwrap()),
    (older.clone(), std::fs::read(&older).unwrap()),
  ];

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::NotAnInterruptedRestore { reason, .. })
        if reason.contains(&kept.display().to_string()) && reason.contains("schema 0")
    ),
    "got {error:?}"
  );
  assert_resume_untouched(&db, &backup_bytes, &kept_contents);
}

#[tokio::test]
async fn a_missing_database_without_a_kept_file_is_refused() {
  let (_dir, db) = scratch();
  migrated_with_new_mail(&db, 2).await;
  checkpoint(&db).await;
  std::fs::remove_file(&db).unwrap();
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::NotAnInterruptedRestore { .. })
    ),
    "got {error:?}"
  );
  assert_resume_untouched(&db, &backup_bytes, &[]);
}

#[tokio::test]
async fn an_interrupted_restore_is_refused_while_the_database_has_a_journal_beside_it() {
  let (_dir, db) = scratch();
  let kept = interrupted_after_first_rename(&db).await;
  let journal = sidecar(&db, JOURNAL_SUFFIX);
  std::fs::write(&journal, b"hot").unwrap();
  let backup_bytes = std::fs::read(sidecar(&db, BACKUP_SUFFIX)).unwrap();
  let kept_contents = vec![(kept.clone(), std::fs::read(&kept).unwrap())];

  let error = restore_backup(&db).await.unwrap_err();

  assert!(
    matches!(
      &error,
      StorageError::RestoreRefused(RestoreRefusal::FileInUse { file, .. }) if *file == journal
    ),
    "got {error:?}"
  );
  assert_resume_untouched(&db, &backup_bytes, &kept_contents);
}
