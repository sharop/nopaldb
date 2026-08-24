# Backup, restore, and read-only access

NopalDB is **single-writer per data directory**. The storage engine takes an
exclusive OS lock when the database is opened, and that is the whole
concurrency model: one process owns a directory at a time. This page covers
what to do when you want to read a database without disturbing the process
that owns it, and how to take a backup you can trust.

## What read-only mode is, and what it is not

```rust
let graph = Graph::open_read_only("data/graph.db").await?;
```

**It guarantees this process cannot modify the data.** Every write path fails
with a typed error instead of touching the database: direct writes,
transaction commits, embeddings, index creation. Use it for an explorer, a
query service, a reporting job — anything that must not write by accident.

**It does not give you concurrent access.** It still takes the same exclusive
lock, so it cannot open a database another process has open; you will get the
"already open" error. This is not an implementation shortcut we plan to
remove: neither embedded engine can offer it today. sled has no read-only mode
at all, and redb's own read-only handle is documented to fail when the file is
open for writing.

If you want to read without touching the live database, take a backup and open
the copy. That is the section below.

One more thing worth knowing: **opening does write.** Pending layout
migrations run and the WAL is replayed, because a half-recovered database is
not coherently readable. The read-only seal closes as soon as that finishes,
so nothing you do afterwards can change anything.

## Cold backup

`Storage::copy_database` copies every keyspace byte for byte and verifies the
result — pair counts plus a checksum recomputed by re-scanning the
destination. Nothing is reinterpreted, so MVCC history and indexes survive
intact.

```rust
use nopaldb::storage::{Storage, StorageOptions};

let report = Storage::copy_database(
    "data/graph.db",  StorageOptions::default(),
    "backups/2026-08-24", StorageOptions::default(),
).await?;
assert!(report.verified);
```

Then read the copy however you like:

```rust
let backup = Graph::open_read_only("backups/2026-08-24").await?;
```

### Rules

- **Both directories must be closed.** This is a *cold* backup: the source is
  opened with the same exclusive lock as a normal open. It is not a hot
  backup, and there is no way to take one today — see above.
- **Close the source cleanly first** (`graph.close().await`), so its WAL is
  applied. `copy_database` copies engine keyspaces, not `nopal.wal`.
- **The destination must be empty.** Copying into a directory that already has
  data is refused rather than merged.
- **Do not copy the files yourself while the database is open.** A filesystem
  copy of a live database can capture a torn state that looks valid and is
  not. The verification above is exactly what a `cp` does not give you.

### Restore

Restoring is opening the copy — there is no separate step. To put it back in
place, stop the writer, move the directory, and open it.

## Which engine

Everything on this page works with both storage backends. The wheels published
on PyPI ship sled; see [ADOPTION.md](ADOPTION.md).
