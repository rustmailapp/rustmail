use sqlx::{QueryBuilder, Sqlite};

/// Narrows a listing to the messages that pass every condition set here.
///
/// An unset field does not filter. Set fields combine with `AND`, and with a
/// search query when there is one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFilter {
  /// Only starred messages.
  pub starred: bool,
  /// Only messages not yet marked as read.
  pub unread: bool,
  /// Only messages carrying at least one attachment.
  pub has_attachments: bool,
  /// Only messages carrying at least one of these tags, compared exactly.
  pub tags: Vec<String>,
}

impl MessageFilter {
  /// Whether this filter lets every message through.
  pub fn is_empty(&self) -> bool {
    *self == Self::default()
  }
}

/// A stored message's position in listing order, for keyset pagination.
///
/// Obtained from [`MessageRepository::cursor`](crate::MessageRepository::cursor),
/// so a page that starts from it costs the same however deep it sits, where
/// an offset makes SQLite step over every row in front of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor(pub(crate) i64);

/// Where a page of results begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageStart {
  /// Skip this many matching messages, newest first.
  Offset(i64),
  /// Start with the newest message older than the cursor's.
  Before(Cursor),
}

/// Appends one `AND` condition per set field of `filter`.
///
/// `alias` is the `messages` table alias of the statement being built.
pub(crate) fn push_filter(
  builder: &mut QueryBuilder<'_, Sqlite>,
  alias: &'static str,
  filter: &MessageFilter,
) {
  if filter.starred {
    builder.push(format_args!(" AND {alias}.is_starred = 1"));
  }
  if filter.unread {
    builder.push(format_args!(" AND {alias}.is_read = 0"));
  }
  if filter.has_attachments {
    builder.push(format_args!(" AND {alias}.has_attachments = 1"));
  }
  if !filter.tags.is_empty() {
    builder.push(format_args!(
      " AND EXISTS (SELECT 1 FROM json_each({alias}.tags) WHERE json_each.value IN ("
    ));
    let mut tags = builder.separated(", ");
    for tag in &filter.tags {
      tags.push_bind(tag.clone());
    }
    builder.push("))");
  }
}

/// Appends the cursor bound, newest-first ordering and page window.
///
/// `rowid` names the column that carries arrival order in the statement.
pub(crate) fn push_page(
  builder: &mut QueryBuilder<'_, Sqlite>,
  rowid: &'static str,
  start: PageStart,
  limit: i64,
) {
  if let PageStart::Before(Cursor(bound)) = start {
    builder.push(format_args!(" AND {rowid} < "));
    builder.push_bind(bound);
  }
  builder.push(format_args!(" ORDER BY {rowid} DESC LIMIT "));
  builder.push_bind(limit);
  if let PageStart::Offset(offset) = start {
    builder.push(" OFFSET ");
    builder.push_bind(offset);
  }
}
