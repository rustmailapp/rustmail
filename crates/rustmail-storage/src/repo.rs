use mail_parser::{MessageParser, MimeHeaders, PartType};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::debug;
use ulid::Ulid;

use crate::error::StorageError;
use crate::models::{Attachment, AttachmentSummary, Message, MessageSummary};

const ISO8601_FMT: &[time::format_description::BorrowedFormatItem<'_>] =
  format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// Repository for storing and querying captured email messages.
///
/// Wraps a [`SqlitePool`] and provides async methods for CRUD operations,
/// full-text search, retention enforcement, and attachment access.
#[derive(Clone)]
pub struct MessageRepository {
  pool: SqlitePool,
}

impl MessageRepository {
  /// Creates a new repository backed by the given connection pool.
  pub fn new(pool: SqlitePool) -> Self {
    Self { pool }
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
    let id = Ulid::new().to_string();
    let recipients_json = serde_json::to_string(recipients).unwrap_or_default();
    let size = raw.len() as i64;
    let now = OffsetDateTime::now_utc()
      .format(ISO8601_FMT)
      .unwrap_or_default();

    let parsed = MessageParser::default().parse(raw);

    let (subject, text_body, html_body, has_attachments) = match &parsed {
      Some(msg) => (
        msg.subject().map(String::from),
        msg.body_text(0).map(|s| s.into_owned()),
        msg.body_html(0).map(|s| s.into_owned()),
        msg.attachment_count() > 0,
      ),
      None => (None, None, None, false),
    };

    let mut txn = self.pool.begin().await?;

    sqlx::query(
      r#"
      INSERT INTO messages (id, sender, recipients, subject, text_body, html_body, raw, size, has_attachments, is_read, is_starred, tags, created_at)
      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, '[]', ?10)
      "#,
    )
    .bind(&id)
    .bind(sender)
    .bind(&recipients_json)
    .bind(&subject)
    .bind(&text_body)
    .bind(&html_body)
    .bind(raw)
    .bind(size)
    .bind(has_attachments)
    .bind(&now)
    .execute(&mut *txn)
    .await?;

    sqlx::query(
      "INSERT INTO messages_fts(rowid, subject, text_body, sender, recipients) SELECT rowid, ?2, ?3, ?4, ?5 FROM messages WHERE id = ?1",
    )
    .bind(&id)
    .bind(&subject)
    .bind(&text_body)
    .bind(sender)
    .bind(&recipients_json)
    .execute(&mut *txn)
    .await?;

    if let Some(parsed_msg) = &parsed {
      let attachment_ids: std::collections::HashSet<u32> =
        parsed_msg.attachments.iter().copied().collect();

      for (idx, part) in parsed_msg.parts.iter().enumerate() {
        let is_attachment = attachment_ids.contains(&(idx as u32));
        let cid = part.content_id().map(String::from);
        let is_inline_binary = matches!(part.body, PartType::InlineBinary(_));

        if !is_attachment && !is_inline_binary {
          continue;
        }

        let content = part.contents();
        if content.is_empty() {
          continue;
        }

        let att_id = Ulid::new().to_string();
        let filename = part.attachment_name().map(String::from);
        let content_type =
          part
            .content_type()
            .map(|ct: &mail_parser::ContentType| match ct.subtype() {
              Some(subtype) => format!("{}/{}", ct.ctype(), subtype),
              None => ct.ctype().to_string(),
            });
        let att_size = content.len() as i64;

        sqlx::query(
          r#"
          INSERT INTO attachments (id, message_id, filename, content_type, content_id, size, content)
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
          "#,
        )
        .bind(&att_id)
        .bind(&id)
        .bind(&filename)
        .bind(&content_type)
        .bind(&cid)
        .bind(att_size)
        .bind(content)
        .execute(&mut *txn)
        .await?;
      }
    }

    txn.commit().await?;

    debug!(id = %id, subject = ?subject, "Message stored");

    Ok(MessageSummary {
      id,
      sender: sender.to_string(),
      recipients: recipients_json,
      subject,
      size,
      has_attachments,
      is_read: false,
      is_starred: false,
      tags: "[]".to_string(),
      created_at: now,
    })
  }

  /// Lists messages ordered by newest first, with pagination.
  pub async fn list(&self, limit: i64, offset: i64) -> Result<Vec<MessageSummary>, StorageError> {
    let messages = sqlx::query_as::<_, MessageSummary>(
      "SELECT id, sender, recipients, subject, size, has_attachments, is_read, is_starred, tags, created_at FROM messages ORDER BY id DESC LIMIT ?1 OFFSET ?2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&self.pool)
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
    let quoted = match Self::sanitize_fts_query(query) {
      Some(q) => q,
      None => return Ok(Vec::new()),
    };
    let messages = sqlx::query_as::<_, MessageSummary>(
      r#"
      SELECT m.id, m.sender, m.recipients, m.subject, m.size, m.has_attachments, m.is_read, m.is_starred, m.tags, m.created_at
      FROM messages m
      INNER JOIN messages_fts fts ON m.rowid = fts.rowid
      WHERE messages_fts MATCH ?1
      ORDER BY m.id DESC
      LIMIT ?2 OFFSET ?3
      "#,
    )
    .bind(&quoted)
    .bind(limit)
    .bind(offset)
    .fetch_all(&self.pool)
    .await?;

    Ok(messages)
  }

  /// Counts the total number of FTS5 search matches.
  pub async fn search_count(&self, query: &str) -> Result<i64, StorageError> {
    let quoted = match Self::sanitize_fts_query(query) {
      Some(q) => q,
      None => return Ok(0),
    };
    let row: (i64,) = sqlx::query_as(
      r#"
      SELECT COUNT(*)
      FROM messages m
      INNER JOIN messages_fts fts ON m.rowid = fts.rowid
      WHERE messages_fts MATCH ?1
      "#,
    )
    .bind(&quoted)
    .fetch_one(&self.pool)
    .await?;
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

  /// Fetches a single message by ID, including bodies and raw bytes.
  pub async fn get(&self, id: &str) -> Result<Message, StorageError> {
    let message = sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id = ?1")
      .bind(id)
      .fetch_optional(&self.pool)
      .await?
      .ok_or_else(|| StorageError::NotFound(id.to_string()))?;

    Ok(message)
  }

  /// Atomically applies one or more metadata updates to a message.
  ///
  /// Only fields that are `Some` are updated. Runs all UPDATEs inside a
  /// single transaction so partial application cannot occur.
  pub async fn update_message(
    &self,
    id: &str,
    is_read: Option<bool>,
    is_starred: Option<bool>,
    tags: Option<&[String]>,
  ) -> Result<(), StorageError> {
    let mut txn = self.pool.begin().await?;

    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM messages WHERE id = ?1")
      .bind(id)
      .fetch_optional(&mut *txn)
      .await?;
    if exists.is_none() {
      return Err(StorageError::NotFound(id.to_string()));
    }

    if let Some(is_read) = is_read {
      sqlx::query("UPDATE messages SET is_read = ?1 WHERE id = ?2")
        .bind(is_read)
        .bind(id)
        .execute(&mut *txn)
        .await?;
    }
    if let Some(is_starred) = is_starred {
      sqlx::query("UPDATE messages SET is_starred = ?1 WHERE id = ?2")
        .bind(is_starred)
        .bind(id)
        .execute(&mut *txn)
        .await?;
    }
    if let Some(tags) = tags {
      let tags_json = serde_json::to_string(tags).unwrap_or_default();
      sqlx::query("UPDATE messages SET tags = ?1 WHERE id = ?2")
        .bind(&tags_json)
        .bind(id)
        .execute(&mut *txn)
        .await?;
    }

    txn.commit().await?;
    Ok(())
  }

  /// Deletes a single message and its FTS5 index entry atomically.
  pub async fn delete(&self, id: &str) -> Result<(), StorageError> {
    let mut txn = self.pool.begin().await?;

    sqlx::query(
      "DELETE FROM messages_fts WHERE rowid = (SELECT rowid FROM messages WHERE id = ?1)",
    )
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
  pub async fn delete_all(&self) -> Result<u64, StorageError> {
    let mut txn = self.pool.begin().await?;

    let result = sqlx::query("DELETE FROM messages")
      .execute(&mut *txn)
      .await?;
    sqlx::query("DELETE FROM messages_fts")
      .execute(&mut *txn)
      .await?;

    txn.commit().await?;
    Ok(result.rows_affected())
  }

  /// Returns the total number of stored messages.
  pub async fn count(&self) -> Result<i64, StorageError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
      .fetch_one(&self.pool)
      .await?;
    Ok(row.0)
  }

  /// Counts messages matching optional subject, sender, and recipient filters (case-insensitive).
  pub async fn count_matching(
    &self,
    subject: Option<&str>,
    sender: Option<&str>,
    recipient: Option<&str>,
  ) -> Result<i64, StorageError> {
    let mut sql = String::from("SELECT COUNT(*) FROM messages WHERE 1=1");
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = subject {
      sql.push_str(" AND LOWER(subject) LIKE ? ESCAPE '\\'");
      binds.push(format!("%{}%", escape_like(&s.to_lowercase())));
    }
    if let Some(s) = sender {
      sql.push_str(" AND LOWER(sender) LIKE ? ESCAPE '\\'");
      binds.push(format!("%{}%", escape_like(&s.to_lowercase())));
    }
    if let Some(r) = recipient {
      sql.push_str(" AND LOWER(recipients) LIKE ? ESCAPE '\\'");
      binds.push(format!("%{}%", escape_like(&r.to_lowercase())));
    }

    let mut query = sqlx::query_as::<_, (i64,)>(&sql);
    for b in &binds {
      query = query.bind(b);
    }

    let row = query.fetch_one(&self.pool).await?;
    Ok(row.0)
  }

  /// Lists all attachments for a given message (metadata only, no binary content).
  pub async fn get_attachments(
    &self,
    message_id: &str,
  ) -> Result<Vec<AttachmentSummary>, StorageError> {
    let attachments = sqlx::query_as::<_, AttachmentSummary>(
      "SELECT id, message_id, filename, content_type, content_id, size FROM attachments WHERE message_id = ?1",
    )
    .bind(message_id)
    .fetch_all(&self.pool)
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
      "SELECT * FROM attachments WHERE id = ?1 AND message_id = ?2",
    )
    .bind(attachment_id)
    .bind(message_id)
    .fetch_optional(&self.pool)
    .await?
    .ok_or_else(|| StorageError::NotFound(attachment_id.to_string()))?;

    Ok(attachment)
  }

  /// Fetches a single attachment by Content-ID, scoped to its parent message.
  pub async fn get_attachment_by_content_id(
    &self,
    message_id: &str,
    content_id: &str,
  ) -> Result<Attachment, StorageError> {
    let attachment = sqlx::query_as::<_, Attachment>(
      "SELECT * FROM attachments WHERE content_id = ?1 AND message_id = ?2",
    )
    .bind(content_id)
    .bind(message_id)
    .fetch_optional(&self.pool)
    .await?
    .ok_or_else(|| StorageError::NotFound(content_id.to_string()))?;

    Ok(attachment)
  }

  /// Returns the raw RFC 5322 bytes for a message.
  pub async fn get_raw(&self, id: &str) -> Result<Vec<u8>, StorageError> {
    let row: (Vec<u8>,) = sqlx::query_as("SELECT raw FROM messages WHERE id = ?1")
      .bind(id)
      .fetch_optional(&self.pool)
      .await?
      .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    Ok(row.0)
  }

  /// Deletes messages older than the given ISO 8601 cutoff. Returns IDs of deleted messages.
  pub async fn delete_older_than(&self, iso_cutoff: &str) -> Result<Vec<String>, StorageError> {
    let mut txn = self.pool.begin().await?;

    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM messages WHERE created_at < ?1")
      .bind(iso_cutoff)
      .fetch_all(&mut *txn)
      .await?;

    if ids.is_empty() {
      return Ok(Vec::new());
    }

    sqlx::query(
      "DELETE FROM messages_fts WHERE rowid IN (SELECT rowid FROM messages WHERE created_at < ?1)",
    )
    .bind(iso_cutoff)
    .execute(&mut *txn)
    .await?;

    sqlx::query("DELETE FROM messages WHERE created_at < ?1")
      .bind(iso_cutoff)
      .execute(&mut *txn)
      .await?;

    txn.commit().await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
  }

  /// Trims stored messages to at most `max`, deleting oldest first. Returns IDs of deleted messages.
  pub async fn trim_to_max(&self, max: i64) -> Result<Vec<String>, StorageError> {
    let mut txn = self.pool.begin().await?;

    let ids: Vec<(String,)> =
      sqlx::query_as("SELECT id FROM messages ORDER BY id DESC LIMIT -1 OFFSET ?1")
        .bind(max)
        .fetch_all(&mut *txn)
        .await?;

    if ids.is_empty() {
      return Ok(Vec::new());
    }

    sqlx::query(
      "DELETE FROM messages_fts WHERE rowid IN (SELECT rowid FROM messages ORDER BY id DESC LIMIT -1 OFFSET ?1)",
    )
    .bind(max)
    .execute(&mut *txn)
    .await?;

    sqlx::query(
      "DELETE FROM messages WHERE id IN (SELECT id FROM messages ORDER BY id DESC LIMIT -1 OFFSET ?1)",
    )
    .bind(max)
    .execute(&mut *txn)
    .await?;

    txn.commit().await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
  }
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
  use crate::initialize_database;

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
    assert_eq!(msg.raw, raw);
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
    let dir = std::env::temp_dir().join(format!("rustmail-test-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
      .max_connections(8)
      .connect(&url)
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
    assert_eq!(ids.len(), 32, "ULIDs must be unique under concurrent inserts");
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

    // FTS and rows must stay in sync — a stale FTS entry would show up in a
    // search for a row that no longer exists in the messages table.
    let remaining = repo.count().await.unwrap();
    let search_hits = repo.search("writer OR seed", 100, 0).await.unwrap();
    assert_eq!(
      search_hits.len() as i64,
      remaining,
      "FTS5 rowid set must match messages table ({} hits vs {} rows)",
      search_hits.len(),
      remaining
    );
  }
}
