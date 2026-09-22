# Operating NopalDB: what to look at when something is slow or seems stuck

Since 0.6.5 the database reports its own operational state in one call, and
long operations can report progress while they run. This page says which
number to read for each symptom, and what to do with it. Nothing here needs
logs; the `log::info!` lines still exist for whoever reads them.

- Python: `graph.get_stats()` (nested dict, see below) and
  `graph.set_progress_callback(fn)` / `Graph.open_with_options(..., on_progress=fn)`.
- Rust: `Graph::stats()` (`StatsReport`), `Graph::set_progress_callback`,
  `Graph::open_with_progress`.
- Terminal, without writing code: `nopaldb stats <dir>` (feature `cli`,
  the same binary as `nopaldb migrate`). Opens the base read-only, prints
  every section and shows the replay progress on stderr. The base must be
  closed: one process per directory.

## The report

| section    | keys | what it answers |
|------------|------|-----------------|
| `graph`    | `total_nodes`, `total_edges`, `avg_degree`, `nodes_per_label`, `edges_per_type` | how big is it |
| `storage`  | `engine`, `profile`, `data_dir`, `read_only` | what did I open |
| `wal`      | `bytes`, `checkpoint_threshold_bytes`, `checkpoints_this_session`, `last_checkpoint_unix_ms`, `direct_write_durability` | what would the next open replay; is the checkpoint running |
| `recovery` | `wal_records_read`, `operations_replayed`, `uncommitted_txs_discarded`, `crash_recovery`, `adjacency_rebuilt`, `open_ms` (`storage`, `wal_replay`, `adjacency`, `indexes`, `total`) | what did the last open do and how long each phase took |
| `indexes`  | list of `name`, `label`, `property`, `type`, `size`, `analyzer` | which user indexes are loaded, how full, tokenized how |
| `hnsw`     | list of `model`, `size`, `tombstones`, `dimension`, `needs_rebuild`, `persisted`, `loaded_from_disk_ms` | state of each vector index in cache |
| `gc`       | `auto_running`, `auto` (config), `last_run` (`unix_ms`, `nodes_scanned`, `versions_removed`, `bytes_freed`, `duration_ms`, `dry_run`) | is MVCC garbage collection running, when did it last run |

`recovery` is filled once, by the open that created the handle; in memory it
is all zeros. Everything else is read live and costs nothing: no scan, only
counters that `open`, `checkpoint` and `gc` already keep.

The flat keys of 0.6.x (`total_nodes`, `total_edges`, `avg_degree`,
`storage_engine`, `wal_bytes`, all strings) stay at the top level of the
Python dict for one more minor release. Read the sections instead.

## Symptom → what to read

**The open is slow.** `recovery.open_ms` says where the time went.

- `wal_replay` dominates and `wal_records_read` is large: the previous
  session ended without `close()` (or crashed) and left a WAL that had to be
  read whole. `operations_replayed` is how many operations were actually
  re-applied; it is usually far smaller than the records read, because the
  applier had already landed most of them. To make the next open instant,
  `checkpoint()` before closing, or lower `wal_checkpoint_bytes`.
- `adjacency` dominates and `adjacency_rebuilt` is true: the adjacency was
  reconstructed from the edges, either because the open was a crash recovery
  or because the adjacency keyspace was empty. It is proportional to the
  number of edges and happens once.
- `indexes` dominates: the user indexes are rebuilt from the nodes when their
  on-disk state is missing or stale. A full-text index over a large label is
  the usual cause.
- `storage` dominates: the engine itself (lock, file open, layout migration
  on a base from before 0.5). On a base created with 0.6 this is tens of
  milliseconds.

**Ingestion is slow or "stopped".** Register a progress callback before the
load and watch `done` move. Phases: `upsert_batch` (total known),
`bulk_load` (nodes plus edges written so far, total unknown until `finish`),
`index_build` (a `create_index` over existing nodes). If `done` does not move
for seconds while the process is alive, the writer is waiting on the write
gate: a `gc()` cycle or an automatic checkpoint is holding it. `gc.last_run`
and `wal.checkpoints_this_session` tell which.

**The WAL keeps growing.** `wal.bytes` returns to about 100 bytes after each
checkpoint. If it does not, `wal.checkpoint_threshold_bytes` is `0`
(automatic checkpoint disabled) and nothing calls `checkpoint()`; or the
applier's checkpoint failed and the log says why. `checkpoints_this_session`
and `last_checkpoint_unix_ms` confirm whether any happened.

**Full-text search returns nothing.** `indexes` lists what is loaded. A
full-text index missing from the list while `<dir>/indexes/metadata.bin`
exists means it failed to load at open (the log has the error;
`rebuild_indexes()` retries). `size` at `0` on a label that has nodes means
the index was created before the nodes and never populated: the nodes did
not pass through the indexed path (a bulk load does not index; run
`rebuild_indexes()`). `analyzer` is what the tokens on disk were produced
with; a query analyzed differently misses them.

**Vector search is slow the first time.** `hnsw` is empty until the first
search builds or loads the index. After it, `persisted` false means the next
open will rebuild from the embeddings; `close()` writes the dump.
`needs_rebuild` true means enough tombstones accumulated that the next search
pays a rebuild.

**Disk keeps growing on an update-heavy base.** `gc.last_run` is `None`:
garbage collection never ran, and every update keeps its previous version.
Start it (`start_auto_gc` in Rust; from Python, run `gc()` from a Rust side
service until the binding lands) and check `versions_removed` on the next
report. `auto_running` false with `auto` set means the scheduler died; the
log has the reason.

## The progress event

```python
def on_progress(e):            # {"phase": str, "done": int, "total": int | None}
    print(e["phase"], e["done"], e["total"])

g = nopaldb.Graph.open_with_options("big.db", on_progress=on_progress)  # wal_replay, adjacency_rebuild, index_load
g.set_progress_callback(on_progress)                                    # upsert_batch, bulk_load, index_build
g.set_progress_callback(None)
```

Phases: `wal_replay`, `adjacency_rebuild`, `index_load`, `index_build`,
`property_index_rebuild`, `bulk_load`, `upsert_batch`. Every phase announces
itself with `done = 0`, then emits every ~1000 items or ~250 ms, whichever
comes first, and once more at the end with `done == total`. The callback runs
on the thread doing the work: keep it cheap, never block, and do not call
the graph from it. Without a callback registered the cost is one comparison
per batch. An exception raised by the callback is logged and ignored; it
cannot abort a replay.

## Reading the logs instead

The same information is in the log at `info` level, in free text:
`Recovery analysis: N total records, C committed txs, U uncommitted txs`,
`Replayed N operations from WAL`, `Crash recovery detected: rebuilding
adjacency from edges...`, `Checkpoint completed`, `Auto GC cycle complete:
scanned=…, deleted=…`. `rebuild_property_index` logs every 1000 nodes. Set
`RUST_LOG=nopaldb=info` (Rust, with `env_logger`) to see them.

See also [DURABILITY.md](DURABILITY.md) for what a checkpoint guarantees and
[BACKUP_AND_READ_ONLY.md](BACKUP_AND_READ_ONLY.md) for reading a base without
touching it.
