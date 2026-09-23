use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::debug;
use ulid::Ulid;

use crate::error::{SQLITE_BUSY, SQLITE_LOCKED, StorageError};
use crate::models::{Attachment, AttachmentSummary, Message, MessageSummary};
use crate::prepared::PreparedMessage;
use crate::query::{Cursor, MessageFilter, PageStart, push_filter, push_page};
use crate::schema::BUSY_TIMEOUT;

const ISO8601_FMT: &[time::format_description::BorrowedFormatItem<'_>] =
  format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// Total attempts a contended write gets, the first one included.
const WRITE_ATTEMPTS: u32 = 5;
/// Delay before the second attempt; doubles from there.
const WRITE_RETRY_BASE_DELAY: Duration = Duration::from_millis(20);
/// Ceiling on a whole retry sequence.
///
/// Tied to [`BUSY_TIMEOUT`] because SQLite's own busy handler already waits
/// that long inside a single attempt before reporting contention. Counting
/// attempts alone would let five of those stack up, and `delete_all` is
/// reachable from an HTTP handler, so a request could hang for a multiple of
/// what one attempt already costs.
const WRITE_RETRY_BUDGET: Duration = BUSY_TIMEOUT;

/// Whether a failed write is worth attempting again.
///
/// SQLite serialises writers, so a busy or locked database is a transient
/// state rather than a rejection. `busy_timeout` covers most of it, but not
/// the cases where SQLite refuses to wait — promoting a transaction that would
/// deadlock, for one — so the caller still has to be prepared to retry.
fn is_retryable_lock(error: &StorageError) -> bool {
  error
    .sqlite_primary_code()
    .is_some_and(|code| matches!(code, SQLITE_BUSY | SQLITE_LOCKED))
}

/// Spread for a retry delay, so racing writers do not wake together.
///
/// The clock's sub-second noise is uncorrelated enough between two processes
/// to serve here, which keeps this off an RNG dependency.
fn jitter(bound: Duration) -> Duration {
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |since| u64::from(since.subsec_nanos()));
  let bound_nanos = u64::try_from(bound.as_nanos()).unwrap_or(u64::MAX).max(1);
  Duration::from_nanos(nanos % bound_nanos)
}

/// Runs `op`, retrying while the database reports the write as contended.
///
/// Bounded twice over: by attempt count and by a wall-clock budget, so an
/// attempt that spends its whole `busy_timeout` inside SQLite cannot stack
/// with four more. Backs off exponentially with jitter. Retrying is
/// safe because a failed write leaves nothing behind: the transaction rolls
/// back, and an insert mints a fresh id per attempt, so no attempt can
/// duplicate a row committed by an earlier one.
async fn retry_on_lock<T, F, Fut>(mut op: F) -> Result<T, StorageError>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<T, StorageError>>,
{
  let deadline = Instant::now() + WRITE_RETRY_BUDGET;
  let mut attempt = 0;
  loop {
    let result = op().await;
    match &result {
      Err(error)
        if is_retryable_lock(error)
          && attempt + 1 < WRITE_ATTEMPTS
          && Instant::now() < deadline =>
      {
        let backoff = WRITE_RETRY_BASE_DELAY.saturating_mul(2_u32.saturating_pow(attempt));
        debug!(
          attempt = attempt + 1,
          "write contended, retrying after backoff"
        );
        tokio::time::sleep(backoff + jitter(backoff)).await;
        attempt += 1;
      }
      _ => return result,
    }
  }
}

/// Repository for storing and querying captured email messages.
///
/// Wraps a [`SqlitePool`] for reads and one for writes, which may be the same
/// pool, and provides async methods for CRUD operations, full-text search,
/// retention enforcement, and attachment access.
#[derive(Clone)]
pub struct MessageRepository {
  readers: SqlitePool,
  writer: SqlitePool,
}

impl MessageRepository {
  /// Creates a new repository that reads and writes through `pool`.
  pub fn new(pool: SqlitePool) -> Self {
    Self {
      readers: pool.clone(),
      writer: pool,
    }
  }

  /// Creates a repository that reads through `readers` and writes only
  /// through `writer`.
  ///
  /// `writer` is meant to hold a single connection. SQLite admits one writer
  /// at a time anyway, and a connection that is the only one writing keeps its
  /// page cache and memory map valid from one transaction to the next, where
  /// writes rotating across a pool make each connection discard both whenever
  /// another one wrote since it last ran.
  pub fn with_writer(readers: SqlitePool, writer: SqlitePool) -> Self {
    Self { readers, writer }
  }

  /// Closes the readers, then the writer.
  ///
  /// The last connection to close checkpoints the WAL, and closing the writer
  /// last leaves that to the connection whose cache already holds the pages.
  pub async fn close(&self) {
    self.readers.close().await;
    self.writer.close().await;
  }

  /// Parses and stores a raw email, extracting metadata and attachments.
  ///
  /// Inserts the message into the `messages` table, populates the FTS5 index,
  /// and stores any MIME attachments in the `attachments` table.
  ///
  /// # Errors
  ///
  /// Returns [`StorageError::Database`] if any insert fails.
  pub async fn insert(
    &self,
    sender: &str,
    recipients: &[String],
    raw: &[u8],
  ) -> Result<MessageSummary, StorageError> {
    let message = PreparedMessage::parse(sender.to_string(), recipients, raw.to_vec());
    self.insert_prepared(&message).await
  }

  /// Stores a message parsed ahead of time by [`PreparedMessage::parse`].
  ///
  /// A contended write is retried without parsing the message again.
  ///
  /// # Errors
  ///
  /// Returns [`StorageError::Database`] if any insert fails.
  pub async fn insert_prepared(
    &self,
    message: &PreparedMessage,
  ) -> Result<MessageSummary, StorageError> {
    retry_on_lock(|| self.insert_prepared_once(message)).await
  }

  async fn insert_prepared_once(
    &self,
    message: &PreparedMessage,
  ) -> Result<MessageSummary, StorageError> {
    let mut txn = self.writer.begin().await?;
    let summary = insert_in(&mut txn, message).await?;
    txn.commit().await?;
    debug!(id = %summary.id, subject = ?summary.subject, "Message stored");
    Ok(summary)
  }

  /// Stores every message in `messages` in one transaction: all of them or
  /// none.
  ///
  /// The commit, with its WAL frames and FTS5 segment flush, dominates the
  /// cost of storing small mail, so committing a batch at once amortises it.
  /// Summaries come back in input order, which is also the arrival order
  /// [`Self::list`] sorts by.
  ///
  /// # Errors
  ///
  /// Returns [`StorageError::Database`] if any insert fails, in which case
  /// no message from the batch is stored.
  pub async fn insert_batch(
    &self,
    messages: &[PreparedMessage],
  ) -> Result<Vec<MessageSummary>, StorageError> {
    retry_on_lock(|| self.insert_batch_once(messages)).await
  }

  async fn insert_batch_once(
    &self,
    messages: &[PreparedMessage],
  ) -> Result<Vec<MessageSummary>, StorageError> {
    let mut txn = self.writer.begin().await?;
    let mut summaries = Vec::with_capacity(messages.len());
    for message in messages {
      summaries.push(insert_in(&mut txn, message).await?);
    }
    txn.commit().await?;
    debug!(count = summaries.len(), "Message batch stored");
    Ok(summaries)
  }

  /// Lists messages ordered by newest first, with pagination.
  ///
  /// Ordering is by `rowid`, which is arrival order. ULIDs sort the same way
  /// only down to the millisecond: mail captured within the same millisecond
  /// is ordered by the ULID's random bits, so `ORDER BY id` shuffles bursts.
  /// [`Self::search`] orders the same way, so browsing and searching agree.
  pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<MessageSummary>, StorageError> {
    self
      .list_page(&MessageFilter::default(), PageStart::Offset(offset), limit)
      .await
  }

  /// Lists up to `limit` messages passing `filter`, newest first, starting
  /// at `start`.
  ///
  /// Orders like [`Self::list`]; a [`PageStart::Before`] page costs the same
  /// at any depth.
  pub async fn list_page(
    &self,
    filter: &MessageFilter,
    start: PageStart,
    limit: i64,
  ) -> Result<Vec<MessageSummary>, StorageError> {
    let mut builder = list_statement(filter, start, limit);
    let messages = builder
      .build_query_as::<MessageSummary>()
      .fetch_all(&self.readers)
      .await?;

    Ok(messages)
  }

  /// Full-text search across subject, body, sender, and recipients via FTS5.
  pub async fn search(
    &self,
    query: &str,
    limit: i64,
    offset: i64,
  ) -> Result<Vec<MessageSummary>, StorageError> {
    self
      .search_page(
        query,
        &MessageFilter::default(),
        PageStart::Offset(offset),
        limit,
      )
      .await
  }

  /// Searches like [`Self::search`], returning up to `limit` matches that
  /// also pass `filter`, newest first, starting at `start`.
  ///
  /// The FTS table drives the join and the ordering is `fts.rowid DESC`, which
  /// FTS5 can satisfy natively. Ordering by a `messages` column instead forces
  /// SQLite to materialise and sort every match before applying `LIMIT`, so a
  /// query matching a large mailbox pays for the whole result set to return
  /// one page of it. The cursor bound goes on `fts.rowid` for the same reason.
  pub async fn search_page(
    &self,
    query: &str,
    filter: &MessageFilter,
    start: PageStart,
    limit: i64,
  ) -> Result<Vec<MessageSummary>, StorageError> {
    let quoted = match Self::sanitize_fts_query(query) {
      Some(q) => q,
      None => return Ok(Vec::new()),
    };
    let mut builder = search_statement(quoted, filter, start, limit);
    let messages = builder
      .build_query_as::<MessageSummary>()
      .fetch_all(&self.readers)
      .await?;

    Ok(messages)
  }

  /// Resolves a message id to its position in listing order.
  ///
  /// # Errors
  ///
  /// Returns [`StorageError::NotFound`] if no stored message has `id`.
  pub async fn cursor(&self, id: &str) -> Result<Cursor, StorageError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT seq FROM messages WHERE id = ?1")
      .bind(id)
      .fetch_optional(&self.readers)
      .await?;
    row
      .map(|(rowid,)| Cursor(rowid))
      .ok_or_else(|| StorageError::NotFound(id.to_string()))
  }

  /// Counts the total number of FTS5 search matches.
  ///
  /// Counted on the index alone. `messages_fts` is an external-content table
  /// over `messages` and `message_content`, so every indexed rowid has exactly
  /// one source row and joining back cannot change the count: it only spends
  /// a primary-key lookup per match to fetch a row the count then discards.
  pub async fn search_count(&self, query: &str) -> Result<i64, StorageError> {
    let quoted = match Self::sanitize_fts_query(query) {
      Some(q) => q,
      None => return Ok(0),
    };
    let row: (i64,) = sqlx::query_as(
      r#"
      SELECT COUNT(*)
      FROM messages_fts
      WHERE messages_fts MATCH ?1
      "#,
    )
    .bind(&quoted)
    .fetch_one(&self.readers)
    .await?;
    Ok(row.0)
  }

  /// Counts the FTS5 search matches that also pass `filter`.
  ///
  /// Without a filter this is [`Self::search_count`], counted on the index
  /// alone; a filter needs each match's row, so it joins back.
  pub async fn search_count_filtered(
    &self,
    query: &str,
    filter: &MessageFilter,
  ) -> Result<i64, StorageError> {
    if filter.is_empty() {
      return self.search_count(query).await;
    }
    let quoted = match Self::sanitize_fts_query(query) {
      Some(q) => q,
      None => return Ok(0),
    };
    let mut builder = search_count_statement(quoted, filter);
    let row: (i64,) = builder.build_query_as().fetch_one(&self.readers).await?;
    Ok(row.0)
  }

  fn sanitize_fts_query(query: &str) -> Option<String> {
    let sanitized: String = query
      .chars()
      .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '@' | '.' | '-' | '+' | '_'))
      .collect();
    if sanitized.trim().is_empty() {
      return None;
    }
    Some(format!("\"{}\"", sanitized))
  }

  /// Fetches a single message by ID, including its parsed bodies.
  ///
  /// The raw RFC 5322 bytes are not read; use [`Self::get_raw`] for those.
  /// They follow the bodies in `message_content`, so SQLite stops reading the
  /// row before it reaches them.
  pub async fn get(&self, id: &str) -> Result<Message, StorageError> {
    let message = sqlx::query_as::<_, Message>(
      "SELECT m.id, m.sender, m.recipients, m.subject, c.text_body, c.html_body, m.size, m.has_attachments, m.is_read, m.is_starred, m.tags, m.created_at FROM messages m JOIN message_content c ON c.seq = m.seq WHERE m.id = ?1",
    )
    .bind(id)
    .fetch_optional(&self.readers)
    .await?
    .ok_or_else(|| StorageError::NotFound(id.to_string()))?;

    Ok(message)
  }

  /// Atomically applies one or more metadata updates to a message.
  ///
  /// Only fields that are `Some` are updated, in a single `UPDATE` so partial
  /// application cannot occur.
  pub async fn update_message(
    &self,
    id: &str,
    is_read: Option<bool>,
    is_starred: Option<bool>,
    tags: Option<&[String]>,
  ) -> Result<(), StorageError> {
    let tags_json = tags.map(|tags| serde_json::to_string(tags).unwrap_or_default());
    let result = retry_on_lock(|| async {
      let result = sqlx::query(
        "UPDATE messages SET is_read = COALESCE(?1, is_read), is_starred = COALESCE(?2, is_starred), tags = COALESCE(?3, tags) WHERE id = ?4",
      )
      .bind(is_read)
      .bind(is_starred)
      .bind(tags_json.as_deref())
      .bind(id)
      .execute(&self.writer)
      .await?;
      Ok(result)
    })
    .await?;

    if result.rows_affected() == 0 {
      return Err(StorageError::NotFound(id.to_string()));
    }
    Ok(())
  }

  /// Deletes a single message, its FTS5 index entry, its content and its
  /// attachments atomically.
  ///
  /// The index entry goes first: FTS5 reads the row back through
  /// `messages_fts_source` to find the tokens to remove. Content and
  /// attachments follow the `messages` row by cascade.
  pub async fn delete(&self, id: &str) -> Result<(), StorageError> {
    let mut txn = self.writer.begin().await?;

    sqlx::query("DELETE FROM messages_fts WHERE rowid = (SELECT seq FROM messages WHERE id = ?1)")
      .bind(id)
      .execute(&mut *txn)
      .await?;

    let result = sqlx::query("DELETE FROM messages WHERE id = ?1")
      .bind(id)
      .execute(&mut *txn)
      .await?;

    if result.rows_affected() == 0 {
      return Err(StorageError::NotFound(id.to_string()));
    }

    txn.commit().await?;
    Ok(())
  }

  /// Deletes all messages and clears the FTS5 index atomically. Returns the count of deleted messages.
  ///
  /// Uses FTS5's `delete-all` command rather than `DELETE FROM messages_fts`.
  /// An external-content index reads the content row to work out which tokens
  /// to remove, so a plain `DELETE` issued after the source rows are gone is a
  /// silent no-op that leaves the whole index behind. Content and attachments
  /// follow the `messages` rows by cascade.
  pub async fn delete_all(&self) -> Result<u64, StorageError> {
    retry_on_lock(|| self.delete_all_once()).await
  }

  async fn delete_all_once(&self) -> Result<u64, StorageError> {
    let mut txn = self.writer.begin().await?;

    sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('delete-all')")
      .execute(&mut *txn)
      .await?;

    let result = sqlx::query("DELETE FROM messages")
      .execute(&mut *txn)
      .await?;

    txn.commit().await?;
    Ok(result.rows_affected())
  }

  /// Returns the total number of stored messages.
  pub async fn count(&self) -> Result<i64, StorageError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
      .fetch_one(&self.readers)
      .await?;
    Ok(row.0)
  }

  /// Counts the stored messages that pass `filter`.
  pub async fn count_filtered(&self, filter: &MessageFilter) -> Result<i64, StorageError> {
    if filter.is_empty() {
      return self.count().await;
    }
    let mut builder = count_statement(filter);
    let row: (i64,) = builder.build_query_as().fetch_one(&self.readers).await?;
    Ok(row.0)
  }

  /// Counts messages matching optional subject, sender, and recipient filters (case-insensitive).
  pub async fn count_matching(
    &self,
    subject: Option<&str>,
    sender: Option<&str>,
    recipient: Option<&str>,
  ) -> Result<i64, StorageError> {
    let (sql, binds) = count_matching_statement(subject, sender, recipient);
    let mut query = sqlx::query_as::<_, (i64,)>(&sql);
    for b in &binds {
      query = query.bind(b);
    }

    let row = query.fetch_one(&self.readers).await?;
    Ok(row.0)
  }

  /// Lists all attachments for a given message (metadata only, no binary content),
  /// in the order the message carries them.
  pub async fn get_attachments(
    &self,
    message_id: &str,
  ) -> Result<Vec<AttachmentSummary>, StorageError> {
    let attachments = sqlx::query_as::<_, AttachmentSummary>(
      "SELECT a.id, m.id AS message_id, a.filename, a.content_type, a.content_id, a.size FROM messages m JOIN attachments a ON a.message_seq = m.seq WHERE m.id = ?1 ORDER BY a.rowid",
    )
    .bind(message_id)
    .fetch_all(&self.readers)
    .await?;

    Ok(attachments)
  }

  /// Fetches a single attachment by ID, scoped to its parent message.
  pub async fn get_attachment(
    &self,
    message_id: &str,
    attachment_id: &str,
  ) -> Result<Attachment, StorageError> {
    let attachment = sqlx::query_as::<_, Attachment>(
      "SELECT a.id, m.id AS message_id, a.filename, a.content_type, a.content_id, a.size, a.content FROM attachments a JOIN messages m ON m.seq = a.message_seq WHERE a.id = ?1 AND m.id = ?2",
    )
    .bind(attachment_id)
    .bind(message_id)
    .fetch_optional(&self.readers)
    .await?
    .ok_or_else(|| StorageError::NotFound(attachment_id.to_string()))?;

    Ok(attachment)
  }

  /// Fetches a single attachment by Content-ID, scoped to its parent message.
  ///
  /// When several parts share the Content-ID, the first one the message
  /// carries is returned.
  pub async fn get_attachment_by_content_id(
    &self,
    message_id: &str,
    content_id: &str,
  ) -> Result<Attachment, StorageError> {
    let attachment = sqlx::query_as::<_, Attachment>(
      "SELECT a.id, m.id AS message_id, a.filename, a.content_type, a.content_id, a.size, a.content FROM messages m JOIN attachments a ON a.message_seq = m.seq WHERE a.content_id = ?1 AND m.id = ?2 ORDER BY a.rowid LIMIT 1",
    )
    .bind(content_id)
    .bind(message_id)
    .fetch_optional(&self.readers)
    .await?
    .ok_or_else(|| StorageError::NotFound(content_id.to_string()))?;

    Ok(attachment)
  }

  /// Returns the raw RFC 5322 bytes for a message.
  pub async fn get_raw(&self, id: &str) -> Result<Vec<u8>, StorageError> {
    let row: (Vec<u8>,) = sqlx::query_as(
      "SELECT c.raw FROM messages m JOIN message_content c ON c.seq = m.seq WHERE m.id = ?1",
    )
    .bind(id)
    .fetch_optional(&self.readers)
    .await?
    .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    Ok(row.0)
  }

  /// Returns at most `max_bytes` from the start of a message's raw bytes.
  ///
  /// Callers that only need the header section, or only enough source to fill
  /// a preview, should use this rather than [`Self::get_raw`]: `substr` saves
  /// copying the rest of the blob out of SQLite and sending it to the caller.
  /// It does not save the read itself: SQLite still loads the whole blob to
  /// take a prefix of it, so this costs about as much disk and cache as
  /// [`Self::get_raw`].
  ///
  /// # Errors
  ///
  /// Returns [`StorageError::NotFound`] if no message has that id.
  pub async fn get_raw_prefix(&self, id: &str, max_bytes: i64) -> Result<Vec<u8>, StorageError> {
    let row: (Vec<u8>,) = sqlx::query_as(
      "SELECT substr(c.raw, 1, ?2) FROM messages m JOIN message_content c ON c.seq = m.seq WHERE m.id = ?1",
    )
    .bind(id)
    .bind(max_bytes)
    .fetch_optional(&self.readers)
    .await?
    .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    Ok(row.0)
  }

  /// Deletes messages older than the given ISO 8601 cutoff. Returns IDs of deleted messages.
  ///
  /// A read-only `EXISTS` check outside the write transaction skips the
  /// write lock entirely on a no-op retention tick. When there is a match,
  /// the deleting transaction opens with a write so it takes the write lock
  /// before it holds a snapshot: a read first would fail outright with
  /// `SQLITE_BUSY_SNAPSHOT` whenever an insert commits in between.
  pub async fn delete_older_than(&self, iso_cutoff: &str) -> Result<Vec<String>, StorageError> {
    retry_on_lock(|| self.delete_older_than_once(iso_cutoff)).await
  }

  async fn delete_older_than_once(&self, iso_cutoff: &str) -> Result<Vec<String>, StorageError> {
    let has_match: bool =
      sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE created_at < ?1)")
        .bind(iso_cutoff)
        .fetch_one(&self.readers)
        .await?;
    if !has_match {
      return Ok(Vec::new());
    }

    let mut txn = self.writer.begin().await?;

    sqlx::query(
      "DELETE FROM messages_fts WHERE rowid IN (SELECT seq FROM messages WHERE created_at < ?1)",
    )
    .bind(iso_cutoff)
    .execute(&mut *txn)
    .await?;

    let ids: Vec<(String,)> =
      sqlx::query_as("DELETE FROM messages WHERE created_at < ?1 RETURNING id")
        .bind(iso_cutoff)
        .fetch_all(&mut *txn)
        .await?;

    txn.commit().await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
  }

  /// Trims stored messages to at most `max`, deleting oldest first. Returns IDs of deleted messages.
  ///
  /// A read-only count outside the write transaction skips the write lock
  /// entirely when the store is already at or under `max`. Ordered by
  /// `seq` to match [`Self::list`], so the rows dropped here are exactly
  /// the ones the UI shows as oldest. The newest doomed `seq` is found once
  /// and both deletes run by range below it, instead of repeating the same
  /// offset scan per statement. The transaction starts `IMMEDIATE` because
  /// that lookup is a read: taking the write lock up front keeps an insert
  /// from committing between it and the deletes.
  pub async fn trim_to_max(&self, max: i64) -> Result<Vec<String>, StorageError> {
    retry_on_lock(|| self.trim_to_max_once(max)).await
  }

  async fn trim_to_max_once(&self, max: i64) -> Result<Vec<String>, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
      .fetch_one(&self.readers)
      .await?;
    if count <= max {
      return Ok(Vec::new());
    }

    let mut txn = self.writer.begin_with("BEGIN IMMEDIATE").await?;

    let threshold: Option<(i64,)> =
      sqlx::query_as("SELECT seq FROM messages ORDER BY seq DESC LIMIT 1 OFFSET ?1")
        .bind(max)
        .fetch_optional(&mut *txn)
        .await?;
    let Some((threshold,)) = threshold else {
      return Ok(Vec::new());
    };

    sqlx::query("DELETE FROM messages_fts WHERE rowid <= ?1")
      .bind(threshold)
      .execute(&mut *txn)
      .await?;

    let ids: Vec<(String,)> = sqlx::query_as("DELETE FROM messages WHERE seq <= ?1 RETURNING id")
      .bind(threshold)
      .fetch_all(&mut *txn)
      .await?;

    txn.commit().await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
  }
}

/// Writes `message` and its content, index and attachment rows on `conn`.
///
/// Mints a fresh id on every call, so a retried or re-batched write can never
/// collide with a row an earlier attempt committed.
async fn insert_in(
  conn: &mut SqliteConnection,
  message: &PreparedMessage,
) -> Result<MessageSummary, StorageError> {
  let id = Ulid::new().to_string();
  let size = message.raw.len() as i64;
  let now = OffsetDateTime::now_utc()
    .format(ISO8601_FMT)
    .unwrap_or_default();

  let seq = sqlx::query(
    r#"
    INSERT INTO messages (id, sender, recipients, subject, size, has_attachments, is_read, is_starred, tags, created_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, '[]', ?7)
    "#,
  )
  .bind(&id)
  .bind(&message.sender)
  .bind(&message.recipients_json)
  .bind(&message.subject)
  .bind(size)
  .bind(message.has_attachments)
  .bind(&now)
  .execute(&mut *conn)
  .await?
  .last_insert_rowid();

  sqlx::query(
    "INSERT INTO message_content (seq, text_body, html_body, raw) VALUES (?1, ?2, ?3, ?4)",
  )
  .bind(seq)
  .bind(&message.text_body)
  .bind(&message.html_body)
  .bind(&message.raw)
  .execute(&mut *conn)
  .await?;

  sqlx::query(
    "INSERT INTO messages_fts(rowid, subject, text_body, sender, recipients) VALUES (?1, ?2, ?3, ?4, ?5)",
  )
  .bind(seq)
  .bind(&message.subject)
  .bind(&message.text_body)
  .bind(&message.sender)
  .bind(&message.recipients_json)
  .execute(&mut *conn)
  .await?;

  for attachment in &message.attachments {
    sqlx::query(
      r#"
      INSERT INTO attachments (id, message_seq, filename, content_type, content_id, size, content)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
      "#,
    )
    .bind(Ulid::new().to_string())
    .bind(seq)
    .bind(&attachment.filename)
    .bind(&attachment.content_type)
    .bind(&attachment.content_id)
    .bind(attachment.content.len() as i64)
    .bind(&attachment.content)
    .execute(&mut *conn)
    .await?;
  }

  Ok(MessageSummary {
    id,
    sender: message.sender.clone(),
    recipients: message.recipients_json.clone(),
    subject: message.subject.clone(),
    size,
    has_attachments: message.has_attachments,
    is_read: false,
    is_starred: false,
    tags: "[]".to_string(),
    created_at: now,
  })
}

/// Columns of a [`MessageSummary`], read from `messages` aliased `m`.
const SUMMARY_COLUMNS: &str = "m.id, m.sender, m.recipients, m.subject, m.size, m.has_attachments, m.is_read, m.is_starred, m.tags, m.created_at";

/// The listing behind [`MessageRepository::list_page`]: `messages` alone,
/// newest first.
fn list_statement(
  filter: &MessageFilter,
  start: PageStart,
  limit: i64,
) -> QueryBuilder<'static, Sqlite> {
  let mut builder = QueryBuilder::<Sqlite>::new(format!(
    "SELECT {SUMMARY_COLUMNS} FROM messages m WHERE 1=1"
  ));
  push_filter(&mut builder, "m", filter);
  push_page(&mut builder, "m.seq", start, limit);
  builder
}

/// The search behind [`MessageRepository::search_page`]: the FTS index
/// joined to `messages` by `seq`, ordered on the index's rowid.
fn search_statement(
  quoted: String,
  filter: &MessageFilter,
  start: PageStart,
  limit: i64,
) -> QueryBuilder<'static, Sqlite> {
  let mut builder = QueryBuilder::<Sqlite>::new(format!(
    "SELECT {SUMMARY_COLUMNS} FROM messages_fts fts INNER JOIN messages m ON m.seq = fts.rowid WHERE messages_fts MATCH "
  ));
  builder.push_bind(quoted);
  push_filter(&mut builder, "m", filter);
  push_page(&mut builder, "fts.rowid", start, limit);
  builder
}

/// The count behind [`MessageRepository::search_count_filtered`] when a
/// filter is set.
fn search_count_statement(quoted: String, filter: &MessageFilter) -> QueryBuilder<'static, Sqlite> {
  let mut builder = QueryBuilder::<Sqlite>::new(
    "SELECT COUNT(*) FROM messages_fts fts INNER JOIN messages m ON m.seq = fts.rowid WHERE messages_fts MATCH ",
  );
  builder.push_bind(quoted);
  push_filter(&mut builder, "m", filter);
  builder
}

/// The count behind [`MessageRepository::count_filtered`] when a filter is
/// set.
fn count_statement(filter: &MessageFilter) -> QueryBuilder<'static, Sqlite> {
  let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM messages m WHERE 1=1");
  push_filter(&mut builder, "m", filter);
  builder
}

/// The SQL and its binds behind [`MessageRepository::count_matching`].
fn count_matching_statement(
  subject: Option<&str>,
  sender: Option<&str>,
  recipient: Option<&str>,
) -> (String, Vec<String>) {
  let mut sql = String::from("SELECT COUNT(*) FROM messages WHERE 1=1");
  let mut binds: Vec<String> = Vec::new();
  let conditions = [
    ("subject", subject),
    ("sender", sender),
    ("recipients", recipient),
  ];
  for (column, needle) in conditions {
    if let Some(needle) = needle {
      sql.push_str(&format!(" AND LOWER({column}) LIKE ? ESCAPE '\\'"));
      binds.push(format!("%{}%", escape_like(&needle.to_lowercase())));
    }
  }
  (sql, binds)
}

fn escape_like(s: &str) -> String {
  s.replace('\\', "\\\\")
    .replace('%', "\\%")
    .replace('_', "\\_")
}

/// Formats an [`OffsetDateTime`] as an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn format_iso8601(dt: OffsetDateTime) -> String {
  dt.format(ISO8601_FMT).unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{connect_options, initialize_database};

  async fn test_repo() -> MessageRepository {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .connect("sqlite::memory:")
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    MessageRepository::new(pool)
  }

  fn raw_email(subject: &str, from: &str, to: &str) -> Vec<u8> {
    format!(
      "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\nHello world"
    )
    .into_bytes()
  }

  fn multipart_email(subject: &str) -> Vec<u8> {
    format!(
      concat!(
        "From: sender@test.com\r\n",
        "To: rcpt@test.com\r\n",
        "Subject: {}\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"BOUNDARY\"\r\n",
        "\r\n",
        "--BOUNDARY\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Body text\r\n",
        "--BOUNDARY\r\n",
        "Content-Type: application/pdf\r\n",
        "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
        "\r\n",
        "fake-pdf-content\r\n",
        "--BOUNDARY--\r\n",
      ),
      subject
    )
    .into_bytes()
  }

  #[tokio::test]
  async fn insert_and_get() {
    let repo = test_repo().await;
    let raw = raw_email("Test Subject", "alice@test.com", "bob@test.com");

    let summary = repo
      .insert("alice@test.com", &["bob@test.com".into()], &raw)
      .await
      .unwrap();

    assert_eq!(summary.sender, "alice@test.com");
    assert_eq!(summary.subject.as_deref(), Some("Test Subject"));
    assert!(!summary.is_read);
    assert!(!summary.is_starred);

    let msg = repo.get(&summary.id).await.unwrap();
    assert_eq!(msg.id, summary.id);
    assert_eq!(msg.text_body.as_deref(), Some("Hello world"));
    assert_eq!(repo.get_raw(&summary.id).await.unwrap(), raw);
  }

  #[tokio::test]
  async fn a_prepared_message_stores_its_attachments() {
    let repo = test_repo().await;
    let prepared = PreparedMessage::parse(
      "sender@test.com".to_string(),
      &["rcpt@test.com".into()],
      multipart_email("Prepared"),
    );

    let summary = repo.insert_prepared(&prepared).await.unwrap();

    assert!(summary.has_attachments);
    let attachments = repo.get_attachments(&summary.id).await.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
    assert_eq!(repo.search("Prepared", 10, 0).await.unwrap().len(), 1);
  }

  #[tokio::test]
  async fn storing_one_prepared_message_twice_keeps_both_rows() {
    let repo = test_repo().await;
    let prepared = PreparedMessage::parse(
      "a@test.com".to_string(),
      &["b@test.com".into()],
      raw_email("Twice", "a@test.com", "b@test.com"),
    );

    let first = repo.insert_prepared(&prepared).await.unwrap();
    let second = repo.insert_prepared(&prepared).await.unwrap();

    assert_ne!(first.id, second.id, "every attempt mints its own id");
    assert_eq!(repo.count().await.unwrap(), 2);
  }

  fn prepared(subject: &str) -> PreparedMessage {
    PreparedMessage::parse(
      "a@test.com".to_string(),
      &["b@test.com".into()],
      raw_email(subject, "a@test.com", "b@test.com"),
    )
  }

  async fn poison_subject(repo: &MessageRepository, subject: &str) {
    sqlx::query(&format!(
      "CREATE TRIGGER poison BEFORE INSERT ON messages WHEN NEW.subject = '{subject}' BEGIN SELECT RAISE(ABORT, 'poisoned'); END"
    ))
    .execute(&repo.writer)
    .await
    .unwrap();
  }

  #[tokio::test]
  async fn a_batch_is_stored_in_arrival_order() {
    let repo = test_repo().await;
    let batch: Vec<PreparedMessage> = ["one", "two", "three"].map(prepared).into();

    let summaries = repo.insert_batch(&batch).await.unwrap();

    let subjects: Vec<_> = summaries.iter().map(|s| s.subject.as_deref()).collect();
    assert_eq!(subjects, [Some("one"), Some("two"), Some("three")]);
    let newest_first: Vec<String> = repo
      .list(10, 0)
      .await
      .unwrap()
      .into_iter()
      .map(|s| s.id)
      .collect();
    let inserted: Vec<String> = summaries.into_iter().rev().map(|s| s.id).collect();
    assert_eq!(newest_first, inserted);
    assert_eq!(repo.search("two", 10, 0).await.unwrap().len(), 1);
  }

  #[tokio::test]
  async fn a_failing_batch_stores_none_of_its_messages() {
    let repo = test_repo().await;
    poison_subject(&repo, "bad").await;
    let batch: Vec<PreparedMessage> = ["good", "bad", "also-good"].map(prepared).into();

    assert!(repo.insert_batch(&batch).await.is_err());
    assert_eq!(repo.count().await.unwrap(), 0);
    assert!(repo.search("good", 10, 0).await.unwrap().is_empty());
  }

  #[tokio::test]
  async fn list_returns_newest_first() {
    let repo = test_repo().await;

    let s1 = repo
      .insert(
        "a@test.com",
        &["b@test.com".into()],
        &raw_email("First", "a@test.com", "b@test.com"),
      )
      .await
      .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let s2 = repo
      .insert(
        "a@test.com",
        &["b@test.com".into()],
        &raw_email("Second", "a@test.com", "b@test.com"),
      )
      .await
      .unwrap();

    let list = repo.list(50, 0).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, s2.id);
    assert_eq!(list[1].id, s1.id);
  }

  #[tokio::test]
  async fn list_pagination() {
    let repo = test_repo().await;
    for i in 0..5 {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("Msg {i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let page1 = repo.list(2, 0).await.unwrap();
    let page2 = repo.list(2, 2).await.unwrap();
    let page3 = repo.list(2, 4).await.unwrap();

    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page3.len(), 1);
    assert_ne!(page1[0].id, page2[0].id);
  }

  async fn insert_subjects(repo: &MessageRepository, subjects: &[&str]) -> Vec<String> {
    let mut ids = Vec::new();
    for subject in subjects {
      let stored = repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(subject, "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
      ids.push(stored.id);
    }
    ids
  }

  fn ids_of(page: &[MessageSummary]) -> Vec<&str> {
    page.iter().map(|m| m.id.as_str()).collect()
  }

  #[tokio::test]
  async fn a_page_before_a_cursor_holds_only_older_messages() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2", "m3", "m4"]).await;
    let cursor = repo.cursor(&ids[3]).await.unwrap();

    let page = repo
      .list_page(&all(), PageStart::Before(cursor), 50)
      .await
      .unwrap();

    assert_eq!(ids_of(&page), [&ids[2], &ids[1], &ids[0]]);
  }

  #[tokio::test]
  async fn a_cursor_page_matches_the_offset_page_it_replaces() {
    let repo = test_repo().await;
    insert_subjects(&repo, &["m0", "m1", "m2", "m3", "m4", "m5"]).await;
    let first = repo.list(2, 0).await.unwrap();
    let cursor = repo.cursor(&first[1].id).await.unwrap();

    let by_cursor = repo
      .list_page(&all(), PageStart::Before(cursor), 2)
      .await
      .unwrap();
    let by_offset = repo.list(2, 2).await.unwrap();

    assert_eq!(ids_of(&by_cursor), ids_of(&by_offset));
  }

  #[tokio::test]
  async fn a_search_page_before_a_cursor_holds_only_older_matches() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["invoice a", "memo", "invoice b", "invoice c"]).await;
    let cursor = repo.cursor(&ids[3]).await.unwrap();

    let page = repo
      .search_page("invoice", &all(), PageStart::Before(cursor), 50)
      .await
      .unwrap();

    assert_eq!(ids_of(&page), [&ids[2], &ids[0]]);
  }

  fn all() -> MessageFilter {
    MessageFilter::default()
  }

  async fn star(repo: &MessageRepository, id: &str) {
    repo
      .update_message(id, None, Some(true), None)
      .await
      .unwrap();
  }

  async fn mark_read(repo: &MessageRepository, id: &str) {
    repo
      .update_message(id, Some(true), None, None)
      .await
      .unwrap();
  }

  async fn tag(repo: &MessageRepository, id: &str, tags: &[&str]) {
    let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    repo
      .update_message(id, None, None, Some(&tags))
      .await
      .unwrap();
  }

  async fn filtered(repo: &MessageRepository, filter: &MessageFilter) -> Vec<String> {
    repo
      .list_page(filter, PageStart::Offset(0), 50)
      .await
      .unwrap()
      .into_iter()
      .map(|m| m.id)
      .collect()
  }

  #[tokio::test]
  async fn the_starred_filter_keeps_only_starred_messages() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    star(&repo, &ids[1]).await;

    let starred = MessageFilter {
      starred: true,
      ..all()
    };

    assert_eq!(filtered(&repo, &starred).await, [ids[1].clone()]);
  }

  #[tokio::test]
  async fn the_unread_filter_drops_read_messages() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    mark_read(&repo, &ids[1]).await;

    let unread = MessageFilter {
      unread: true,
      ..all()
    };

    assert_eq!(
      filtered(&repo, &unread).await,
      [ids[2].clone(), ids[0].clone()]
    );
  }

  #[tokio::test]
  async fn the_attachment_filter_keeps_only_messages_with_attachments() {
    let repo = test_repo().await;
    insert_subjects(&repo, &["plain"]).await;
    let with_file = repo
      .insert("a@t.com", &["b@t.com".into()], &multipart_email("report"))
      .await
      .unwrap();

    let with_attachments = MessageFilter {
      has_attachments: true,
      ..all()
    };

    assert_eq!(filtered(&repo, &with_attachments).await, [with_file.id]);
  }

  #[tokio::test]
  async fn the_tag_filter_matches_whole_tags_only() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    tag(&repo, &ids[0], &["urgent"]).await;
    tag(&repo, &ids[1], &["urgent-ish"]).await;
    tag(&repo, &ids[2], &["Urgent"]).await;

    let urgent = MessageFilter {
      tags: vec!["urgent".into()],
      ..all()
    };

    assert_eq!(filtered(&repo, &urgent).await, [ids[0].clone()]);
  }

  #[tokio::test]
  async fn several_tags_match_a_message_carrying_any_of_them() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    tag(&repo, &ids[0], &["a"]).await;
    tag(&repo, &ids[2], &["x", "b"]).await;

    let a_or_b = MessageFilter {
      tags: vec!["a".into(), "b".into()],
      ..all()
    };

    assert_eq!(
      filtered(&repo, &a_or_b).await,
      [ids[2].clone(), ids[0].clone()]
    );
  }

  #[tokio::test]
  async fn filters_combine_with_and() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    star(&repo, &ids[0]).await;
    star(&repo, &ids[1]).await;
    mark_read(&repo, &ids[1]).await;

    let starred_unread = MessageFilter {
      starred: true,
      unread: true,
      ..all()
    };

    assert_eq!(filtered(&repo, &starred_unread).await, [ids[0].clone()]);
  }

  #[tokio::test]
  async fn a_filtered_page_before_a_cursor_holds_only_older_matches() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2", "m3"]).await;
    for id in &ids {
      star(&repo, id).await;
    }
    let cursor = repo.cursor(&ids[2]).await.unwrap();
    let starred = MessageFilter {
      starred: true,
      ..all()
    };

    let page = repo
      .list_page(&starred, PageStart::Before(cursor), 1)
      .await
      .unwrap();

    assert_eq!(ids_of(&page), [&ids[1]]);
  }

  #[tokio::test]
  async fn a_filtered_search_keeps_only_matches_passing_the_filter() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["invoice a", "memo", "invoice b"]).await;
    star(&repo, &ids[0]).await;
    star(&repo, &ids[1]).await;
    let starred = MessageFilter {
      starred: true,
      ..all()
    };

    let page = repo
      .search_page("invoice", &starred, PageStart::Offset(0), 50)
      .await
      .unwrap();

    assert_eq!(ids_of(&page), [&ids[0]]);
  }

  #[tokio::test]
  async fn the_filtered_count_counts_only_matching_messages() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["m0", "m1", "m2"]).await;
    star(&repo, &ids[0]).await;
    star(&repo, &ids[2]).await;
    let starred = MessageFilter {
      starred: true,
      ..all()
    };

    assert_eq!(repo.count_filtered(&starred).await.unwrap(), 2);
  }

  #[tokio::test]
  async fn the_filtered_search_count_needs_both_the_query_and_the_filter() {
    let repo = test_repo().await;
    let ids = insert_subjects(&repo, &["invoice a", "memo", "invoice b"]).await;
    star(&repo, &ids[0]).await;
    star(&repo, &ids[1]).await;
    let starred = MessageFilter {
      starred: true,
      ..all()
    };

    assert_eq!(
      repo
        .search_count_filtered("invoice", &starred)
        .await
        .unwrap(),
      1
    );
  }

  #[tokio::test]
  async fn the_cursor_of_an_unknown_id_is_not_found() {
    let repo = test_repo().await;

    let error = repo.cursor("01ARZ3NDEKTSV4RRFFQ69G5FAV").await.unwrap_err();

    assert!(matches!(error, StorageError::NotFound(_)));
  }

  #[tokio::test]
  async fn search_by_subject() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Invoice #42", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Meeting notes", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let results = repo.search("Invoice", 50, 0).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].subject.as_deref(), Some("Invoice #42"));
  }

  #[tokio::test]
  async fn search_by_sender() {
    let repo = test_repo().await;
    repo
      .insert(
        "alice@corp.com",
        &["b@t.com".into()],
        &raw_email("Hi", "alice@corp.com", "b@t.com"),
      )
      .await
      .unwrap();
    repo
      .insert(
        "bob@corp.com",
        &["b@t.com".into()],
        &raw_email("Hi", "bob@corp.com", "b@t.com"),
      )
      .await
      .unwrap();

    let results = repo.search("alice", 50, 0).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].sender, "alice@corp.com");
  }

  #[tokio::test]
  async fn count_and_count_matching() {
    let repo = test_repo().await;
    repo
      .insert(
        "alice@t.com",
        &["b@t.com".into()],
        &raw_email("Welcome", "alice@t.com", "b@t.com"),
      )
      .await
      .unwrap();
    repo
      .insert(
        "bob@t.com",
        &["c@t.com".into()],
        &raw_email("Welcome", "bob@t.com", "c@t.com"),
      )
      .await
      .unwrap();
    repo
      .insert(
        "alice@t.com",
        &["d@t.com".into()],
        &raw_email("Goodbye", "alice@t.com", "d@t.com"),
      )
      .await
      .unwrap();

    assert_eq!(repo.count().await.unwrap(), 3);
    assert_eq!(
      repo
        .count_matching(Some("welcome"), None, None)
        .await
        .unwrap(),
      2
    );
    assert_eq!(
      repo
        .count_matching(None, Some("alice"), None)
        .await
        .unwrap(),
      2
    );
    assert_eq!(
      repo
        .count_matching(Some("welcome"), Some("bob"), None)
        .await
        .unwrap(),
      1
    );
    assert_eq!(
      repo
        .count_matching(None, None, Some("d@t.com"))
        .await
        .unwrap(),
      1
    );
  }

  #[tokio::test]
  async fn update_message_fields() {
    let repo = test_repo().await;
    let s = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Test", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    repo
      .update_message(&s.id, Some(true), None, None)
      .await
      .unwrap();
    let msg = repo.get(&s.id).await.unwrap();
    assert!(msg.is_read);
    assert!(!msg.is_starred);

    repo
      .update_message(&s.id, None, Some(true), None)
      .await
      .unwrap();
    let msg = repo.get(&s.id).await.unwrap();
    assert!(msg.is_starred);

    let tags = vec!["important".into(), "work".into()];
    repo
      .update_message(&s.id, None, None, Some(&tags))
      .await
      .unwrap();
    let msg = repo.get(&s.id).await.unwrap();
    let parsed_tags: Vec<String> = serde_json::from_str(&msg.tags).unwrap();
    assert_eq!(parsed_tags, vec!["important", "work"]);
  }

  #[tokio::test]
  async fn update_nonexistent_returns_not_found() {
    let repo = test_repo().await;
    let err = repo
      .update_message("nonexistent", Some(true), None, None)
      .await
      .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
  }

  #[tokio::test]
  async fn delete_single() {
    let repo = test_repo().await;
    let s = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Del", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    repo.delete(&s.id).await.unwrap();
    assert_eq!(repo.count().await.unwrap(), 0);

    let err = repo.get(&s.id).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
  }

  #[tokio::test]
  async fn delete_nonexistent_returns_not_found() {
    let repo = test_repo().await;
    let err = repo.delete("nonexistent").await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
  }

  #[tokio::test]
  async fn delete_all() {
    let repo = test_repo().await;
    for i in 0..3 {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let deleted = repo.delete_all().await.unwrap();
    assert_eq!(deleted, 3);
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  #[tokio::test]
  async fn delete_cleans_fts() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Unique subject xyz", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let results = repo.search("xyz", 50, 0).await.unwrap();
    assert_eq!(results.len(), 1);
    let id = results[0].id.clone();

    repo.delete(&id).await.unwrap();

    let results = repo.search("xyz", 50, 0).await.unwrap();
    assert_eq!(results.len(), 0);
  }

  #[tokio::test]
  async fn delete_all_cleans_fts() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Searchable abc", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    repo.delete_all().await.unwrap();

    let results = repo.search("abc", 50, 0).await.unwrap();
    assert_eq!(results.len(), 0);
  }

  #[tokio::test]
  async fn repeated_delete_all_does_not_grow_the_fts_index() {
    let repo = test_repo().await;

    async fn index_rows(repo: &MessageRepository) -> i64 {
      sqlx::query_scalar("SELECT count(*) FROM messages_fts_data")
        .fetch_one(&repo.writer)
        .await
        .unwrap()
    }

    let mut sizes = Vec::new();
    for round in 0..3 {
      for i in 0..5 {
        repo
          .insert(
            "a@t.com",
            &["b@t.com".into()],
            &raw_email(&format!("round{round}msg{i}"), "a@t.com", "b@t.com"),
          )
          .await
          .unwrap();
      }
      repo.delete_all().await.unwrap();
      sizes.push(index_rows(&repo).await);
    }

    assert!(
      sizes.iter().all(|size| *size == sizes[0]),
      "index kept growing across delete_all cycles: {sizes:?}"
    );
    assert!(
      repo.search("round0msg0", 50, 0).await.unwrap().is_empty(),
      "deleted terms must not stay in the index"
    );
  }

  #[tokio::test]
  async fn get_raw_bytes() {
    let repo = test_repo().await;
    let raw = raw_email("Raw test", "a@t.com", "b@t.com");
    let s = repo
      .insert("a@t.com", &["b@t.com".into()], &raw)
      .await
      .unwrap();

    let fetched = repo.get_raw(&s.id).await.unwrap();
    assert_eq!(fetched, raw);
  }

  #[tokio::test]
  async fn get_raw_prefix_returns_only_the_requested_bytes() {
    let repo = test_repo().await;
    let raw = raw_email("Prefix test", "a@t.com", "b@t.com");
    let s = repo
      .insert("a@t.com", &["b@t.com".into()], &raw)
      .await
      .unwrap();

    let head = repo.get_raw_prefix(&s.id, 12).await.unwrap();
    assert_eq!(head, raw[..12]);

    let beyond_end = repo
      .get_raw_prefix(&s.id, raw.len() as i64 * 2)
      .await
      .unwrap();
    assert_eq!(
      beyond_end, raw,
      "asking for more than the message holds must yield the whole message"
    );
  }

  #[tokio::test]
  async fn get_raw_prefix_reports_a_missing_message() {
    let repo = test_repo().await;
    assert!(matches!(
      repo.get_raw_prefix("nope", 16).await,
      Err(StorageError::NotFound(_))
    ));
  }

  /// Enough messages to land inside one millisecond, which is where ULID
  /// ordering and arrival ordering diverge.
  const BURST: usize = 40;

  #[tokio::test]
  async fn search_and_list_agree_on_order_within_a_burst() {
    let repo = test_repo().await;

    for i in 0..BURST {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("burstterm item{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let listed: Vec<String> = repo
      .list(BURST as i64, 0)
      .await
      .unwrap()
      .into_iter()
      .map(|m| m.id)
      .collect();
    let found: Vec<String> = repo
      .search("burstterm", BURST as i64, 0)
      .await
      .unwrap()
      .into_iter()
      .map(|m| m.id)
      .collect();

    assert_eq!(listed.len(), BURST);
    assert_eq!(
      found, listed,
      "browsing and searching must return the same order, newest first"
    );
  }

  #[tokio::test]
  async fn search_counts_every_match_without_joining_the_source_table() {
    let repo = test_repo().await;

    for i in 0..BURST {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("countterm item{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    assert_eq!(repo.search_count("countterm").await.unwrap(), BURST as i64);
    assert_eq!(repo.search_count("nothingmatchesthis").await.unwrap(), 0);
  }

  #[tokio::test]
  async fn attachments_stored_and_retrieved() {
    let repo = test_repo().await;
    let raw = multipart_email("With attachment");
    let s = repo
      .insert("sender@test.com", &["rcpt@test.com".into()], &raw)
      .await
      .unwrap();

    assert!(s.has_attachments);

    let attachments = repo.get_attachments(&s.id).await.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
    assert_eq!(
      attachments[0].content_type.as_deref(),
      Some("application/pdf")
    );

    let full = repo
      .get_attachment(&s.id, &attachments[0].id)
      .await
      .unwrap();
    assert!(!full.content.is_empty());
  }

  #[tokio::test]
  async fn trim_to_max() {
    let repo = test_repo().await;
    for i in 0..5 {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let deleted_ids = repo.trim_to_max(3).await.unwrap();
    assert_eq!(deleted_ids.len(), 2);
    assert_eq!(repo.count().await.unwrap(), 3);

    let remaining = repo.list(50, 0).await.unwrap();
    assert_eq!(remaining.len(), 3);
  }

  #[tokio::test]
  async fn trim_to_max_removes_the_oldest_rows_and_their_index_entries() {
    let repo = test_repo().await;
    let mut ids = Vec::new();
    for i in 0..5 {
      let summary = repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("trimmed{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
      ids.push(summary.id);
    }

    let mut deleted = repo.trim_to_max(2).await.unwrap();

    let mut oldest = ids[..3].to_vec();
    oldest.sort();
    deleted.sort();
    assert_eq!(deleted, oldest);
    sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)")
      .execute(&repo.writer)
      .await
      .expect("the index must hold no entries for trimmed rows");
  }

  #[tokio::test]
  async fn trim_to_max_under_the_cap_deletes_nothing() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("kept", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    assert!(repo.trim_to_max(1).await.unwrap().is_empty());
    assert_eq!(repo.count().await.unwrap(), 1);
  }

  #[tokio::test]
  async fn delete_older_than() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Old", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let future_cutoff = "2099-01-01T00:00:00Z";
    let deleted_ids = repo.delete_older_than(future_cutoff).await.unwrap();
    assert_eq!(deleted_ids.len(), 1);
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  #[tokio::test]
  async fn delete_older_than_preserves_recent() {
    let repo = test_repo().await;
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("Recent", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let past_cutoff = "2000-01-01T00:00:00Z";
    let deleted_ids = repo.delete_older_than(past_cutoff).await.unwrap();
    assert_eq!(deleted_ids.len(), 0);
    assert_eq!(repo.count().await.unwrap(), 1);
  }

  #[tokio::test]
  async fn multiple_recipients_stored_as_json() {
    let repo = test_repo().await;
    let recipients = vec!["a@t.com".into(), "b@t.com".into(), "c@t.com".into()];
    let s = repo
      .insert(
        "from@t.com",
        &recipients,
        &raw_email("Multi", "from@t.com", "a@t.com"),
      )
      .await
      .unwrap();

    let parsed: Vec<String> = serde_json::from_str(&s.recipients).unwrap();
    assert_eq!(parsed, recipients);
  }

  async fn shared_repo() -> (MessageRepository, std::path::PathBuf) {
    // File-backed temp DB so multiple pooled connections hit the same store.
    // Mirrors production (WAL + file) far better than shared-cache in-memory,
    // where FTS5 hits SQLITE_LOCKED under concurrent writes.
    //
    // The pool is built through `connect_options` for the same reason: a raw
    // URL leaves every connection on SQLite's defaults, including a
    // `busy_timeout` of zero, so any write collision fails instantly instead
    // of waiting the way production does.
    let dir = std::env::temp_dir().join(format!("rustmail-test-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .max_connections(8)
      .connect_with(connect_options(&url).unwrap())
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    (MessageRepository::new(pool), dir)
  }

  struct TempDir(std::path::PathBuf);
  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[tokio::test]
  async fn concurrent_inserts_all_persisted_with_unique_ids() {
    let (repo, dir) = shared_repo().await;
    let _guard = TempDir(dir);

    let mut handles = Vec::new();
    for i in 0..32 {
      let repo = repo.clone();
      handles.push(tokio::spawn(async move {
        repo
          .insert(
            "a@t.com",
            &["b@t.com".into()],
            &raw_email(&format!("concurrent-{i}"), "a@t.com", "b@t.com"),
          )
          .await
          .unwrap()
          .id
      }));
    }

    let mut ids = Vec::new();
    for h in handles {
      ids.push(h.await.unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(
      ids.len(),
      32,
      "ULIDs must be unique under concurrent inserts"
    );
    assert_eq!(repo.count().await.unwrap(), 32);
  }

  #[tokio::test]
  async fn concurrent_search_during_inserts_returns_consistent_results() {
    let (repo, dir) = shared_repo().await;
    let _guard = TempDir(dir);

    let writer_repo = repo.clone();
    let writer = tokio::spawn(async move {
      for i in 0..20 {
        writer_repo
          .insert(
            "w@t.com",
            &["r@t.com".into()],
            &raw_email(&format!("writer-{i}"), "w@t.com", "r@t.com"),
          )
          .await
          .unwrap();
      }
    });

    // Poll search while writer is running. Must never panic or error.
    let reader_repo = repo.clone();
    let reader = tokio::spawn(async move {
      let mut observations = Vec::new();
      for _ in 0..20 {
        let results = reader_repo.search("writer", 50, 0).await.unwrap();
        observations.push(results.len());
      }
      observations
    });

    writer.await.unwrap();
    let observed = reader.await.unwrap();
    assert!(observed.iter().all(|n| *n <= 20));

    // Final state matches writer output.
    let final_results = repo.search("writer", 50, 0).await.unwrap();
    assert_eq!(final_results.len(), 20);
  }

  /// Pool that never waits on a lock, so only a retry can get a write through.
  async fn impatient_repo() -> (MessageRepository, std::path::PathBuf, SqlitePool) {
    let dir = std::env::temp_dir().join(format!("rustmail-busy-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.join("test.db").display());
    let options = connect_options(&url)
      .unwrap()
      .busy_timeout(std::time::Duration::ZERO);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .max_connections(4)
      .connect_with(options)
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    (MessageRepository::new(pool.clone()), dir, pool)
  }

  /// A writer holds the lock for less than the retry budget.
  ///
  /// The sleep stands in for a concurrent writer's duration, not for test
  /// synchronisation: the hold has to overlap the insert's first attempt and
  /// end well inside the retry budget, which is at least 300ms of backoff.
  #[tokio::test]
  async fn insert_waits_out_a_writer_holding_the_lock() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    let hold = std::time::Duration::from_millis(60);

    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();

    let releaser = tokio::spawn(async move {
      tokio::time::sleep(hold).await;
      sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    });

    let summary = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("contended", "a@t.com", "b@t.com"),
      )
      .await
      .expect("a contended insert should be retried, not surfaced");

    releaser.await.unwrap();
    assert_eq!(repo.count().await.unwrap(), 1);
    assert_eq!(summary.sender, "a@t.com");
  }

  #[tokio::test]
  async fn insert_gives_up_when_the_lock_is_never_released() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);

    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();

    let error = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("blocked", "a@t.com", "b@t.com"),
      )
      .await
      .expect_err("a lock that never clears must surface, not hang forever");

    assert!(
      is_retryable_lock(&error),
      "expected a lock error to be reported as-is, got {error:?}"
    );
    sqlx::query("ROLLBACK")
      .execute(&mut *blocker)
      .await
      .unwrap();
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  /// Holds the write lock on `pool` for `hold`, then releases it.
  ///
  /// The sleep stands in for a concurrent writer's duration, as in
  /// [`insert_waits_out_a_writer_holding_the_lock`].
  async fn hold_write_lock(pool: &SqlitePool, hold: Duration) -> tokio::task::JoinHandle<()> {
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();
    tokio::spawn(async move {
      tokio::time::sleep(hold).await;
      sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    })
  }

  #[tokio::test]
  async fn a_lock_that_outlasts_the_retries_is_store_wide() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();

    let error = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("blocked", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap_err();

    assert!(error.is_store_wide(), "got {error:?}");
    sqlx::query("ROLLBACK")
      .execute(&mut *blocker)
      .await
      .unwrap();
  }

  #[tokio::test]
  async fn a_closed_pool_is_store_wide() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    pool.close().await;

    let error = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("closed", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap_err();

    assert!(error.is_store_wide(), "got {error:?}");
  }

  #[tokio::test]
  async fn a_row_the_schema_refuses_is_not_store_wide() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    sqlx::query(
      "CREATE TRIGGER poison BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'poisoned'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("poison", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap_err();

    assert!(!error.is_store_wide(), "got {error:?}");
  }

  const CONTENDED_HOLD: Duration = Duration::from_millis(60);

  #[tokio::test]
  async fn delete_older_than_waits_out_a_writer_holding_the_lock() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("old", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let releaser = hold_write_lock(&pool, CONTENDED_HOLD).await;
    let deleted = repo
      .delete_older_than("2099-01-01T00:00:00Z")
      .await
      .expect("a contended retention purge should be retried, not surfaced");

    releaser.await.unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(repo.count().await.unwrap(), 0);
  }

  #[tokio::test]
  async fn trim_to_max_waits_out_a_writer_holding_the_lock() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    for i in 0..3 {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("M{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let releaser = hold_write_lock(&pool, CONTENDED_HOLD).await;
    let deleted = repo
      .trim_to_max(1)
      .await
      .expect("a contended trim should be retried, not surfaced");

    releaser.await.unwrap();
    assert_eq!(deleted.len(), 2);
    assert_eq!(repo.count().await.unwrap(), 1);
  }

  #[tokio::test]
  async fn delete_older_than_skips_the_write_lock_when_nothing_matches() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("recent", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();

    let deleted = tokio::time::timeout(
      Duration::from_millis(500),
      repo.delete_older_than("2000-01-01T00:00:00Z"),
    )
    .await
    .expect("a no-op purge must not wait on a write lock it never needs")
    .unwrap();

    sqlx::query("ROLLBACK")
      .execute(&mut *blocker)
      .await
      .unwrap();

    assert!(deleted.is_empty());
    assert_eq!(repo.count().await.unwrap(), 1);
  }

  #[tokio::test]
  async fn trim_to_max_skips_the_write_lock_when_under_the_cap() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("kept", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
      .execute(&mut *blocker)
      .await
      .unwrap();

    let deleted = tokio::time::timeout(Duration::from_millis(500), repo.trim_to_max(5))
      .await
      .expect("a no-op trim must not wait on a write lock it never needs")
      .unwrap();

    sqlx::query("ROLLBACK")
      .execute(&mut *blocker)
      .await
      .unwrap();

    assert!(deleted.is_empty());
    assert_eq!(repo.count().await.unwrap(), 1);
  }

  #[tokio::test]
  async fn update_message_waits_out_a_writer_holding_the_lock() {
    let (repo, dir, pool) = impatient_repo().await;
    let _guard = TempDir(dir);
    let summary = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("flag me", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();

    let releaser = hold_write_lock(&pool, CONTENDED_HOLD).await;
    repo
      .update_message(&summary.id, Some(true), None, None)
      .await
      .expect("a contended update should be retried, not surfaced");

    releaser.await.unwrap();
    assert!(repo.get(&summary.id).await.unwrap().is_read);
  }

  #[tokio::test]
  async fn update_message_leaves_unset_fields_alone() {
    let repo = test_repo().await;
    let summary = repo
      .insert(
        "a@t.com",
        &["b@t.com".into()],
        &raw_email("partial", "a@t.com", "b@t.com"),
      )
      .await
      .unwrap();
    let tags = vec!["keep".to_string()];
    repo
      .update_message(&summary.id, Some(true), Some(true), Some(&tags))
      .await
      .unwrap();

    repo
      .update_message(&summary.id, None, Some(false), None)
      .await
      .unwrap();

    let msg = repo.get(&summary.id).await.unwrap();
    assert!(msg.is_read);
    assert!(!msg.is_starred);
    let stored_tags: Vec<String> = serde_json::from_str(&msg.tags).unwrap();
    assert_eq!(stored_tags, tags);
  }

  #[tokio::test]
  async fn concurrent_insert_and_delete_all_leaves_no_fts_orphans() {
    let (repo, dir) = shared_repo().await;
    let _guard = TempDir(dir);

    // Seed some rows so delete_all has something to remove.
    for i in 0..10 {
      repo
        .insert(
          "a@t.com",
          &["b@t.com".into()],
          &raw_email(&format!("seed-{i}"), "a@t.com", "b@t.com"),
        )
        .await
        .unwrap();
    }

    let writer_repo = repo.clone();
    let writer = tokio::spawn(async move {
      for i in 0..10 {
        writer_repo
          .insert(
            "a@t.com",
            &["b@t.com".into()],
            &raw_email(&format!("writer-{i}"), "a@t.com", "b@t.com"),
          )
          .await
          .unwrap();
      }
    });

    let deleter_repo = repo.clone();
    let deleter = tokio::spawn(async move {
      // Let a few writes land, then wipe.
      tokio::time::sleep(std::time::Duration::from_millis(5)).await;
      deleter_repo.delete_all().await.unwrap()
    });

    writer.await.unwrap();
    let _deleted = deleter.await.unwrap();

    // delete_all racing with inserts must not deadlock, error, or leave
    // counts impossible. Final row count must be in [0, writes_from_writer]
    // because deleter wipes everything then the tail of the writer's batch
    // may or may not have landed afterwards — both outcomes are legal.
    let remaining = repo.count().await.unwrap();
    assert!(
      (0..=10).contains(&remaining),
      "row count out of expected range after delete_all race: {remaining}"
    );

    // Every row that is still in messages must also be findable via FTS5 —
    // INNER JOIN in search() naturally filters orphan FTS rows, so if any
    // row lacked an FTS entry the hits count would be strictly less than
    // the row count.
    let writer_hits = repo.search("writer", 100, 0).await.unwrap();
    let seed_hits = repo.search("seed", 100, 0).await.unwrap();
    let total_hits = (writer_hits.len() + seed_hits.len()) as i64;
    assert!(
      total_hits >= remaining,
      "every stored row must be FTS-searchable ({total_hits} hits, {remaining} rows)"
    );
  }

  const SEEDED_MESSAGES: usize = 6;
  const PLAN_TABLES_ALLOWED: &[&str] = &["m", "fts", "messages", "messages_fts", "json_each"];

  fn every_filter() -> MessageFilter {
    MessageFilter {
      starred: true,
      unread: true,
      has_attachments: true,
      tags: vec!["a".to_string(), "b".to_string()],
    }
  }

  fn listing_statements() -> Vec<(&'static str, String)> {
    let quoted = || "\"term\"".to_string();
    let mut statements = Vec::new();
    for filter in [MessageFilter::default(), every_filter()] {
      for start in [PageStart::Offset(0), PageStart::Before(Cursor(1))] {
        statements.push(("list", list_statement(&filter, start, 50).into_sql()));
        statements.push((
          "search",
          search_statement(quoted(), &filter, start, 50).into_sql(),
        ));
      }
      statements.push(("count", count_statement(&filter).into_sql()));
      statements.push((
        "search count",
        search_count_statement(quoted(), &filter).into_sql(),
      ));
    }
    let (count_matching, _) = count_matching_statement(Some("s"), Some("f"), Some("r"));
    statements.push(("count matching", count_matching));
    statements.push(("count", "SELECT COUNT(*) FROM messages".to_string()));
    statements.push((
      "search count",
      "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?1".to_string(),
    ));
    statements
  }

  /// Tables and aliases each `SCAN` or `SEARCH` step of `sql`'s plan reads.
  async fn tables_read(repo: &MessageRepository, sql: &str) -> Vec<String> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sql}"))
      .fetch_all(&repo.readers)
      .await
      .unwrap();
    rows
      .into_iter()
      .filter_map(|(_, _, _, detail)| {
        let mut words = detail.split_whitespace();
        match words.next() {
          Some("SCAN" | "SEARCH") => words.next().map(str::to_string),
          _ => None,
        }
      })
      .collect()
  }

  async fn seed_mailbox(repo: &MessageRepository) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..SEEDED_MESSAGES {
      let raw = if i % 2 == 0 {
        multipart_email(&format!("seeded item{i} attached"))
      } else {
        raw_email(&format!("seeded item{i} plain"), "a@t.com", "b@t.com")
      };
      let summary = repo
        .insert("sender@test.com", &["rcpt@test.com".into()], &raw)
        .await
        .unwrap();
      ids.push(summary.id);
    }
    ids
  }

  async fn column_set(repo: &MessageRepository, sql: &str) -> Vec<i64> {
    sqlx::query_scalar(sql)
      .fetch_all(&repo.writer)
      .await
      .unwrap()
  }

  /// Asserts the index, content and attachments hold rows for exactly the
  /// stored messages, and that the index agrees with its content.
  async fn assert_rows_follow_messages(repo: &MessageRepository) {
    let seqs = column_set(repo, "SELECT seq FROM messages ORDER BY seq").await;
    assert_eq!(
      column_set(repo, "SELECT id FROM messages_fts_docsize ORDER BY id").await,
      seqs,
      "the FTS index must hold one document per stored message"
    );
    assert_eq!(
      column_set(repo, "SELECT seq FROM message_content ORDER BY seq").await,
      seqs,
      "every stored message, and only those, must keep its content"
    );
    let orphaned_attachments: i64 = sqlx::query_scalar(
      "SELECT COUNT(*) FROM attachments WHERE message_seq NOT IN (SELECT seq FROM messages)",
    )
    .fetch_one(&repo.writer)
    .await
    .unwrap();
    assert_eq!(
      orphaned_attachments, 0,
      "attachments outlived their message"
    );
    sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)")
      .execute(&repo.writer)
      .await
      .expect("the FTS index must match its content");
  }

  async fn attachment_rows(repo: &MessageRepository) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
      .fetch_one(&repo.writer)
      .await
      .unwrap()
  }

  async fn backdate(repo: &MessageRepository, ids: &[String]) {
    for id in ids {
      sqlx::query("UPDATE messages SET created_at = '2000-01-01T00:00:00Z' WHERE id = ?1")
        .bind(id)
        .execute(&repo.writer)
        .await
        .unwrap();
    }
  }

  #[tokio::test]
  async fn listing_searching_and_counting_plan_no_read_of_message_content() {
    let repo = test_repo().await;

    for (path, sql) in listing_statements() {
      let tables = tables_read(&repo, &sql).await;
      assert!(!tables.is_empty(), "{path}: no plan for {sql}");
      assert!(
        tables
          .iter()
          .all(|table| PLAN_TABLES_ALLOWED.contains(&table.as_str())),
        "{path} reads beyond the metadata: {tables:?} in {sql}"
      );
    }
  }

  #[tokio::test]
  async fn listing_searching_and_counting_work_without_message_content() {
    let repo = test_repo().await;
    let ids = seed_mailbox(&repo).await;
    sqlx::query("DROP VIEW messages_fts_source")
      .execute(&repo.writer)
      .await
      .unwrap();
    sqlx::query("DROP TABLE message_content")
      .execute(&repo.writer)
      .await
      .unwrap();
    let attached = MessageFilter {
      has_attachments: true,
      ..MessageFilter::default()
    };
    let cursor = repo.cursor(&ids[SEEDED_MESSAGES - 1]).await.unwrap();

    assert_eq!(repo.list(50, 0).await.unwrap().len(), SEEDED_MESSAGES);
    assert_eq!(
      repo
        .list_page(&attached, PageStart::Before(cursor), 50)
        .await
        .unwrap()
        .len(),
      SEEDED_MESSAGES / 2
    );
    assert_eq!(
      repo.search("seeded", 50, 0).await.unwrap().len(),
      SEEDED_MESSAGES
    );
    assert_eq!(
      repo
        .search_page("seeded", &attached, PageStart::Before(cursor), 50)
        .await
        .unwrap()
        .len(),
      SEEDED_MESSAGES / 2
    );
    assert_eq!(
      repo.search_count("seeded").await.unwrap(),
      SEEDED_MESSAGES as i64
    );
    assert_eq!(
      repo
        .search_count_filtered("seeded", &attached)
        .await
        .unwrap(),
      (SEEDED_MESSAGES / 2) as i64
    );
    assert_eq!(repo.count().await.unwrap(), SEEDED_MESSAGES as i64);
    assert_eq!(
      repo.count_filtered(&attached).await.unwrap(),
      (SEEDED_MESSAGES / 2) as i64
    );
    assert_eq!(
      repo
        .count_matching(Some("attached"), Some("sender@"), Some("rcpt@"))
        .await
        .unwrap(),
      (SEEDED_MESSAGES / 2) as i64
    );
  }

  #[tokio::test]
  async fn deleting_a_message_takes_its_index_entry_content_and_attachments() {
    let repo = test_repo().await;
    let ids = seed_mailbox(&repo).await;
    let attachments_before = attachment_rows(&repo).await;

    repo.delete(&ids[0]).await.unwrap();

    assert_rows_follow_messages(&repo).await;
    assert_eq!(attachment_rows(&repo).await, attachments_before - 1);
    assert_eq!(repo.count().await.unwrap(), (SEEDED_MESSAGES - 1) as i64);
  }

  #[tokio::test]
  async fn deleting_every_message_takes_every_index_entry_content_and_attachment() {
    let repo = test_repo().await;
    seed_mailbox(&repo).await;

    assert_eq!(repo.delete_all().await.unwrap(), SEEDED_MESSAGES as u64);

    assert_rows_follow_messages(&repo).await;
    assert_eq!(attachment_rows(&repo).await, 0);
  }

  #[tokio::test]
  async fn expiring_messages_takes_their_index_entries_content_and_attachments() {
    let repo = test_repo().await;
    let ids = seed_mailbox(&repo).await;
    let expired = &ids[..2];
    backdate(&repo, expired).await;

    let mut deleted = repo
      .delete_older_than("2001-01-01T00:00:00Z")
      .await
      .unwrap();

    deleted.sort();
    let mut expected = expired.to_vec();
    expected.sort();
    assert_eq!(deleted, expected);
    assert_rows_follow_messages(&repo).await;
    assert_eq!(
      attachment_rows(&repo).await,
      (SEEDED_MESSAGES / 2 - 1) as i64
    );
  }

  #[tokio::test]
  async fn trimming_messages_takes_their_index_entries_content_and_attachments() {
    let repo = test_repo().await;
    seed_mailbox(&repo).await;

    let deleted = repo.trim_to_max(3).await.unwrap();

    assert_eq!(deleted.len(), SEEDED_MESSAGES - 3);
    assert_rows_follow_messages(&repo).await;
    assert_eq!(attachment_rows(&repo).await, 1);
  }

  #[tokio::test]
  async fn a_message_with_attachments_reads_back_its_bodies_raw_and_parts_in_order() {
    let repo = test_repo().await;
    let raw = multipart_email("layout");
    let summary = repo
      .insert("sender@test.com", &["rcpt@test.com".into()], &raw)
      .await
      .unwrap();

    let message = repo.get(&summary.id).await.unwrap();
    assert_eq!(message.text_body.as_deref(), Some("Body text"));
    assert_eq!(repo.get_raw(&summary.id).await.unwrap(), raw);
    let attachments = repo.get_attachments(&summary.id).await.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].message_id, summary.id);
    let attachment = repo
      .get_attachment(&summary.id, &attachments[0].id)
      .await
      .unwrap();
    assert_eq!(attachment.message_id, summary.id);
    assert_eq!(attachment.content, b"fake-pdf-content");
  }
}
