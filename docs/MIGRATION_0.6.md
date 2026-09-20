# Migrating to 0.6.0: redb is the default engine

> Español: [docs/es/MIGRACION_0.6.md](es/MIGRACION_0.6.md).

Since 0.6.0 new databases are created with **redb**. **sled**, the engine of
every 0.5.x database, stays available behind the `storage-sled` feature to
open and migrate those databases, at least through 0.7. Nothing else changes:
same layout, same WAL, same NQL, same Python API.

## Nothing breaks on upgrade

`Graph::open(path)` (Rust) and `Graph.open(path)` (Python) use
`StorageEngine::Auto`: they look at the directory and use the engine that is
already there. A 0.5.x database keeps working with sled and logs a warning
that says how to migrate. A new directory gets redb.

| Directory contains | `Auto` opens with |
|---|---|
| `nopal.redb` | redb |
| `conf` and `db` | sled |
| nothing | redb (the build's default) |

Explicit requests stay explicit: `engine = Redb` on a sled directory (or the
reverse) is an error whose message says how to migrate. A build compiled
without `storage-sled` cannot open a sled database and says so, naming the
feature and the migration tools. The PyPI wheels ship both engines.

`graph.storage().backend_name()` (Rust) and `graph.storage_engine()` (Python)
tell you which engine a handle actually opened.

## Why migrate

Measured on the same machine (0.5.24; see the [performance table in issue #131](https://github.com/sharop/nopaldb/issues/131)):
transactional commits 2.4–2.9× faster, reads 1.03×, bulk ingest 1.42×, GC of
20k versions 69× faster, disk after GC 0.03× of sled; reopening a large
database takes milliseconds instead of seconds, and sled has had no upstream
development since 2021. Two costs stay with redb and are documented in
[DURABILITY.md](DURABILITY.md): creating a **new** database costs ~60 ms of
fsyncs (once per directory), and a direct `add_edge` outside a transaction is
slower per operation than on sled (168 µs per four edges) because redb writes
its pages on every commit window.

## How to migrate

The migration is a byte-for-byte copy of every keyspace (nodes, MVCC versions,
edges, adjacency, property index, clocks, embeddings) followed by a re-scan of
the destination that checks counts and checksums, plus a file-by-file copy of
the two directories that live outside the KV: `indexes/` (user indexes and
their analyzers) and `hnsw/` (the vector index dump), each file verified by
size and checksum. Time-travel survives because nothing is reinterpreted; a
round trip sled → redb → sled of a million-pair database is verified in the
test suite. See [What travels and what is rebuilt](#what-travels-and-what-is-rebuilt).

Preconditions:

1. No process has either directory open.
2. The source was opened and closed cleanly at least once with NopalDB, so its
   WAL is applied (open it and call `close()` if unsure).
3. The destination directory is empty or does not exist. The copy never merges.

### Python

```python
import nopaldb

report = nopaldb.Graph.migrate("data/plantas.db", "data/plantas_redb.db")
assert report["verified"]                 # counts and checksums match
graph = nopaldb.Graph.open("data/plantas_redb.db")
graph.storage_engine()                    # "redb"
```

`src_engine` and `dst_engine` default to `"auto"` (detect the source; redb for
the destination). A failed verification raises and the destination must not be
used.

### Rust

```rust
use nopaldb::{Storage, StorageEngine, StorageOptions};

let report = Storage::copy_database(
    "data/plantas.db",       StorageOptions::default(),                                   // Auto: detects sled
    "data/plantas_redb.db",  StorageOptions { engine: StorageEngine::Redb, ..Default::default() },
).await?;
assert!(report.verified);
```

### Command line

The `nopaldb` binary (feature `cli`, both engines built in) wraps the same
function:

```bash
cargo install nopaldb --features cli
nopaldb engine data/plantas.db                       # sled | redb | ninguno
nopaldb migrate data/plantas.db data/plantas_redb.db  # --from auto --to redb by default
```

It prints one line per keyspace (pairs and bytes), the totals and the
verification result. Exit codes: 0 verified; 1 usage or invalid arguments;
2 the copy failed or the destination was not empty. Without installing, the
repository example does the same:

```bash
cargo run --example migrate_engine --features storage-sled -- data/plantas.db auto data/plantas_redb.db redb
```

### Going back

The copy works in both directions: `dst_engine="sled"` recreates a sled
database from a redb one, verified the same way.

## What travels and what is rebuilt

| Part of the database | Where it lives | Migration | On open in the destination |
|---|---|---|---|
| Nodes, edges, MVCC history, adjacency, clocks, property index, embeddings | the 12 KV keyspaces | copied pair by pair, verified by count and checksum | used as is |
| User indexes (hash, btree, full-text, taxonomy): catalog `indexes/metadata.bin`, full-text segments and `analyzer.json` under `indexes/fulltext_<name>/` | `<dir>/indexes/` | copied file by file, verified by size and checksum (since 0.6.3) | hash/btree/taxonomy are rebuilt in memory from the catalog and the nodes, as on every open; full-text opens its segments with the analyzer it was created with |
| HNSW index | `<dir>/hnsw/` | copied if the source had a dump (since 0.6.3) | loaded from disk on the first search; if there was no dump, rebuilt from `embeddings` on the first search, exactly as the source would |
| WAL | `<dir>/wal/` | not copied: precondition 2 means it is already applied | starts empty |

The report has the three sections: `keyspaces` + `verified`, `indexes` (name,
label, property, type, analyzer) and `sidecars` + `hnsw_copied`. Before 0.6.3
only the first existed, and `verified=true` said nothing about the indexes,
which silently did not travel. If you copy a database with your own tools, put
`indexes/` in place and call `Graph::rebuild_indexes()` /
`graph.rebuild_indexes()`, or reopen.

## Compatibility matrix

| Database written by | Opens directly in 0.6.x | Notes and way back |
|---|---|---|
| ≤ 0.4.35 | yes, sled (`Auto` detects it) | first open migrates the property index to `prop_idx_v2` (0.4.36) and the layout to v2 (0.5.3), in place and idempotently; **not readable by ≤ 0.5.2 afterwards**. Keep a copy of the volume if you may need to go back. |
| 0.5.3 – 0.5.12, sled | yes, sled | full-text indexes have no `analyzer.json`: they keep the default analyzer. |
| 0.5.13 – 0.5.24, sled or redb | yes, either engine | 0.5.20+ writes `hnsw/`; 0.5.22+ mints UUID v7 ids (older versions read them as plain UUIDs). |
| 0.6.x, redb | yes | a 0.5.x build with `storage-redb` opens it too, but sled-only wheels of 0.5.x do not: migrate to sled first (`dst_engine="sled"`). |
| 0.6.x, sled | yes (with a migration hint in the log) | readable by 0.5.3+. |

Going back a version: restore the previous image and the untouched copy of the
volume, or migrate to the engine that version supports. Never open a database
modified by a newer version with an older one without a verified copy.

## Creating a sled database on purpose

Only if you need it (for example, to hand a database to a 0.5.x build): pass
`engine = StorageEngine::Sled` / `engine="sled"` explicitly. `Auto` never
creates a new sled database.

## Policy

- 0.6 and 0.7 keep `storage-sled` and both engines in the wheels.
- Its removal will be decided with usage data in 0.8 and announced one minor
  ahead in the CHANGELOG.
