# Upgrading

RustMail 0.8 stores messages in a new database layout (schema 1). The first time the new version starts on a database written by RustMail 0.7 or earlier, it upgrades that database automatically. There is nothing to run by hand, and no API changes.

This page covers what happens during that upgrade, where the backup goes, and how to go back.

## What happens on first start

Before the SMTP and HTTP listeners start, RustMail:

1. Takes a lock file next to the database, `rustmail.db.migration-lock`.
2. Builds the new database beside the old one, as `rustmail.db.migrating`, copying messages in batches.
3. Verifies the copy against the old database (counts, search index, integrity checks).
4. Swaps the files: the old database becomes `rustmail.db.schema0.bak` and the copy becomes `rustmail.db`.

The old database is only read, never modified. If a check fails, RustMail exits with an error, keeps `rustmail.db.migrating` for a bug report, and leaves `rustmail.db` as it was.

The log shows progress while it runs. Every line carries `event="storage_migration"` and a `phase`:

| Phase | Message |
|---|---|
| `start` | `Migrating the database to the current storage schema; SMTP and HTTP start once it is done`, with the message count and database size |
| `copy` | `Storage migration in progress`, every 2 seconds, with `migrated`, `total`, `rate_per_s` and `eta_s` |
| `verify` | `Verifying the migrated copy` |
| `done` | `Storage migration complete; the previous database is kept as the backup`, with `duration_s` and `backup_path` |
| `failed` | `Storage migration failed`, with the `error` and a `hint` |

### How long it takes

Expect about **4 seconds per 100,000 small messages** on a recent machine, growing linearly with the mailbox. Large messages with attachments cost more per message. Under Docker Desktop, which runs containers in a VM, expect it to take a few times longer.

SMTP and HTTP are not available until the upgrade finishes. Mail sent to RustMail in the meantime is refused at connection time, so senders that retry will deliver it afterwards.

### Stopping and resuming

You can stop RustMail during the upgrade (`Ctrl+C` or `SIGTERM`). It finishes the batch in progress, logs `Storage migration paused at N/T; it resumes on next start`, and exits. The next start picks up where it stopped. If the old database was changed in the meantime (for example by an older RustMail), the copy is rebuilt from scratch.

### Disk space

During the upgrade both files exist side by side, so you need room for the old database **plus** the new one. The new file is usually no larger than the old one, since binary attachments are no longer stored twice. If the disk fills, RustMail exits with:

```
migrating <db> needs about N MB free next to it (estimate); the existing database is untouched; free space and restart to resume
```

Free the space and start RustMail again; it resumes.

### A second process during the upgrade

Only one process can upgrade a database. A second RustMail started on the same file while the upgrade runs exits immediately with:

```
another rustmail process holds <db>.migration-lock (storage migration or restore in progress); wait for it to finish, its log shows progress, or stop it
```

The lock file stays next to the database after the upgrade. Leave it there; RustMail reuses it.

### Search results after the upgrade

The search index is rebuilt from the stored mail during the upgrade. Older databases could hold stale index entries for mail that had been deleted, so some searches can return different results or totals after the upgrade. The messages themselves are unchanged.

## The backup

The database from before the upgrade is kept next to the new one, with `.schema0.bak` appended to its name:

| Install | Backup location |
|---|---|
| macOS (default path, Homebrew) | `~/Library/Application Support/rustmail/rustmail.db.schema0.bak` |
| Linux (default path) | `~/.local/share/rustmail/rustmail.db.schema0.bak` |
| Docker | `/data/rustmail.db.schema0.bak` in the data volume |
| Custom `--db-path` / `RUSTMAIL_DB_PATH` | `<db-path>.schema0.bak` |

RustMail never deletes it. Every start logs its path and size (`event="storage_backup"`) as a reminder.

**When it is safe to delete:** once you have checked that your mail looks right in the new version and you do not plan to go back to 0.7 or earlier. After you delete it, `rustmail restore-backup` has nothing to restore.

## Going back to an older version

Older binaries cannot read the new layout. Restore the backup **with the new binary first**, then install the older version.

1. Stop every RustMail process using the database.
2. Run:

   ```sh
   rustmail restore-backup
   ```

   Pass `--db-path` (or set `RUSTMAIL_DB_PATH`) if you use a custom path; it resolves the database the same way `rustmail serve` does.

3. Install and start the older version.

`restore-backup` takes the same lock as the upgrade, so it refuses to run while another RustMail holds it, and it refuses while another process still has the database open. It renames the current database to `rustmail.db.schema1-<timestamp>` (it never deletes it) and moves `rustmail.db.schema0.bak` back to `rustmail.db`.

::: warning Mail received after the upgrade is not restored
The backup holds the mailbox as it was when you upgraded. Mail received after that, and any changes such as deletions or read flags, exist only in the schema-1 file (`rustmail.db.schema1-<timestamp>`). Starting the new version again on the restored file upgrades it again.
:::

### Docker

```sh
docker compose stop rustmail
docker run --rm -v rustmail-data:/data smyile/rustmail:<new-version> restore-backup
```

Then change the image tag in your Compose file to the older version and start it.

### If you downgrade without restoring

Older binaries refuse the upgraded file at startup and write nothing to it:

- RustMail 0.7.0 and earlier exit with an SQL error ending in `no such column: message_id`.
- Releases that include the schema check exit with `<db> is schema 1, written by a newer rustmail; this binary supports schema 0. Upgrade rustmail.`

Either way your data is intact. Reinstall the new version, run `rustmail restore-backup`, then downgrade.

## Docker notes

- The upgrade writes the new file next to the old one, so the data volume must be **writable** and have room for both files. A read-only mount fails the upgrade before anything changes.
- The image's health check allows 10 seconds for startup. A large mailbox can take longer to upgrade, so the container may show as `unhealthy` until it finishes; that does not restart it. On Kubernetes, add a `startupProbe` with enough headroom so the pod is not killed mid-upgrade. A killed upgrade resumes on the next start, but a probe that keeps killing it means it never finishes.
- Pin the image tag (for example `smyile/rustmail:0.8.0` rather than `latest`) so the upgrade happens when you choose, and so you know which tag to go back to.

## Homebrew notes

`brew upgrade rustmail` followed by `brew services restart rustmail` runs the upgrade unattended as a background service. HTTP is unavailable until it finishes; follow its progress in the service log:

```sh
tail -f "$(brew --prefix)/var/log/rustmail.log"
```
