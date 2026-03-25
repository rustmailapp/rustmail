/// Errors returned by the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
  /// An underlying SQLite/sqlx error occurred.
  #[error("Database error: {0}")]
  Database(#[from] sqlx::Error),
  /// The requested message or attachment was not found.
  #[error("Message not found: {0}")]
  NotFound(String),
}
