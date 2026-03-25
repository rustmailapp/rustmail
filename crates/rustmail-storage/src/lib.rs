//! SQLite-backed storage layer for captured emails.
//!
//! This crate provides persistent (or in-memory) storage for email messages
//! and attachments using [`sqlx`] with SQLite. Features include:
//!
//! - **FTS5 full-text search** across subject, body, sender, and recipients
//! - **ULID-based IDs** for time-sortable, globally unique identifiers
//! - **WAL mode** for concurrent read/write access
//! - **Retention policies** via time-based deletion and max-message trimming
//!
//! # Example
//!
//! ```no_run
//! use rustmail_storage::{MessageRepository, initialize_database};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = sqlx::sqlite::SqlitePoolOptions::new()
//!     .connect("sqlite::memory:")
//!     .await?;
//! initialize_database(&pool).await?;
//!
//! let repo = MessageRepository::new(pool);
//! let messages = repo.list(50, 0).await?;
//! # Ok(())
//! # }
//! ```

mod error;
mod models;
mod repo;
mod schema;

pub use error::StorageError;
pub use models::{Attachment, AttachmentSummary, Message, MessageSummary};
pub use repo::{MessageRepository, format_iso8601};
pub use schema::initialize_database;
