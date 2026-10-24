---
description: "Upgrade RustMail to 0.8, 0.10 and later: the automatic database upgrade, where the backup goes, and how to roll back."
---

# Upgrading

Two RustMail releases store messages in a new database layout:

| Release | Layout | Upgrades databases written by |
|---|---|---|
| 0.8 | schema 1: bodies and metadata in separate tables | 0.7 and earlier |
| 0.10 | schema 2: each message's source stored compressed, bodies read from it | 0.7 and earlier, 0.8 and 0.9 |

The first time a newer version starts on an older database, it upgrades that database automatically, in one step even when it skips a release. There is nothing to run by hand, and no API changes.

This page covers what happens during that upgrade, where the backup goes, and how to go back.

## What happens on first start

Before the SMTP and HTTP listeners start, RustMail:

1. Takes a lock file next to the database, `rustmail.db.migration-lock`.
2. Builds the new database beside the old one, as `rustmail.db.migrating`, copying messages in batches.
3. Verifies the copy against the old database (counts, search index, integrity checks), and every compressed message against the source it came from.
4. Swaps the files: the old database becomes the backup (see [The backup](#the-backup)) and the copy becomes `rustmail.db`.

The old database is only read, never modified. If a check fails, RustMail exits with an error, keeps `rustmail.db.migrating` for a bug report, and leaves `rustmail.db` as it was.

The log shows progress while it runs. Every line carries `event="storage_migration"` and a `phase`:

| Phase | Message |
|---|---|
| `start` | `Migrating the database to the current storage schema; SMTP and HTTP start once it is done`, with the message count, the database size and the `source_schema` |
| `copy` | `Storage migration in progress`, every 2 seconds, with `migrated`, `total`, `rate_per_s` and `eta_s` |
| `verify` | `Verifying the migrated copy` |
| `done` | `Storage migration complete; the previous database is kept as the backup`, with `duration_s` and `backup_path` |
| `failed` | `Storage migration failed`, with the `error` and a `hint` |

### How long it takes

Expect about **4 seconds per 100,000 small messages** on a recent machine, growing linearly with the mailbox; an upgrade from 0.9 took 4.2 seconds for 100,000 small messages on an Apple M4 Pro. Large messages with attachments cost more per message. Under Docker Desktop, which runs containers in a VM, expect it to take a few times longer.

SMTP and HTTP are not available until the upgrade finishes. Mail sent to RustMail in the meantime is refused at connection time, so senders that retry will deliver it afterwards.

### Stopping and resuming

You can stop RustMail during the upgrade (`Ctrl+C` or `SIGTERM`). It finishes the batch in progress, logs `Storage migration paused at N/T; it resumes on next start`, and exits. The next start picks up where it stopped. If the old database was changed in the meantime (for example by an older RustMail), the copy is rebuilt from scratch. A copy that RustMail 0.8 or 0.9 left behind is rebuilt in the new layout, unless they were stopped while swapping it in: then it is swapped in and upgraded again on the same start.

### Disk space

During the upgrade both files exist side by side, so you need room for the old database **plus** the new one. The new file is smaller than the old one: upgrading 100,000 small text messages from 0.9 shrank the database from 466 MB to 101 MB. Mail that is mostly attachments shrinks less, since attachments are usually compressed already. If the disk fills, RustMail exits with:

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

When upgrading from 0.7 or earlier, the search index is rebuilt from the stored mail. Older databases could hold stale index entries for mail that had been deleted, so some searches can return different results or totals after the upgrade. The messages themselves are unchanged. An upgrade from 0.8 or 0.9 indexes the same text those versions indexed.

## The backup

The database from before the upgrade is kept next to the new one, named after the layout it is in:

| Upgraded from | Backup |
|---|---|
| 0.7 or earlier | `rustmail.db.schema0.bak` |
| 0.8 or 0.9 | `rustmail.db.schema1.bak` |

It sits in the same directory as the database:

| Install | Directory |
|---|---|
| macOS (default path, Homebrew) | `~/Library/Application Support/rustmail/` |
| Linux (default path) | `~/.local/share/rustmail/` |
| Docker | `/data/` in the data volume |
| Custom `--db-path` / `RUSTMAIL_DB_PATH` | next to `<db-path>`, as `<db-path>.schema0.bak` or `<db-path>.schema1.bak` |

If you upgraded through both releases, both backups are there. RustMail never deletes them. Every start logs each one's path and size (`event="storage_backup"`) as a reminder.

**When it is safe to delete:** once you have checked that your mail looks right in the new version and you do not plan to go back to the version that wrote the backup. After you delete it, `rustmail restore-backup` has nothing to restore.

## Going back to an older version

Older binaries cannot read a newer layout. Restore the backup **with the new binary first**, then install the older version.

1. Stop every RustMail process using the database.
2. Run:

   ```sh
   rustmail restore-backup
   ```

   Pass `--db-path` (or set `RUSTMAIL_DB_PATH`) if you use a custom path; it resolves the database the same way `rustmail serve` does.

3. Install and start the older version.

`restore-backup` puts back the backup of the upgrade that produced the current database: `rustmail.db.schema1.bak` if you upgraded from 0.8 or 0.9, `rustmail.db.schema0.bak` if you upgraded from 0.7 or earlier. To go back further, from 0.9 to 0.7 for example, run `restore-backup` again with the 0.8 or 0.9 binary.

`restore-backup` takes the same lock as the upgrade, so it refuses to run while another RustMail holds it, and it refuses while another process still has the database open. It renames the current database to `rustmail.db.schema2-<timestamp>` (it never deletes it) and moves the backup back to `rustmail.db`.

If `restore-backup` is stopped between those two moves, `rustmail.db` is missing; run `restore-backup` again and it finishes the restore, after the same checks. It takes the newest `rustmail.db.schema2-<timestamp>` as the file it moved away, reads from it which backup belongs back, and leaves every kept file as it is. It refuses instead when the files do not show that state, for example when no `rustmail.db.schema2-<timestamp>` sits beside the backup or the newest one is not a schema-2 database, and its message says which file to move by hand.

::: warning Mail received after the upgrade is not restored
The backup holds the mailbox as it was when you upgraded. Mail received after that, and any changes such as deletions or read flags, exist only in the kept file (`rustmail.db.schema2-<timestamp>`). Starting the new version again on the restored file upgrades it again.
:::

### Docker

```sh
docker compose stop rustmail
docker run --rm -v rustmail-data:/data smyile/rustmail:<new-version> restore-backup
```

Then change the image tag in your Compose file to the older version and start it.

### If you downgrade without restoring

Older binaries refuse the upgraded file at startup and write nothing to it:

- RustMail 0.8 and 0.9 exit with an error naming the file: `<db> is schema 2, written by a newer rustmail; this binary supports schema 1`.
- RustMail 0.7.0 and earlier exit with an SQL error ending in `no such column: message_id`.

Your data is intact. Reinstall the new version, run `rustmail restore-backup`, then downgrade.

## Docker notes

- The upgrade writes the new file next to the old one, so the data volume must be **writable** and have room for both files. A read-only mount fails the upgrade before anything changes.
- The image's health check allows 10 seconds for startup. A large mailbox can take longer to upgrade, so the container may show as `unhealthy` until it finishes; that does not restart it. On Kubernetes, add a `startupProbe` with enough headroom so the pod is not killed mid-upgrade. A killed upgrade resumes on the next start, but a probe that keeps killing it means it never finishes.
- Pin the image tag (for example `smyile/rustmail:0.10.0` rather than `latest`) so the upgrade happens when you choose, and so you know which tag to go back to.

## Homebrew notes

`brew upgrade rustmail` followed by `brew services restart rustmail` runs the upgrade unattended as a background service. HTTP is unavailable until it finishes; follow its progress in the service log:

```sh
tail -f "$(brew --prefix)/var/log/rustmail.log"
```
