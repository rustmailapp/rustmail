//! Putting the backup of a migration back in place of the migrated database.

use std::path::{Path, PathBuf};

use sqlx::{Connection, SqliteConnection};
use time::OffsetDateTime;
use tracing::info;

use super::{
  DbState, JOURNAL_SUFFIX, Paths, SHM_SUFFIX, WAL_SUFFIX, acquire_lock, existing_file, exists,
  inspect_db, rename, sidecar, sync_parent,
};
use crate::error::{RestoreRefusal, StorageError};
use crate::schema::{FileSchema, probe};

/// Suffix of the name the migrated database is kept under, before its
/// timestamp.
const KEPT_SUFFIX: &str = ".schema1-";
/// The files beside a database that mean a process has it open or that
/// SQLite would apply to it on the next open.
const SIDE_SUFFIXES: [&str; 3] = [WAL_SUFFIX, SHM_SUFFIX, JOURNAL_SUFFIX];
/// What `PRAGMA quick_check` answers for an intact file.
const QUICK_CHECK_OK: &str = "ok";

/// What [`restore_backup`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
  /// The database path, which now holds the restored schema-0 file.
  pub database: PathBuf,
  /// Where the restored file was kept as the backup, `<db>.schema0.bak`.
  pub restored_from: PathBuf,
  /// Where the migrated schema-1 database was moved,
  /// `<db>.schema1-<UTC timestamp>`. Mail received since the migration is
  /// only in this file.
  pub kept_as: PathBuf,
}

/// Puts the database kept by a migration back at `path`, so a rustmail from
/// before schema 1 can open it again.
///
/// Takes `<path>.migration-lock` first. Requires `path` to be a schema-1
/// database with no migration copy beside it, and `<path>.schema0.bak` to be
/// an intact schema-0 database that no process has open. The schema-1
/// database is checkpointed and closed, then moved to
/// `<path>.schema1-<UTC timestamp>` and never deleted; the backup is moved to
/// `path` and the directory is synced. Neither file keeps a `-wal`, `-shm` or
/// `-journal`, so nothing of one can be applied to the other.
///
/// # Errors
///
/// Returns [`StorageError::MigrationLocked`] if another process holds the
/// lock, [`StorageError::RestoreRefused`] for files a restore cannot start
/// from, including a database another process still has open,
/// [`StorageError::NewerSchema`] or [`StorageError::UnrecognizedSchema`] for a
/// database rustmail cannot read, and [`StorageError::Database`] or
/// [`StorageError::MigrationIo`] if SQLite or the filesystem fails. Nothing is
/// renamed unless every check passed.
pub async fn restore_backup(path: &Path) -> Result<RestoreReport, StorageError> {
  let paths = Paths::new(path);
  let _lock = acquire_lock(&paths).await?;
  match inspect_db(&paths.db).await? {
    DbState::Current => {}
    state => {
      return Err(
        RestoreRefusal::NotMigrated {
          database: paths.db.clone(),
          database_state: state.describe(),
        }
        .into(),
      );
    }
  }
  if exists(&paths.migrating).await? {
    return Err(
      RestoreRefusal::MigrationCopyPresent {
        database: paths.db.clone(),
        migrating: paths.migrating.clone(),
      }
      .into(),
    );
  }
  if !exists(&paths.backup).await? {
    return Err(
      RestoreRefusal::NoBackup {
        database: paths.db.clone(),
        backup: paths.backup.clone(),
      }
      .into(),
    );
  }
  refuse_side_files(&paths.db, &paths.backup).await?;
  check_backup(&paths.backup).await?;
  refuse_side_files(&paths.db, &paths.backup).await?;
  checkpoint(&paths.db).await?;
  refuse_side_files(&paths.db, &paths.db).await?;

  let kept_as = sidecar(&paths.db, &format!("{KEPT_SUFFIX}{}", timestamp()));
  if exists(&kept_as).await? {
    return Err(RestoreRefusal::KeptNameTaken { kept: kept_as }.into());
  }
  rename(&paths.db, &kept_as).await?;
  rename(&paths.backup, &paths.db).await?;
  sync_parent(&paths.db).await?;
  info!(
    event = "storage_restore",
    from = %paths.backup.display(),
    to = %paths.db.display(),
    kept_as = %kept_as.display(),
    "Restored the database from before the storage migration"
  );
  Ok(RestoreReport {
    database: paths.db,
    restored_from: paths.backup,
    kept_as,
  })
}

/// Refuses if `file` has a `-wal`, `-shm` or `-journal` beside it.
async fn refuse_side_files(database: &Path, file: &Path) -> Result<(), StorageError> {
  for suffix in SIDE_SUFFIXES {
    let side = sidecar(file, suffix);
    if exists(&side).await? {
      return Err(
        RestoreRefusal::FileInUse {
          database: database.to_path_buf(),
          file: side,
        }
        .into(),
      );
    }
  }
  Ok(())
}

/// Requires the backup to be an intact schema-0 database, reading it without
/// writing it.
async fn check_backup(backup: &Path) -> Result<(), StorageError> {
  let invalid = |reason: String| RestoreRefusal::BackupInvalid {
    backup: backup.to_path_buf(),
    reason,
  };
  let mut conn = SqliteConnection::connect_with(&existing_file(backup))
    .await
    .map_err(|error| invalid(error.to_string()))?;
  let checked = async {
    let schema = probe(&mut conn).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
      .fetch_one(&mut conn)
      .await?;
    Ok::<_, StorageError>((schema, integrity))
  }
  .await;
  conn.close().await?;
  match checked {
    Ok((FileSchema::Legacy, integrity)) if integrity == QUICK_CHECK_OK => Ok(()),
    Ok((FileSchema::Legacy, integrity)) => Err(invalid(format!("quick_check: {integrity}")).into()),
    Ok((FileSchema::Empty, _)) => Err(invalid("it has no tables".to_string()).into()),
    Ok((FileSchema::Current, _)) => Err(invalid("it is schema 1".to_string()).into()),
    Err(error) => Err(invalid(error.to_string()).into()),
  }
}

/// Copies the database's write-ahead log into it and closes it, which
/// removes the `-wal` and `-shm` unless another process has it open.
async fn checkpoint(database: &Path) -> Result<(), StorageError> {
  let mut conn = SqliteConnection::connect_with(&existing_file(database)).await?;
  let result: Result<(i64, i64, i64), sqlx::Error> =
    sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
      .fetch_one(&mut conn)
      .await;
  conn.close().await?;
  let (busy, _, _) = result?;
  if busy != 0 {
    return Err(
      RestoreRefusal::CheckpointBusy {
        database: database.to_path_buf(),
      }
      .into(),
    );
  }
  Ok(())
}

/// The current UTC time as `YYYYMMDDTHHMMSSZ`.
fn timestamp() -> String {
  let now = OffsetDateTime::now_utc();
  format!(
    "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
    now.year(),
    u8::from(now.month()),
    now.day(),
    now.hour(),
    now.minute(),
    now.second()
  )
}
