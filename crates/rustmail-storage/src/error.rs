use std::path::PathBuf;

/// Errors returned by the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
  /// An underlying SQLite/sqlx error occurred.
  #[error("Database error: {0}")]
  Database(#[from] sqlx::Error),
  /// The requested message or attachment was not found.
  #[error("Message not found: {0}")]
  NotFound(String),
  /// The database was written by a newer rustmail, in a schema this binary
  /// cannot read. It is refused before anything is written to it.
  #[error(
    "{database} is schema {found}, written by a newer rustmail; \
     this binary supports schema {supported}. Upgrade rustmail, or run `rustmail restore-backup` with that newer binary."
  )]
  NewerSchema {
    /// The database file, as SQLite reports it.
    database: String,
    /// The schema version recorded in the file's `user_version`.
    found: i64,
    /// The newest schema version this binary supports.
    supported: i64,
  },
  /// The database predates schema versioning: rustmail 0.7 and earlier kept
  /// every message in one `messages` row. It is refused before anything is
  /// written to it; [`crate::prepare_database_file`] migrates it first.
  #[error(
    "{database} is schema 0, written by rustmail 0.7 or earlier; \
     this binary supports schema {supported} and migrates such a file before opening it. \
     Start rustmail on it to migrate it, or open it with the rustmail that wrote it."
  )]
  LegacySchema {
    /// The database file, as SQLite reports it.
    database: String,
    /// The schema version this binary supports.
    supported: i64,
  },
  /// A located attachment did not decode, from its message's raw source, to
  /// the size recorded at ingest. Nothing is served in its place.
  #[error(
    "attachment {attachment_id} of message {message_id} does not decode to its recorded size \
     from the raw message; its locator or the raw source is damaged. Re-send the message."
  )]
  AttachmentCorrupt {
    /// The message that carries the attachment.
    message_id: String,
    /// The attachment that failed its check.
    attachment_id: String,
    /// The stored `transfer_encoding` code, if any.
    transfer_encoding: Option<i64>,
    /// The decoded size recorded at ingest, if any.
    expected_size: Option<i64>,
    /// The size the located bytes decoded to, or `None` if they did not
    /// decode or lie outside the raw message.
    actual_size: Option<i64>,
  },
  /// The blocking task decoding a large attachment did not complete.
  #[error("decoding a stored attachment did not complete: {0}")]
  DecodeAborted(#[from] tokio::task::JoinError),
  /// The database holds tables rustmail did not create. It is refused before
  /// anything is written to it.
  #[error(
    "{database} is not a rustmail database: schema {found} with unexpected tables ({}). \
     Point rustmail at a new database file, or at one it created.",
    .tables.join(", ")
  )]
  UnrecognizedSchema {
    /// The database file, as SQLite reports it.
    database: String,
    /// The schema version recorded in the file's `user_version`.
    found: i64,
    /// The tables and views found in the file, by name.
    tables: Vec<String>,
  },
  /// Another process holds the migration lock beside the database.
  #[error(
    "another rustmail process holds {} (storage migration or restore in progress); \
     wait for it to finish, its log shows progress, or stop it",
    .lock_path.display()
  )]
  MigrationLocked {
    /// The lock file, `<db>.migration-lock`.
    lock_path: PathBuf,
  },
  /// The files beside the database are in a state the migration cannot
  /// resolve on its own. Nothing was changed.
  #[error(transparent)]
  MigrationRefused(#[from] MigrationRefusal),
  /// The files beside the database are not in a state the backup can be
  /// restored from. Nothing was changed.
  #[error(transparent)]
  RestoreRefused(#[from] RestoreRefusal),
  /// The migrated copy failed a check against the legacy database. The copy
  /// is kept for a bug report and the legacy database is untouched.
  #[error(
    "the migrated copy {} failed its {check} check (expected {expected}, found {found}); \
     {} is untouched. Keep both files and report this as a bug.",
    .target.display(),
    .database.display()
  )]
  MigrationVerifyFailed {
    /// The legacy database being migrated.
    database: PathBuf,
    /// The copy being built, `<db>.migrating`.
    target: PathBuf,
    /// Which check failed.
    check: &'static str,
    /// What the legacy database implies.
    expected: String,
    /// What the copy holds.
    found: String,
  },
  /// The disk filled while the migrated copy was being written. The legacy
  /// database is untouched, and the copy resumes where it stopped.
  #[error(
    "migrating {} needs about {} MB free next to it (estimate); \
     the existing database is untouched; free space and restart to resume",
    .database.display(),
    .needed_bytes.div_ceil(BYTES_PER_MB)
  )]
  MigrationDiskFull {
    /// The legacy database being migrated.
    database: PathBuf,
    /// Estimated bytes the copy still needs.
    needed_bytes: u64,
  },
  /// A file operation of the migration failed.
  #[error("could not {action} {}: {source}", .path.display())]
  MigrationIo {
    /// What was being done, as a verb phrase.
    action: &'static str,
    /// The file it was done to.
    path: PathBuf,
    /// The underlying error.
    source: std::io::Error,
  },
  /// A blocking file operation of the migration did not complete.
  #[error("a file operation of the storage migration did not complete: {0}")]
  MigrationTaskAborted(tokio::task::JoinError),
}

/// Bytes in the megabyte the migration's free-space estimate is quoted in.
const BYTES_PER_MB: u64 = 1_000_000;

/// Why the migration refused to touch the files beside a database.
///
/// Each message names the files involved and how to resolve the state by
/// hand; the refusal itself changes nothing.
#[derive(Debug, thiserror::Error)]
pub enum MigrationRefusal {
  /// A legacy database sits next to a backup from an earlier migration, so
  /// an older rustmail probably created a new database after a partial swap.
  #[error(
    "{} is schema 0 but the backup {} already exists, so an older rustmail may have \
     written a new database after an interrupted migration (migration copy: {}). Keep the \
     file whose mail you want as {}, move the others away, and restart.",
    .database.display(),
    .backup.display(),
    .migrating.as_ref().map_or_else(|| "none".to_string(), |path| path.display().to_string()),
    .database.display()
  )]
  AmbiguousBackup {
    /// The database.
    database: PathBuf,
    /// The existing backup.
    backup: PathBuf,
    /// The migration copy, if one exists.
    migrating: Option<PathBuf>,
  },
  /// A migration copy sits next to a database that is already schema 1.
  #[error(
    "{} is already schema 1, but a stray migration copy {} sits next to it. \
     Move {} away, with any -wal or -shm beside it, and restart.",
    .database.display(),
    .migrating.display(),
    .migrating.display()
  )]
  StrayMigrationCopy {
    /// The database.
    database: PathBuf,
    /// The stray copy.
    migrating: PathBuf,
  },
  /// The finished copy was not made from the backup beside it, so the
  /// interrupted swap cannot be completed safely.
  #[error(
    "{} is missing and the finished migration copy {} was not made from the backup {}. \
     Rename the file you want to keep to {} and restart.",
    .database.display(),
    .migrating.display(),
    .backup.display(),
    .database.display()
  )]
  BackupChanged {
    /// The database.
    database: PathBuf,
    /// The backup.
    backup: PathBuf,
    /// The finished copy.
    migrating: PathBuf,
  },
  /// Another process still has the database or the copy open, so swapping
  /// them would leave its writes behind.
  #[error(
    "{} exists, so another process still has it open; stop every rustmail and sqlite3 \
     using {} and restart to finish the migration",
    .file.display(),
    .database.display()
  )]
  FileInUse {
    /// The database.
    database: PathBuf,
    /// The `-wal` or `-shm` file found.
    file: PathBuf,
  },
  /// The legacy database changed while the migration held its write lock,
  /// which only a writer bypassing SQLite's locks can do.
  #[error(
    "{} changed while the migration held its write lock; stop every process \
     writing to it and restart",
    .database.display()
  )]
  SourceUnstable {
    /// The database.
    database: PathBuf,
  },
  /// Any other combination of database, backup and migration copy.
  #[error(
    "cannot tell how to open {}: the database is {}, the backup {} is {}, and the \
     migration copy {} is {}. Put the file you want at {}, move the others away, and restart.",
    .database.display(),
    .database_state,
    .backup.display(),
    .backup_state,
    .migrating.display(),
    .migrating_state,
    .database.display()
  )]
  UnknownState {
    /// The database.
    database: PathBuf,
    /// What the database is: absent, empty, schema 0 or schema 1.
    database_state: &'static str,
    /// The backup.
    backup: PathBuf,
    /// Whether the backup exists.
    backup_state: &'static str,
    /// The migration copy.
    migrating: PathBuf,
    /// Whether the copy exists, and whether it is complete.
    migrating_state: &'static str,
  },
}

/// Why `restore-backup` refused to put the backup back in place.
///
/// Each message names the files involved and what to do; the refusal itself
/// renames nothing.
#[derive(Debug, thiserror::Error)]
pub enum RestoreRefusal {
  /// The database is not a migrated schema-1 file, so there is nothing to
  /// restore the backup over.
  #[error(
    "{} is {}, not a schema-1 database, so there is nothing to restore the backup over; \
     restore-backup only undoes a completed storage migration (was the backup already restored?)",
    .database.display(),
    .database_state
  )]
  NotMigrated {
    /// The database.
    database: PathBuf,
    /// What the database is: absent, empty or schema 0.
    database_state: &'static str,
  },
  /// No backup sits beside the database.
  #[error(
    "there is no backup {} to restore over {}; only a database migrated from rustmail 0.7 \
     or earlier has one, and it is gone once deleted or restored",
    .backup.display(),
    .database.display()
  )]
  NoBackup {
    /// The database.
    database: PathBuf,
    /// Where the backup would be, `<db>.schema0.bak`.
    backup: PathBuf,
  },
  /// The backup is not an intact schema-0 database.
  #[error(
    "{} is not an intact schema-0 rustmail database ({reason}), so it is not restored; \
     nothing was changed. Keep the file for inspection.",
    .backup.display()
  )]
  BackupInvalid {
    /// The backup.
    backup: PathBuf,
    /// What is wrong with it.
    reason: String,
  },
  /// A migration copy sits beside the database.
  #[error(
    "a migration copy {} sits next to {}; move it away, with any -wal or -shm beside it, \
     and run restore-backup again",
    .migrating.display(),
    .database.display()
  )]
  MigrationCopyPresent {
    /// The database.
    database: PathBuf,
    /// The copy.
    migrating: PathBuf,
  },
  /// A process still has the database or the backup open, or SQLite left a
  /// journal beside one that it would apply to the file renamed there.
  #[error(
    "{} exists, so another process still has it open; stop every rustmail and sqlite3 \
     using {} and run restore-backup again",
    .file.display(),
    .database.display()
  )]
  FileInUse {
    /// The database.
    database: PathBuf,
    /// The `-wal`, `-shm` or `-journal` file found.
    file: PathBuf,
  },
  /// The database's write-ahead log could not be checkpointed.
  #[error(
    "the write-ahead log of {} could not be checkpointed because another process is using \
     it; stop every rustmail and sqlite3 using it and run restore-backup again",
    .database.display()
  )]
  CheckpointBusy {
    /// The database.
    database: PathBuf,
  },
  /// The name the schema-1 database would be kept under is taken.
  #[error(
    "{} already exists; move it away, or wait a second, and run restore-backup again",
    .kept.display()
  )]
  KeptNameTaken {
    /// The name that is taken.
    kept: PathBuf,
  },
}

/// SQLite primary result code for `SQLITE_BUSY`.
pub(crate) const SQLITE_BUSY: i32 = 5;
/// SQLite primary result code for `SQLITE_LOCKED`.
pub(crate) const SQLITE_LOCKED: i32 = 6;
/// SQLite primary result code for `SQLITE_NOMEM`.
const SQLITE_NOMEM: i32 = 7;
/// SQLite primary result code for `SQLITE_READONLY`.
const SQLITE_READONLY: i32 = 8;
/// SQLite primary result code for `SQLITE_IOERR`.
const SQLITE_IOERR: i32 = 10;
/// SQLite primary result code for `SQLITE_CORRUPT`.
const SQLITE_CORRUPT: i32 = 11;
/// SQLite primary result code for `SQLITE_FULL`.
pub(crate) const SQLITE_FULL: i32 = 13;
/// SQLite primary result code for `SQLITE_CANTOPEN`.
const SQLITE_CANTOPEN: i32 = 14;
/// SQLite primary result code for `SQLITE_NOTADB`.
const SQLITE_NOTADB: i32 = 26;
/// Low byte of a SQLite result code, which carries the primary code.
const PRIMARY_CODE_MASK: i32 = 0xFF;

impl StorageError {
  /// The SQLite primary result code this error carries, if SQLite reported it.
  pub(crate) fn sqlite_primary_code(&self) -> Option<i32> {
    let StorageError::Database(sqlx::Error::Database(db_error)) = self else {
      return None;
    };
    db_error
      .code()
      .and_then(|code| code.parse::<i32>().ok())
      .map(|code| code & PRIMARY_CODE_MASK)
  }

  /// Whether the failure concerns the store as a whole rather than the
  /// message being written.
  ///
  /// A locked, full, unreadable or unreachable database refuses every write
  /// alike, so writing the same messages again one at a time only fails
  /// again, each attempt as slowly as the first. Anything else, a constraint
  /// or a trigger refusing a row among them, may be specific to one message.
  pub fn is_store_wide(&self) -> bool {
    match self {
      StorageError::Database(
        sqlx::Error::Configuration(_)
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed,
      ) => true,
      _ => self.sqlite_primary_code().is_some_and(|code| {
        matches!(
          code,
          SQLITE_BUSY
            | SQLITE_LOCKED
            | SQLITE_NOMEM
            | SQLITE_READONLY
            | SQLITE_IOERR
            | SQLITE_CORRUPT
            | SQLITE_FULL
            | SQLITE_CANTOPEN
            | SQLITE_NOTADB
        )
      }),
    }
  }
}
