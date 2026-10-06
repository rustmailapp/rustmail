//! Putting the backup of a migration back in place of the migrated database.

use std::path::{Path, PathBuf};

use sqlx::{Connection, SqliteConnection};
use time::OffsetDateTime;
use tracing::info;

use super::{
  DbState, JOURNAL_SUFFIX, Paths, SHM_SUFFIX, WAL_SUFFIX, acquire_lock, blocking, existing_file,
  exists, inspect_db, io_error, rename, sidecar, sync_parent,
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
/// Digits in the date part of a kept file's timestamp, `YYYYMMDD`.
const DATE_DIGITS: usize = 8;
/// Digits in the time part of a kept file's timestamp, `HHMMSS`.
const TIME_DIGITS: usize = 6;

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
/// A restore that stopped between those two renames is finished instead:
/// when `path` is missing, the backup passes the same checks, nothing has
/// `path` or the backup open, and exactly one `<path>.schema1-<UTC timestamp>`
/// sits beside them and is a schema-1 database, the backup is moved to `path`
/// and the directory is synced. That kept file is left as it is.
///
/// # Errors
///
/// Returns [`StorageError::MigrationLocked`] if another process holds the
/// lock, [`StorageError::RestoreRefused`] for files a restore cannot start
/// from or finish, including a database another process still has open,
/// [`StorageError::NewerSchema`] or [`StorageError::UnrecognizedSchema`] for a
/// database rustmail cannot read, and [`StorageError::Database`] or
/// [`StorageError::MigrationIo`] if SQLite or the filesystem fails. Nothing is
/// renamed unless every check passed.
pub async fn restore_backup(path: &Path) -> Result<RestoreReport, StorageError> {
  let paths = Paths::new(path);
  let _lock = acquire_lock(&paths).await?;
  let state = inspect_db(&paths.db).await?;
  let interrupted = state == DbState::Absent && exists(&paths.backup).await?;
  if !interrupted && state != DbState::Current {
    return Err(
      RestoreRefusal::NotMigrated {
        database: paths.db.clone(),
        database_state: state.describe(),
      }
      .into(),
    );
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
  if interrupted {
    return finish_interrupted_restore(paths).await;
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
  put_backup_in_place(paths, kept_as).await
}

/// Finishes a restore that moved the schema-1 database away but not the
/// backup into its place, after the same checks a restore starts with.
async fn finish_interrupted_restore(paths: Paths) -> Result<RestoreReport, StorageError> {
  let kept_as = interrupted_kept_file(&paths).await?;
  refuse_side_files(&paths.db, &paths.db).await?;
  refuse_side_files(&paths.db, &paths.backup).await?;
  check_backup(&paths.backup).await?;
  refuse_side_files(&paths.db, &paths.backup).await?;
  refuse_side_files(&paths.db, &paths.db).await?;
  info!(
    event = "storage_restore_resume",
    database = %paths.db.display(),
    kept_as = %kept_as.display(),
    "Finishing a restore that stopped between its two renames"
  );
  put_backup_in_place(paths, kept_as).await
}

/// Moves the backup to the database path, now free, and syncs the directory.
async fn put_backup_in_place(
  paths: Paths,
  kept_as: PathBuf,
) -> Result<RestoreReport, StorageError> {
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

/// The schema-1 database an interrupted restore moved away: the only
/// `<db>.schema1-<UTC timestamp>` beside the missing database, which must be a
/// schema-1 database.
async fn interrupted_kept_file(paths: &Paths) -> Result<PathBuf, StorageError> {
  let refuse = |reason: String| -> StorageError {
    RestoreRefusal::NotAnInterruptedRestore {
      database: paths.db.clone(),
      backup: paths.backup.clone(),
      reason,
    }
    .into()
  };
  let mut kept = kept_files(&paths.db).await?;
  let kept_as = match kept.len() {
    0 => {
      return Err(refuse(format!(
        "no {}<UTC timestamp> file sits beside it",
        sidecar(&paths.db, KEPT_SUFFIX).display()
      )));
    }
    1 => kept.remove(0),
    count => {
      let names: Vec<String> = kept.iter().map(|file| file.display().to_string()).collect();
      return Err(refuse(format!(
        "{count} files could be the schema-1 database it moved away ({})",
        names.join(", ")
      )));
    }
  };
  match inspect_db(&kept_as).await {
    Ok(DbState::Current) => Ok(kept_as),
    Ok(state) => Err(refuse(format!(
      "{} is {}, not schema 1",
      kept_as.display(),
      state.describe()
    ))),
    Err(error) => Err(refuse(format!(
      "{} could not be read as a database ({error})",
      kept_as.display()
    ))),
  }
}

/// The files named `<db>.schema1-<UTC timestamp>` beside `db`, sorted.
async fn kept_files(db: &Path) -> Result<Vec<PathBuf>, StorageError> {
  let parent = db.parent().unwrap_or(Path::new("")).to_path_buf();
  let listed = if parent.as_os_str().is_empty() {
    PathBuf::from(".")
  } else {
    parent.clone()
  };
  let prefix = sidecar(db, KEPT_SUFFIX).to_string_lossy().into_owned();
  blocking(move || {
    let list_error = |source| io_error("list the directory", &listed, source);
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(&listed).map_err(list_error)? {
      let path = parent.join(entry.map_err(list_error)?.file_name());
      if path
        .to_string_lossy()
        .strip_prefix(prefix.as_str())
        .is_some_and(is_timestamp)
      {
        kept.push(path);
      }
    }
    kept.sort();
    Ok(kept)
  })
  .await
}

/// Whether `text` has the shape [`timestamp`] writes, `YYYYMMDDTHHMMSSZ`.
fn is_timestamp(text: &str) -> bool {
  let all_digits =
    |part: &str, len: usize| part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit());
  text
    .strip_suffix('Z')
    .and_then(|rest| rest.split_once('T'))
    .is_some_and(|(date, time)| all_digits(date, DATE_DIGITS) && all_digits(time, TIME_DIGITS))
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
