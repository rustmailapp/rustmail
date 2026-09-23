//! Identifying a legacy database's contents without reading its bodies.

use sqlx::SqliteConnection;

use crate::StorageError;

/// Rows one fingerprint query reads before the next page is asked for.
const FINGERPRINT_PAGE_ROWS: i64 = 10_000;
/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Marks an absent value in the hashed stream, so `NULL` and `""` differ.
const NULL_MARKER: u8 = 0;
/// Marks a present value in the hashed stream.
const VALUE_MARKER: u8 = 1;

/// FNV-1a over a stream of typed fields, stable across Rust versions.
struct Fnv1a(u64);

impl Fnv1a {
  fn new() -> Self {
    Self(FNV_OFFSET_BASIS)
  }

  fn bytes(&mut self, bytes: &[u8]) {
    for &byte in bytes {
      self.0 ^= u64::from(byte);
      self.0 = self.0.wrapping_mul(FNV_PRIME);
    }
  }

  fn int(&mut self, value: i64) {
    self.bytes(&value.to_le_bytes());
  }

  fn text(&mut self, value: Option<&str>) {
    match value {
      Some(text) => {
        self.bytes(&[VALUE_MARKER]);
        self.int(i64::try_from(text.len()).unwrap_or(i64::MAX));
        self.bytes(text.as_bytes());
      }
      None => self.bytes(&[NULL_MARKER]),
    }
  }

  fn opt_int(&mut self, value: Option<i64>) {
    match value {
      Some(value) => {
        self.bytes(&[VALUE_MARKER]);
        self.int(value);
      }
      None => self.bytes(&[NULL_MARKER]),
    }
  }
}

/// How the columns later rustmail versions added read in this legacy file.
///
/// Files older than the columns hold none of them; they read as the defaults
/// those versions gave every existing row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LegacyColumns {
  /// SQL for `is_starred`: the column, or its default.
  pub(crate) is_starred: &'static str,
  /// SQL for `tags`: the column, or its default.
  pub(crate) tags: &'static str,
  /// Whether the file has an `attachments` table at all.
  pub(crate) has_attachments_table: bool,
}

impl LegacyColumns {
  /// Reads which optional columns the legacy file in `schema` holds.
  pub(crate) async fn probe(
    conn: &mut SqliteConnection,
    schema: &str,
  ) -> Result<Self, StorageError> {
    let columns: Vec<String> =
      sqlx::query_scalar("SELECT name FROM pragma_table_info('messages', ?1)")
        .bind(schema)
        .fetch_all(&mut *conn)
        .await?;
    let has = |name: &str| columns.iter().any(|column| column == name);
    let has_attachments_table: bool = sqlx::query_scalar(
      "SELECT EXISTS(SELECT 1 FROM pragma_table_list WHERE schema = ?1 AND name = 'attachments' AND type = 'table')",
    )
    .bind(schema)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Self {
      is_starred: if has("is_starred") { "is_starred" } else { "0" },
      tags: if has("tags") { "tags" } else { "'[]'" },
      has_attachments_table,
    })
  }
}

/// What the fingerprint scan learned about a legacy database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourcePrint {
  /// FNV-1a 64 over every message's and attachment's identifying fields, as
  /// 16 hex digits.
  pub(crate) fingerprint: String,
  /// Messages in the file.
  pub(crate) messages: u64,
  /// Attachment rows in the file.
  pub(crate) attachments: u64,
  /// Total length of every message's raw source.
  pub(crate) raw_bytes: u64,
}

/// `(rowid, id, is_read, is_starred, tags, length(raw))` of a legacy message.
type MessageKey = (
  i64,
  Option<String>,
  Option<i64>,
  Option<i64>,
  Option<String>,
  Option<i64>,
);
/// `(rowid, id, message_id, length(content))` of a legacy attachment.
type AttachmentKey = (i64, Option<String>, Option<String>, Option<i64>);

/// Fingerprints the legacy database that is `main` on `conn`.
///
/// Hashes, in rowid order, `(rowid, id, is_read, is_starred, tags,
/// length(raw))` of every message and `(rowid, id, message_id,
/// length(content))` of every attachment, then both counts. Anything a
/// legacy rustmail writes changes one of those: a new or deleted message, a
/// flag or tag edit. `length()` reads a blob's size from its row header, so
/// the scan never loads a body.
///
/// Run it inside the transaction that keeps writers out, or the result may
/// describe no single state of the file.
pub(crate) async fn fingerprint(
  conn: &mut SqliteConnection,
  columns: &LegacyColumns,
) -> Result<SourcePrint, StorageError> {
  let mut hash = Fnv1a::new();
  let mut messages: u64 = 0;
  let mut raw_bytes: u64 = 0;
  let message_page = format!(
    "SELECT rowid, id, is_read, {}, {}, length(raw) FROM main.messages WHERE rowid > ?1 ORDER BY rowid LIMIT ?2",
    columns.is_starred, columns.tags
  );
  let mut after = i64::MIN;
  loop {
    let rows: Vec<MessageKey> = sqlx::query_as(&message_page)
      .bind(after)
      .bind(FINGERPRINT_PAGE_ROWS)
      .fetch_all(&mut *conn)
      .await?;
    let Some(last) = rows.last() else {
      break;
    };
    after = last.0;
    for (rowid, id, is_read, is_starred, tags, raw_len) in &rows {
      hash.int(*rowid);
      hash.text(id.as_deref());
      hash.opt_int(*is_read);
      hash.opt_int(*is_starred);
      hash.text(tags.as_deref());
      hash.opt_int(*raw_len);
      raw_bytes += raw_len.and_then(|len| u64::try_from(len).ok()).unwrap_or(0);
    }
    messages += rows.len() as u64;
  }

  let mut attachments: u64 = 0;
  if columns.has_attachments_table {
    let mut after = i64::MIN;
    loop {
      let rows: Vec<AttachmentKey> = sqlx::query_as(
        "SELECT rowid, id, message_id, length(content) FROM main.attachments WHERE rowid > ?1 ORDER BY rowid LIMIT ?2",
      )
      .bind(after)
      .bind(FINGERPRINT_PAGE_ROWS)
      .fetch_all(&mut *conn)
      .await?;
      let Some(last) = rows.last() else {
        break;
      };
      after = last.0;
      for (rowid, id, message_id, content_len) in &rows {
        hash.int(*rowid);
        hash.text(id.as_deref());
        hash.text(message_id.as_deref());
        hash.opt_int(*content_len);
      }
      attachments += rows.len() as u64;
    }
  }

  hash.int(i64::try_from(messages).unwrap_or(i64::MAX));
  hash.int(i64::try_from(attachments).unwrap_or(i64::MAX));
  Ok(SourcePrint {
    fingerprint: format!("{:016x}", hash.0),
    messages,
    attachments,
    raw_bytes,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fnv1a_matches_the_reference_vectors() {
    let mut empty = Fnv1a::new();
    empty.bytes(b"");
    assert_eq!(empty.0, 0xcbf2_9ce4_8422_2325);
    let mut a = Fnv1a::new();
    a.bytes(b"a");
    assert_eq!(a.0, 0xaf63_dc4c_8601_ec8c);
    let mut foobar = Fnv1a::new();
    foobar.bytes(b"foobar");
    assert_eq!(foobar.0, 0x8594_4171_f739_67e8);
  }

  #[test]
  fn a_null_and_an_empty_text_hash_differently() {
    let mut null = Fnv1a::new();
    null.text(None);
    let mut empty = Fnv1a::new();
    empty.text(Some(""));
    assert_ne!(null.0, empty.0);
  }
}
