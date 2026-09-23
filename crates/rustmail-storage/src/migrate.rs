//! Migrating a legacy (schema 0) database file to schema 1 at startup.
//!
//! The legacy file is never written. Schema 1 is built beside it in
//! `<db>.migrating`, in committed batches that a later start resumes, while
//! a write transaction on the legacy file keeps older rustmail versions out.
//! Once the copy is verified the files are swapped: the legacy file becomes
//! `<db>.schema0.bak`, the backup, and the copy becomes `<db>`.
//!
//! Every start of a file database goes through [`prepare_database_file`],
//! which takes `<db>.migration-lock` before it looks at any file and
//! resolves whatever an earlier run left behind:
//!
//! | `db` | `.schema0.bak` | `.migrating` | Action |
//! |---|---|---|---|
//! | absent or empty | absent | absent | fresh: nothing to do |
//! | schema 1 | any | absent | open; the backup is logged |
//! | schema 0 | absent | absent | migrate |
//! | schema 0 | absent | incomplete | resume if made from this `db`, else restart |
//! | schema 0 | absent | complete | swap if made from this `db`, else restart |
//! | absent | present | complete | finish the swap if made from the backup, else refuse |
//! | schema 0 | present | any | refuse: ambiguous |
//! | schema 1 | any | present | refuse: stray copy |
//! | anything else | | | refuse |
//!
//! "Made from" compares the fingerprint the copy stores with one computed
//! from the legacy file under its write lock.
//!
//! [`restore_backup`] undoes a migration under the same lock: it moves the
//! schema-1 `db` to `<db>.schema1-<UTC timestamp>` and the backup back to
//! `db`, which the next start then migrates again. The kept file is none of
//! the three above, so it never changes which state a start sees.

mod build;
mod fingerprint;
mod restore;
#[cfg(test)]
mod tests;

use std::fs::{File, Metadata, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};
use tracing::{error, info, warn};
use ulid::Ulid;

use crate::error::{MigrationRefusal, SQLITE_FULL, StorageError};
use crate::schema::{BUSY_TIMEOUT, FileSchema, probe};
use build::{CopyOutcome, Origin, Target, read_origin};
use fingerprint::{LegacyColumns, SourcePrint, fingerprint};
pub use restore::{RestoreReport, restore_backup};

/// Legacy messages one batch copies at most.
const BATCH_MAX_MESSAGES: i64 = 1_000;
/// Raw bytes one batch copies at most; a larger message is a batch alone.
const BATCH_MAX_RAW_BYTES: i64 = 32 * 1024 * 1024;
/// Times the copy is rebuilt because the legacy file changed under it
/// before the migration gives up.
const MAX_BUILD_ATTEMPTS: u32 = 2;
/// WAL the copy may keep on disk besides itself, for the disk-space estimate.
const TARGET_WAL_ALLOWANCE_BYTES: u64 = 64 * 1024 * 1024;
/// Permission bits a new migration file copies from the database.
#[cfg(unix)]
const MODE_BITS: u32 = 0o7777;

const LOCK_SUFFIX: &str = ".migration-lock";
const MIGRATING_SUFFIX: &str = ".migrating";
const BACKUP_SUFFIX: &str = ".schema0.bak";
const WAL_SUFFIX: &str = "-wal";
const SHM_SUFFIX: &str = "-shm";
const JOURNAL_SUFFIX: &str = "-journal";

/// What [`prepare_database_file`] left the database as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preparation {
  /// The database can be opened with [`crate::initialize_database`]. Carries
  /// the report of a migration this call completed, if any.
  Ready(Option<MigrationReport>),
  /// The migration stopped between batches because it was asked to; the
  /// next call resumes it. The database must not be opened meanwhile.
  Paused {
    /// Messages copied so far.
    migrated: u64,
    /// Messages in the legacy database.
    total: u64,
  },
}

/// What a completed migration did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
  /// Messages migrated.
  pub messages: u64,
  /// Attachments now served from their message's raw source.
  pub attachments_located: u64,
  /// Attachments kept decoded, as before.
  pub attachments_inline: u64,
  /// Attachments ingest would have stored differently, kept with their
  /// legacy contents.
  pub attachment_mismatches: u64,
  /// Wall time of this call.
  pub duration: Duration,
  /// Where the legacy database is kept.
  pub backup_path: PathBuf,
}

/// Tuning of a migration run; production runs use the defaults.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MigrationOptions {
  pub(crate) batch_messages: i64,
  pub(crate) batch_raw_bytes: i64,
  /// Caps the copy's page count, so a test can make it run out of space.
  pub(crate) target_max_pages: Option<u64>,
}

impl Default for MigrationOptions {
  fn default() -> Self {
    Self {
      batch_messages: BATCH_MAX_MESSAGES,
      batch_raw_bytes: BATCH_MAX_RAW_BYTES,
      target_max_pages: None,
    }
  }
}

/// The files that make up one database's migration state.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
  pub(crate) db: PathBuf,
  pub(crate) lock: PathBuf,
  pub(crate) migrating: PathBuf,
  pub(crate) backup: PathBuf,
}

impl Paths {
  pub(crate) fn new(db: &Path) -> Self {
    Self {
      db: db.to_path_buf(),
      lock: sidecar(db, LOCK_SUFFIX),
      migrating: sidecar(db, MIGRATING_SUFFIX),
      backup: sidecar(db, BACKUP_SUFFIX),
    }
  }
}

/// `path` with `suffix` appended to its file name.
pub(crate) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
  let mut name = path.as_os_str().to_owned();
  name.push(suffix);
  PathBuf::from(name)
}

/// Makes the database file at `path` ready to open at the current schema,
/// migrating a legacy database first.
///
/// Takes `<path>.migration-lock` before reading any file state and holds it
/// until the state is resolved, whatever it turns out to be; see the module
/// documentation for the states. `should_stop` is asked before each batch of
/// a migration; once it answers `true` the call returns
/// [`Preparation::Paused`] after the batch in flight commits.
///
/// In-memory databases need none of this: they always start empty.
///
/// # Errors
///
/// Returns [`StorageError::MigrationLocked`] if another process holds the
/// lock, [`StorageError::MigrationRefused`] for a state that needs a manual
/// decision, [`StorageError::MigrationVerifyFailed`] if the copy does not
/// match the legacy database, [`StorageError::MigrationDiskFull`] if the
/// disk fills, [`StorageError::NewerSchema`] or
/// [`StorageError::UnrecognizedSchema`] for a file rustmail cannot open, and
/// [`StorageError::Database`] or [`StorageError::MigrationIo`] if SQLite or
/// the filesystem fails. The legacy database is left untouched by every one
/// of them.
pub async fn prepare_database_file(
  path: &Path,
  should_stop: impl Fn() -> bool + Send + Sync,
) -> Result<Preparation, StorageError> {
  prepare_with(path, &MigrationOptions::default(), &should_stop).await
}

pub(crate) async fn prepare_with(
  path: &Path,
  options: &MigrationOptions,
  should_stop: &(dyn Fn() -> bool + Send + Sync),
) -> Result<Preparation, StorageError> {
  let paths = Paths::new(path);
  let _lock = acquire_lock(&paths).await?;
  let db = inspect_db(&paths.db).await?;
  let backup = exists(&paths.backup).await?;
  let copy = inspect_copy(&paths.migrating).await?;
  match (db, backup, copy) {
    (DbState::Absent | DbState::Empty, false, CopyState::Absent) => Ok(Preparation::Ready(None)),
    (DbState::Current, backup, CopyState::Absent) => {
      if backup {
        log_backup(&paths.backup).await?;
      }
      Ok(Preparation::Ready(None))
    }
    (DbState::Current, _, _) => Err(
      MigrationRefusal::StrayMigrationCopy {
        database: paths.db.clone(),
        migrating: paths.migrating.clone(),
      }
      .into(),
    ),
    (DbState::Legacy, false, copy) => Migration::new(&paths, options).run(copy, should_stop).await,
    (DbState::Legacy, true, copy) => Err(
      MigrationRefusal::AmbiguousBackup {
        database: paths.db.clone(),
        backup: paths.backup.clone(),
        migrating: (copy != CopyState::Absent).then(|| paths.migrating.clone()),
      }
      .into(),
    ),
    (DbState::Absent, true, CopyState::Complete(origin)) => {
      finish_interrupted_swap(&paths, origin).await?;
      Ok(Preparation::Ready(None))
    }
    (db, backup, copy) => Err(
      MigrationRefusal::UnknownState {
        database: paths.db.clone(),
        database_state: db.describe(),
        backup: paths.backup.clone(),
        backup_state: if backup { "present" } else { "absent" },
        migrating: paths.migrating.clone(),
        migrating_state: copy.describe(),
      }
      .into(),
    ),
  }
}

/// What the database file is, read without writing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbState {
  Absent,
  Empty,
  Legacy,
  Current,
}

impl DbState {
  fn describe(self) -> &'static str {
    match self {
      Self::Absent => "absent",
      Self::Empty => "empty",
      Self::Legacy => "schema 0",
      Self::Current => "schema 1",
    }
  }
}

/// What the migration copy is, read without writing it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CopyState {
  Absent,
  /// `user_version` 0: still being built.
  Incomplete(Option<Origin>),
  /// `user_version` 1: built, verified and finalized.
  Complete(Option<Origin>),
}

impl CopyState {
  fn describe(&self) -> &'static str {
    match self {
      Self::Absent => "absent",
      Self::Incomplete(_) => "incomplete",
      Self::Complete(_) => "complete",
    }
  }
}

/// Holds `<db>.migration-lock` until dropped.
struct MigrationLock {
  _file: File,
}

async fn acquire_lock(paths: &Paths) -> Result<MigrationLock, StorageError> {
  let db = paths.db.clone();
  let lock = paths.lock.clone();
  let result = blocking(move || {
    let mode = mode_of(&db)?;
    let file = open_or_create(&lock, mode)?;
    match file.try_lock() {
      Ok(()) => Ok(MigrationLock { _file: file }),
      Err(TryLockError::WouldBlock) => Err(StorageError::MigrationLocked { lock_path: lock }),
      Err(TryLockError::Error(source)) => Err(StorageError::MigrationIo {
        action: "lock",
        path: lock,
        source,
      }),
    }
  })
  .await;
  if let Err(StorageError::MigrationLocked { lock_path }) = &result {
    error!(
      event = "storage_migration_locked",
      lock_path = %lock_path.display(),
      "Another rustmail process holds the storage migration lock"
    );
  }
  result
}

/// Opens the lock file, creating it with the database's permissions.
fn open_or_create(path: &Path, mode: Option<u32>) -> Result<File, StorageError> {
  match create_with_mode(path, mode) {
    Err(StorageError::MigrationIo { source, .. })
      if source.kind() == io::ErrorKind::AlreadyExists =>
    {
      OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error("open", path, source))
    }
    result => result,
  }
}

/// Creates the file at `path`, which must not exist, with permissions `mode`.
fn create_with_mode(path: &Path, mode: Option<u32>) -> Result<File, StorageError> {
  let file = OpenOptions::new()
    .read(true)
    .write(true)
    .create_new(true)
    .open(path)
    .map_err(|source| io_error("create", path, source))?;
  if let Some(mode) = mode {
    set_mode(&file, mode).map_err(|source| io_error("set the permissions of", path, source))?;
  }
  Ok(file)
}

#[cfg(unix)]
fn mode_bits(metadata: &Metadata) -> Option<u32> {
  use std::os::unix::fs::PermissionsExt;
  Some(metadata.permissions().mode() & MODE_BITS)
}

#[cfg(not(unix))]
fn mode_bits(_metadata: &Metadata) -> Option<u32> {
  None
}

#[cfg(unix)]
fn set_mode(file: &File, mode: u32) -> io::Result<()> {
  use std::os::unix::fs::PermissionsExt;
  file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: u32) -> io::Result<()> {
  Ok(())
}

/// The permission bits of the file at `path`, or `None` if it is absent.
fn mode_of(path: &Path) -> Result<Option<u32>, StorageError> {
  match std::fs::metadata(path) {
    Ok(metadata) => Ok(mode_bits(&metadata)),
    Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
    Err(source) => Err(io_error("read the permissions of", path, source)),
  }
}

fn io_error(action: &'static str, path: &Path, source: io::Error) -> StorageError {
  StorageError::MigrationIo {
    action,
    path: path.to_path_buf(),
    source,
  }
}

/// Runs a filesystem operation on the blocking pool.
async fn blocking<T, F>(operation: F) -> Result<T, StorageError>
where
  T: Send + 'static,
  F: FnOnce() -> Result<T, StorageError> + Send + 'static,
{
  tokio::task::spawn_blocking(operation)
    .await
    .map_err(StorageError::MigrationTaskAborted)?
}

async fn exists(path: &Path) -> Result<bool, StorageError> {
  let path = path.to_path_buf();
  blocking(move || {
    path
      .try_exists()
      .map_err(|source| io_error("check for", &path, source))
  })
  .await
}

async fn file_len(path: &Path) -> Result<u64, StorageError> {
  let path = path.to_path_buf();
  blocking(move || match std::fs::metadata(&path) {
    Ok(metadata) => Ok(metadata.len()),
    Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(0),
    Err(source) => Err(io_error("read the size of", &path, source)),
  })
  .await
}

/// Connection options that open an existing file as it is: no tuning, no
/// schema changes, and no file created if it is missing.
fn existing_file(path: &Path) -> SqliteConnectOptions {
  SqliteConnectOptions::new()
    .filename(path)
    .create_if_missing(false)
    .busy_timeout(BUSY_TIMEOUT)
    .foreign_keys(false)
}

async fn inspect_db(path: &Path) -> Result<DbState, StorageError> {
  if !exists(path).await? {
    return Ok(DbState::Absent);
  }
  let mut conn = SqliteConnection::connect_with(&existing_file(path)).await?;
  let schema = probe(&mut conn).await;
  conn.close().await?;
  Ok(match schema? {
    FileSchema::Empty => DbState::Empty,
    FileSchema::Legacy => DbState::Legacy,
    FileSchema::Current => DbState::Current,
  })
}

async fn inspect_copy(path: &Path) -> Result<CopyState, StorageError> {
  if !exists(path).await? {
    return Ok(CopyState::Absent);
  }
  let mut conn = SqliteConnection::connect_with(&existing_file(path)).await?;
  let read = async {
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
      .fetch_one(&mut conn)
      .await?;
    let origin = read_origin(&mut conn).await?;
    Ok::<_, StorageError>((version, origin))
  }
  .await;
  conn.close().await?;
  let (version, origin) = read?;
  Ok(if version == crate::SCHEMA_VERSION {
    CopyState::Complete(origin)
  } else {
    CopyState::Incomplete(origin)
  })
}

async fn log_backup(backup: &Path) -> Result<(), StorageError> {
  let bytes = file_len(backup).await?;
  info!(
    event = "storage_backup",
    backup_path = %backup.display(),
    backup_bytes = bytes,
    "The database from before the storage migration is kept; delete it once you no longer need it"
  );
  Ok(())
}

/// The first rename of a swap happened and the second did not: the copy is
/// renamed into place if it was made from the backup.
async fn finish_interrupted_swap(
  paths: &Paths,
  origin: Option<Origin>,
) -> Result<(), StorageError> {
  let mut conn = SqliteConnection::connect_with(&existing_file(&paths.backup)).await?;
  let print = async {
    sqlx::query("BEGIN").execute(&mut conn).await?;
    let columns = LegacyColumns::probe(&mut conn, "main").await?;
    let print = fingerprint(&mut conn, &columns).await?;
    sqlx::query("ROLLBACK").execute(&mut conn).await?;
    Ok::<_, StorageError>(print)
  }
  .await;
  conn.close().await?;
  let print = print?;
  if origin.is_none_or(|origin| origin.fingerprint != print.fingerprint) {
    return Err(
      MigrationRefusal::BackupChanged {
        database: paths.db.clone(),
        backup: paths.backup.clone(),
        migrating: paths.migrating.clone(),
      }
      .into(),
    );
  }
  refuse_open_files(paths, &[&paths.backup, &paths.migrating]).await?;
  rename(&paths.migrating, &paths.db).await?;
  sync_parent(&paths.db).await?;
  info!(
    event = "storage_migration",
    phase = "swap",
    backup_path = %paths.backup.display(),
    "Finished a storage migration swap an earlier run was interrupted in"
  );
  Ok(())
}

/// Refuses if a `-wal` or `-shm` sits beside any of `files`: some process
/// still has that database open.
async fn refuse_open_files(paths: &Paths, files: &[&Path]) -> Result<(), StorageError> {
  for file in files {
    for suffix in [WAL_SUFFIX, SHM_SUFFIX] {
      let sidecar = sidecar(file, suffix);
      if exists(&sidecar).await? {
        return Err(
          MigrationRefusal::FileInUse {
            database: paths.db.clone(),
            file: sidecar,
          }
          .into(),
        );
      }
    }
  }
  Ok(())
}

async fn rename(from: &Path, to: &Path) -> Result<(), StorageError> {
  let from = from.to_path_buf();
  let to = to.to_path_buf();
  blocking(move || std::fs::rename(&from, &to).map_err(|source| io_error("rename", &from, source)))
    .await
}

/// Makes the renames in `path`'s directory durable.
async fn sync_parent(path: &Path) -> Result<(), StorageError> {
  let dir = match path.parent() {
    Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
    _ => PathBuf::from("."),
  };
  blocking(move || {
    File::open(&dir)
      .and_then(|handle| handle.sync_all())
      .map_err(|source| io_error("sync the directory", &dir, source))
  })
  .await
}

/// Removes the copy and its SQLite side files, whichever exist.
async fn remove_copy(paths: &Paths) -> Result<(), StorageError> {
  let files: Vec<PathBuf> = [WAL_SUFFIX, SHM_SUFFIX, JOURNAL_SUFFIX]
    .iter()
    .map(|suffix| sidecar(&paths.migrating, suffix))
    .chain(std::iter::once(paths.migrating.clone()))
    .collect();
  blocking(move || {
    for file in &files {
      match std::fs::remove_file(file) {
        Ok(()) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("remove", file, source)),
      }
    }
    Ok(())
  })
  .await
}

/// A connection on the legacy database holding its write lock.
///
/// An older rustmail still running on the file gets `SQLITE_BUSY` for as long
/// as this is held, so the file cannot change between two fingerprints.
struct Source {
  conn: SqliteConnection,
}

impl Source {
  async fn lock(path: &Path) -> Result<Self, StorageError> {
    let mut conn = SqliteConnection::connect_with(&existing_file(path)).await?;
    if let Err(error) = sqlx::query("BEGIN IMMEDIATE").execute(&mut conn).await {
      conn.close().await?;
      return Err(error.into());
    }
    Ok(Self { conn })
  }

  async fn fingerprint(&mut self, columns: &LegacyColumns) -> Result<SourcePrint, StorageError> {
    fingerprint(&mut self.conn, columns).await
  }

  /// Ends the transaction without writing and closes the connection, which
  /// removes the file's `-wal` and `-shm` unless another process has it open.
  async fn release(mut self) -> Result<(), StorageError> {
    sqlx::query("ROLLBACK").execute(&mut self.conn).await?;
    self.conn.close().await?;
    Ok(())
  }
}

/// Where a run starts building, decided under the source lock.
enum Start {
  Fresh,
  Resume,
  Complete,
}

/// How far a locked run got.
enum Built {
  Swappable(build::AttachmentCounts, u64),
  Paused { migrated: u64, total: u64 },
}

/// One migration run of a legacy database.
struct Migration<'a> {
  paths: &'a Paths,
  options: &'a MigrationOptions,
  id: String,
  started: Instant,
}

impl<'a> Migration<'a> {
  fn new(paths: &'a Paths, options: &'a MigrationOptions) -> Self {
    Self {
      paths,
      options,
      id: Ulid::new().to_string(),
      started: Instant::now(),
    }
  }

  async fn run(
    self,
    copy: CopyState,
    should_stop: &(dyn Fn() -> bool + Send + Sync),
  ) -> Result<Preparation, StorageError> {
    let result = self.run_steps(copy, should_stop).await;
    if let Err(error) = &result {
      error!(
        event = "storage_migration",
        phase = "failed",
        migration_id = %self.id,
        error = %error,
        hint = "the existing database is untouched; fix the cause and restart to resume",
        "Storage migration failed"
      );
    }
    result
  }

  async fn run_steps(
    &self,
    copy: CopyState,
    should_stop: &(dyn Fn() -> bool + Send + Sync),
  ) -> Result<Preparation, StorageError> {
    let mut source = Source::lock(&self.paths.db).await?;
    let built = self.build_locked(&mut source, copy, should_stop).await;
    let released = source.release().await;
    let (counts, messages) = match built? {
      Built::Paused { migrated, total } => {
        released?;
        info!(
          event = "storage_migration",
          phase = "copy",
          migration_id = %self.id,
          migrated,
          total,
          "Storage migration paused at {migrated}/{total}; it resumes on next start"
        );
        return Ok(Preparation::Paused { migrated, total });
      }
      Built::Swappable(counts, messages) => (counts, messages),
    };
    released?;
    self.swap().await?;
    let duration = self.started.elapsed();
    info!(
      event = "storage_migration",
      phase = "done",
      migration_id = %self.id,
      messages,
      attachments_located = counts.located,
      attachments_inline = counts.inline,
      attachment_mismatches = counts.mismatches,
      duration_s = duration.as_secs_f64(),
      backup_path = %self.paths.backup.display(),
      "Storage migration complete; the previous database is kept as the backup"
    );
    Ok(Preparation::Ready(Some(MigrationReport {
      messages,
      attachments_located: counts.located,
      attachments_inline: counts.inline,
      attachment_mismatches: counts.mismatches,
      duration,
      backup_path: self.paths.backup.clone(),
    })))
  }

  async fn build_locked(
    &self,
    source: &mut Source,
    copy: CopyState,
    should_stop: &(dyn Fn() -> bool + Send + Sync),
  ) -> Result<Built, StorageError> {
    let columns = LegacyColumns::probe(&mut source.conn, "main").await?;
    let mut print = source.fingerprint(&columns).await?;
    let source_bytes = file_len(&self.paths.db).await?;
    let (mut start, fingerprint_match) = match &copy {
      CopyState::Absent => (Start::Fresh, None),
      CopyState::Incomplete(origin) => match origin {
        Some(origin) if origin.fingerprint == print.fingerprint => (Start::Resume, Some(true)),
        _ => (Start::Fresh, Some(false)),
      },
      CopyState::Complete(origin) => match origin {
        Some(origin) if origin.fingerprint == print.fingerprint => (Start::Complete, Some(true)),
        _ => (Start::Fresh, Some(false)),
      },
    };
    info!(
      event = "storage_migration",
      phase = "start",
      migration_id = %self.id,
      messages = print.messages,
      attachments = print.attachments,
      source_bytes,
      estimated_bytes = print.raw_bytes,
      fingerprint_match = ?fingerprint_match,
      database = %self.paths.db.display(),
      "Migrating the database to the current storage schema; SMTP and HTTP start once it is done"
    );
    for attempt in 1..=MAX_BUILD_ATTEMPTS {
      if let Start::Fresh = start {
        remove_copy(self.paths).await?;
      }
      let counts = match start {
        Start::Complete => None,
        Start::Fresh | Start::Resume => {
          match self
            .build(matches!(start, Start::Fresh), &print, should_stop)
            .await
          {
            Ok(Some(counts)) => Some(counts),
            Ok(None) => {
              let migrated = self.copied_messages().await?;
              return Ok(Built::Paused {
                migrated,
                total: print.messages,
              });
            }
            Err(error) if error.sqlite_primary_code() == Some(SQLITE_FULL) => {
              return Err(self.disk_full().await?);
            }
            Err(error) => return Err(error),
          }
        }
      };
      let now = source.fingerprint(&columns).await?;
      if now.fingerprint == print.fingerprint {
        let counts = match counts {
          Some(counts) => counts,
          None => self.completed_counts().await?,
        };
        return Ok(Built::Swappable(counts, print.messages));
      }
      warn!(
        event = "storage_migration",
        phase = "verify",
        migration_id = %self.id,
        fingerprint_match = false,
        attempt,
        "The database changed since its copy was made; rebuilding the copy"
      );
      print = now;
      start = Start::Fresh;
    }
    Err(
      MigrationRefusal::SourceUnstable {
        database: self.paths.db.clone(),
      }
      .into(),
    )
  }

  /// Builds a fresh copy, or resumes the existing one. `None` means it was
  /// asked to stop.
  async fn build(
    &self,
    fresh: bool,
    print: &SourcePrint,
    should_stop: &(dyn Fn() -> bool + Send + Sync),
  ) -> Result<Option<build::AttachmentCounts>, StorageError> {
    let mut target = if fresh {
      let db = self.paths.db.clone();
      let migrating = self.paths.migrating.clone();
      blocking(move || create_with_mode(&migrating, mode_of(&db)?).map(drop)).await?;
      Target::create(
        &self.paths.migrating,
        &self.paths.db,
        &print.fingerprint,
        self.options,
      )
      .await?
    } else {
      Target::open(&self.paths.migrating, &self.paths.db, self.options).await?
    };
    let result = async {
      let columns = target.legacy_columns().await?;
      let outcome = target
        .copy(&columns, self.options, print.messages, &self.id, should_stop)
        .await?;
      if let CopyOutcome::Paused { .. } = outcome {
        return Ok(None);
      }
      info!(event = "storage_migration", phase = "verify", migration_id = %self.id, "Verifying the migrated copy");
      target.verify(&columns, &self.paths.db).await.map(Some)
    }
    .await;
    match result {
      Ok(Some(counts)) => {
        target.finalize().await?;
        Ok(Some(counts))
      }
      Ok(None) => {
        target.close().await?;
        Ok(None)
      }
      Err(error) => {
        let _ = target.close().await;
        Err(error)
      }
    }
  }

  async fn copied_messages(&self) -> Result<u64, StorageError> {
    let mut conn = SqliteConnection::connect_with(&existing_file(&self.paths.migrating)).await?;
    let count: Result<i64, sqlx::Error> = sqlx::query_scalar("SELECT count(*) FROM messages")
      .fetch_one(&mut conn)
      .await;
    conn.close().await?;
    Ok(u64::try_from(count?).unwrap_or(0))
  }

  /// Attachment counts of a copy an earlier run completed.
  async fn completed_counts(&self) -> Result<build::AttachmentCounts, StorageError> {
    let mut conn = SqliteConnection::connect_with(&existing_file(&self.paths.migrating)).await?;
    let counts: Result<(i64, i64, i64), sqlx::Error> = sqlx::query_as(
      "SELECT (SELECT count(*) FROM attachments WHERE raw_offset IS NOT NULL), \
              (SELECT count(*) FROM attachments WHERE raw_offset IS NULL), \
              (SELECT attachment_mismatches FROM migration_origin WHERE id = 1)",
    )
    .fetch_one(&mut conn)
    .await;
    conn.close().await?;
    let (located, inline, mismatches) = counts?;
    Ok(build::AttachmentCounts {
      located: u64::try_from(located).unwrap_or(0),
      inline: u64::try_from(inline).unwrap_or(0),
      mismatches: u64::try_from(mismatches).unwrap_or(0),
    })
  }

  /// The error for a copy that ran out of space, with the space it needs.
  async fn disk_full(&self) -> Result<StorageError, StorageError> {
    let source = file_len(&self.paths.db).await?;
    let written = file_len(&self.paths.migrating).await?
      + file_len(&sidecar(&self.paths.migrating, WAL_SUFFIX)).await?;
    Ok(StorageError::MigrationDiskFull {
      database: self.paths.db.clone(),
      needed_bytes: (source + TARGET_WAL_ALLOWANCE_BYTES).saturating_sub(written),
    })
  }

  /// Renames the legacy database to the backup and the copy into its place.
  async fn swap(&self) -> Result<(), StorageError> {
    refuse_open_files(self.paths, &[&self.paths.db, &self.paths.migrating]).await?;
    if exists(&self.paths.backup).await? {
      return Err(
        MigrationRefusal::AmbiguousBackup {
          database: self.paths.db.clone(),
          backup: self.paths.backup.clone(),
          migrating: Some(self.paths.migrating.clone()),
        }
        .into(),
      );
    }
    rename(&self.paths.db, &self.paths.backup).await?;
    rename(&self.paths.migrating, &self.paths.db).await?;
    sync_parent(&self.paths.db).await?;
    info!(
      event = "storage_migration",
      phase = "swap",
      migration_id = %self.id,
      backup_path = %self.paths.backup.display(),
      "Swapped the migrated copy into place"
    );
    Ok(())
  }
}
