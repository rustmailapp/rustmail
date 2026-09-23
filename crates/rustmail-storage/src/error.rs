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
     this binary supports schema {supported}. Upgrade rustmail."
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
  /// written to it.
  #[error(
    "{database} is schema 0, written by rustmail 0.7 or earlier; \
     this binary supports schema {supported} and cannot upgrade it yet. \
     Open it with the rustmail that wrote it, or start this one on a new database file."
  )]
  LegacySchema {
    /// The database file, as SQLite reports it.
    database: String,
    /// The schema version this binary supports.
    supported: i64,
  },
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
const SQLITE_FULL: i32 = 13;
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
