//! Giving the pages freed by large deletes back to the filesystem.
//!
//! Schema-1 files use `auto_vacuum=INCREMENTAL`, so a delete only moves pages
//! onto the freelist. [`reclaim`] releases them in bounded steps on the
//! writer, then truncates the WAL so the file really shrinks.

use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tracing::{debug, info};

use crate::StorageError;
use crate::repo::{is_retryable_lock, retry_on_lock};

/// Pages one `incremental_vacuum` statement may release.
///
/// Each step is its own statement on the writer, so an insert waiting for the
/// writer runs between two steps instead of after the whole reclaim. At the
/// default 4 KiB page this is 4 MiB per step.
const RECLAIM_STEP_PAGES: i64 = 1024;
/// Time the reclaim after a delete-all may spend releasing pages.
///
/// The request that asked for the delete waits for it, so it is bounded; an
/// emptied mailbox frees everything well inside it.
pub(crate) const DELETE_ALL_RECLAIM_BUDGET: Duration = Duration::from_secs(10);
/// Time the reclaim after a retention sweep may spend releasing pages.
///
/// Whatever it leaves on the freelist is picked up by a later sweep once the
/// threshold is crossed again.
pub(crate) const RETENTION_RECLAIM_BUDGET: Duration = Duration::from_secs(2);
/// Free bytes below which a retention sweep never reclaims.
const RETENTION_MIN_FREE_BYTES: i64 = 64 * 1024 * 1024;
/// Share of the file, in percent, the freelist must exceed before a retention
/// sweep reclaims.
const RETENTION_MIN_FREE_PERCENT: i64 = 25;
/// Freed bytes above which a reclaim is logged at `info` rather than `debug`.
const NOTABLE_RECLAIM_BYTES: i64 = 64 * 1024 * 1024;
const PERCENT: i64 = 100;

/// What asked for a reclaim, as reported in its log line.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ReclaimTrigger {
  DeleteAll,
  Retention,
}

impl ReclaimTrigger {
  fn as_str(self) -> &'static str {
    match self {
      Self::DeleteAll => "delete_all",
      Self::Retention => "retention",
    }
  }
}

/// Page counts of the main database, read in one statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct PageStats {
  page_size: i64,
  page_count: i64,
  freelist_count: i64,
}

impl PageStats {
  fn free_bytes(self) -> i64 {
    self.freelist_count.saturating_mul(self.page_size)
  }

  fn file_bytes(self) -> i64 {
    self.page_count.saturating_mul(self.page_size)
  }

  /// Whether a retention sweep has freed enough to be worth reclaiming: more
  /// than the larger of [`RETENTION_MIN_FREE_BYTES`] and
  /// [`RETENTION_MIN_FREE_PERCENT`] of the file.
  ///
  /// Below it the free pages are left for new mail to reuse, which costs
  /// nothing, where reclaiming them would move pages only for the file to
  /// grow straight back.
  pub(crate) fn warrants_retention_reclaim(self) -> bool {
    let free = self.free_bytes();
    let past_share_of_file =
      free.saturating_mul(PERCENT) > self.file_bytes().saturating_mul(RETENTION_MIN_FREE_PERCENT);
    free > RETENTION_MIN_FREE_BYTES && past_share_of_file
  }
}

/// Reads the page size, page count and freelist length of the main database.
pub(crate) async fn page_stats(pool: &SqlitePool) -> Result<PageStats, StorageError> {
  Ok(
    sqlx::query_as(
      "SELECT s.page_size, c.page_count, f.freelist_count
       FROM pragma_page_size() s, pragma_page_count() c, pragma_freelist_count() f",
    )
    .fetch_one(pool)
    .await?,
  )
}

/// Releases free pages in steps of [`RECLAIM_STEP_PAGES`] until the freelist
/// is empty, a step frees nothing, or `budget` runs out, then checkpoints the
/// WAL with `TRUNCATE` and logs `event=storage_reclaim`.
///
/// No step holds a transaction open, so a waiting insert commits between two
/// steps. A contended step is retried like any other write. A checkpoint that
/// cannot finish because readers still hold an older snapshot is reported as
/// `wal_checkpoint_busy` rather than failing: the database pages are already
/// free, and the next checkpoint truncates the file.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if a step or the checkpoint fails for
/// any reason other than contention.
pub(crate) async fn reclaim(
  writer: &SqlitePool,
  trigger: ReclaimTrigger,
  budget: Duration,
) -> Result<(), StorageError> {
  let started = Instant::now();
  let deadline = started + budget;
  let before = page_stats(writer).await?;
  let mut free_pages = before.freelist_count;
  while free_pages > 0 && Instant::now() < deadline {
    let left = retry_on_lock(|| vacuum_step(writer)).await?;
    let progressed = left < free_pages;
    free_pages = left;
    if !progressed {
      break;
    }
  }
  let wal_checkpoint_busy = checkpoint_truncate(writer).await?;
  let freed_pages = before.freelist_count.saturating_sub(free_pages).max(0);
  let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
  if freed_pages.saturating_mul(before.page_size) > NOTABLE_RECLAIM_BYTES {
    info!(
      event = "storage_reclaim",
      trigger = trigger.as_str(),
      freed_pages,
      free_pages_left = free_pages,
      duration_ms,
      wal_checkpoint_busy,
      "reclaimed free pages"
    );
  } else {
    debug!(
      event = "storage_reclaim",
      trigger = trigger.as_str(),
      freed_pages,
      free_pages_left = free_pages,
      duration_ms,
      wal_checkpoint_busy,
      "reclaimed free pages"
    );
  }
  Ok(())
}

/// Runs one bounded `incremental_vacuum` and returns the freelist left after
/// it, both on the same connection and outside any transaction.
async fn vacuum_step(writer: &SqlitePool) -> Result<i64, StorageError> {
  let mut conn = writer.acquire().await?;
  sqlx::query(&format!("PRAGMA incremental_vacuum({RECLAIM_STEP_PAGES})"))
    .execute(&mut *conn)
    .await?;
  Ok(
    sqlx::query_scalar("PRAGMA freelist_count")
      .fetch_one(&mut *conn)
      .await?,
  )
}

/// Checkpoints the WAL and truncates it, returning whether readers kept the
/// checkpoint from completing.
async fn checkpoint_truncate(writer: &SqlitePool) -> Result<bool, StorageError> {
  let result: Result<(i64, i64, i64), StorageError> =
    sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
      .fetch_one(writer)
      .await
      .map_err(StorageError::from);
  match result {
    Ok((busy, _, _)) => Ok(busy != 0),
    Err(error) if is_retryable_lock(&error) => Ok(true),
    Err(error) => Err(error),
  }
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use sqlx::sqlite::SqlitePoolOptions;
  use ulid::Ulid;

  use super::*;
  use crate::{MessageRepository, PreparedMessage, connect_options, initialize_database};

  const MIB: i64 = 1024 * 1024;
  const PAGE: i64 = 4096;
  const READER_CONNECTIONS: u32 = 4;
  const MAX_FILE_AFTER_DELETE_ALL: u64 = 5 * 1024 * 1024;
  const SMALL_MESSAGES: usize = 3000;
  const FEW_SMALL_MESSAGES: usize = 1000;
  const SMALL_BODY_BYTES: usize = 8 * 1024;
  const LARGE_MESSAGES: usize = 80;
  const LARGE_ATTACHMENT_BYTES: usize = 1024 * 1024;
  const CONCURRENT_INSERTS: usize = 32;
  const BATCH: usize = 500;
  const FUTURE_CUTOFF: &str = "9999-12-31T23:59:59Z";
  const IN_MEMORY_URL: &str = "sqlite::memory:";

  struct TempDir(PathBuf);
  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  struct FileRepo {
    _dir: TempDir,
    path: PathBuf,
    repo: MessageRepository,
    writer: SqlitePool,
  }

  fn stats(page_count: i64, freelist_count: i64) -> PageStats {
    PageStats {
      page_size: PAGE,
      page_count,
      freelist_count,
    }
  }

  async fn file_repo(label: &str) -> FileRepo {
    let dir = std::env::temp_dir().join(format!("rustmail-reclaim-{label}-{}", Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mail.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let writer = SqlitePoolOptions::new()
      .min_connections(1)
      .max_connections(1)
      .connect_with(connect_options(&url).unwrap())
      .await
      .unwrap();
    initialize_database(&writer).await.unwrap();
    let readers = SqlitePoolOptions::new()
      .max_connections(READER_CONNECTIONS)
      .connect_with(connect_options(&url).unwrap())
      .await
      .unwrap();
    FileRepo {
      _dir: TempDir(dir),
      path,
      repo: MessageRepository::with_writer(readers, writer.clone()),
      writer,
    }
  }

  fn on_disk_bytes(path: &Path) -> u64 {
    [
      path.to_path_buf(),
      PathBuf::from(format!("{}-wal", path.display())),
    ]
    .iter()
    .filter_map(|file| std::fs::metadata(file).ok())
    .map(|meta| meta.len())
    .sum()
  }

  fn small_message(index: usize) -> PreparedMessage {
    let body = "lorem ipsum dolor sit amet ".repeat(SMALL_BODY_BYTES / 27);
    PreparedMessage::parse(
      "a@test.com".to_string(),
      &["b@test.com".into()],
      format!("From: a@test.com\r\nTo: b@test.com\r\nSubject: small {index}\r\n\r\n{body}")
        .into_bytes(),
    )
  }

  fn large_message(index: usize) -> PreparedMessage {
    let line = format!("{index:0>76}\r\n");
    let payload = line.repeat(LARGE_ATTACHMENT_BYTES / line.len());
    PreparedMessage::parse(
      "a@test.com".to_string(),
      &["b@test.com".into()],
      format!(
        concat!(
          "From: a@test.com\r\nTo: b@test.com\r\nSubject: large {index}\r\n",
          "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n",
          "--B\r\nContent-Type: text/plain\r\n\r\nsee attached\r\n",
          "--B\r\nContent-Type: application/octet-stream\r\n",
          "Content-Disposition: attachment; filename=\"blob.bin\"\r\n\r\n",
          "{payload}--B--\r\n"
        ),
        index = index,
        payload = payload
      )
      .into_bytes(),
    )
  }

  async fn fill(repo: &MessageRepository, messages: Vec<PreparedMessage>) {
    for batch in messages.chunks(BATCH) {
      repo.insert_batch(batch).await.unwrap();
    }
  }

  #[test]
  fn retention_does_not_reclaim_up_to_64_mib_free_in_a_small_file() {
    let file_pages = 100 * MIB / PAGE;
    assert!(!stats(file_pages, 64 * MIB / PAGE).warrants_retention_reclaim());
  }

  #[test]
  fn retention_reclaims_past_64_mib_free_in_a_small_file() {
    let file_pages = 100 * MIB / PAGE;
    assert!(stats(file_pages, 65 * MIB / PAGE).warrants_retention_reclaim());
  }

  #[test]
  fn retention_does_not_reclaim_up_to_a_quarter_of_a_large_file() {
    let file_pages = 1024 * MIB / PAGE;
    assert!(!stats(file_pages, 256 * MIB / PAGE).warrants_retention_reclaim());
  }

  #[test]
  fn retention_reclaims_past_a_quarter_of_a_large_file() {
    let file_pages = 1024 * MIB / PAGE;
    assert!(stats(file_pages, 257 * MIB / PAGE).warrants_retention_reclaim());
  }

  #[tokio::test]
  async fn delete_all_leaves_no_free_pages_and_a_small_file() {
    let file = file_repo("delete-all").await;
    fill(&file.repo, (0..SMALL_MESSAGES).map(small_message).collect()).await;
    let populated = on_disk_bytes(&file.path);
    assert!(
      populated > MAX_FILE_AFTER_DELETE_ALL * 4,
      "the fixture is too small to show a reclaim: {populated} bytes"
    );

    let deleted = file.repo.delete_all().await.unwrap();

    assert_eq!(deleted, SMALL_MESSAGES as u64);
    assert_eq!(page_stats(&file.writer).await.unwrap().freelist_count, 0);
    let emptied = on_disk_bytes(&file.path);
    assert!(
      emptied <= MAX_FILE_AFTER_DELETE_ALL,
      "{emptied} bytes left on disk after delete-all (from {populated})"
    );
  }

  #[tokio::test]
  async fn retention_below_the_threshold_leaves_the_free_pages_in_place() {
    let file = file_repo("retention-below").await;
    fill(
      &file.repo,
      (0..FEW_SMALL_MESSAGES).map(small_message).collect(),
    )
    .await;
    file.repo.delete_older_than(FUTURE_CUTOFF).await.unwrap();
    let swept = page_stats(&file.writer).await.unwrap();
    assert!(swept.freelist_count > 0);
    assert!(!swept.warrants_retention_reclaim());

    file.repo.reclaim_after_retention().await.unwrap();

    assert_eq!(page_stats(&file.writer).await.unwrap(), swept);
  }

  #[tokio::test]
  async fn retention_above_the_threshold_reclaims_the_free_pages() {
    let file = file_repo("retention-above").await;
    fill(&file.repo, (0..LARGE_MESSAGES).map(large_message).collect()).await;
    file.repo.delete_older_than(FUTURE_CUTOFF).await.unwrap();
    let swept = page_stats(&file.writer).await.unwrap();
    assert!(
      swept.warrants_retention_reclaim(),
      "the fixture freed too little to cross the threshold: {swept:?}"
    );

    file.repo.reclaim_after_retention().await.unwrap();

    let reclaimed = page_stats(&file.writer).await.unwrap();
    assert_eq!(reclaimed.freelist_count, 0);
    assert!(reclaimed.page_count < swept.page_count - swept.freelist_count / 2);
  }

  #[tokio::test]
  async fn an_insert_concurrent_with_a_reclaim_is_stored() {
    let file = file_repo("concurrent").await;
    fill(&file.repo, (0..SMALL_MESSAGES).map(small_message).collect()).await;
    file.repo.delete_older_than(FUTURE_CUTOFF).await.unwrap();
    assert!(page_stats(&file.writer).await.unwrap().freelist_count > RECLAIM_STEP_PAGES);

    let writer = file.writer.clone();
    let reclaiming = tokio::spawn(async move {
      reclaim(
        &writer,
        ReclaimTrigger::DeleteAll,
        DELETE_ALL_RECLAIM_BUDGET,
      )
      .await
    });
    let mut inserts = Vec::new();
    for index in 0..CONCURRENT_INSERTS {
      let repo = file.repo.clone();
      inserts.push(tokio::spawn(async move {
        repo.insert_prepared(&small_message(index)).await
      }));
    }

    let mut stored = Vec::new();
    for insert in inserts {
      stored.push(insert.await.unwrap().unwrap().id);
    }
    reclaiming.await.unwrap().unwrap();

    assert_eq!(file.repo.count().await.unwrap(), CONCURRENT_INSERTS as i64);
    for id in stored {
      assert_eq!(file.repo.get(&id).await.unwrap().id, id);
    }
  }

  #[tokio::test]
  async fn delete_all_shrinks_an_in_memory_database() {
    let pool = SqlitePoolOptions::new()
      .min_connections(1)
      .max_connections(1)
      .connect_with(connect_options(IN_MEMORY_URL).unwrap())
      .await
      .unwrap();
    initialize_database(&pool).await.unwrap();
    let repo = MessageRepository::new(pool.clone());
    fill(&repo, (0..SMALL_MESSAGES).map(small_message).collect()).await;
    let populated = page_stats(&pool).await.unwrap();

    repo.delete_all().await.unwrap();

    let emptied = page_stats(&pool).await.unwrap();
    assert_eq!(emptied.freelist_count, 0);
    assert!(
      emptied.page_count < populated.page_count / 10,
      "in-memory page count went from {} to {}",
      populated.page_count,
      emptied.page_count
    );
  }
}
