# Upgrading to 0.10

> Español: [docs/es/ACTUALIZAR_0.10.md](es/ACTUALIZAR_0.10.md).

This page is for an application that runs NopalDB **0.9.4** and moves to
**0.10.0**. Coming from 0.9.3 or earlier, read
[UPGRADING_0.9.md](UPGRADING_0.9.md) first: everything there still applies.

0.10 fixes the MVCC history (the versions behind `history`, `as_of` and
`get_node_at`), the GC, and the durability of NQL `update`. It is a minor
release because two Rust APIs changed in incompatible ways (section 2). Each
item links to its issue.

Install:

```bash
pip install -U "nopaldb==0.10.0"   # Python 3.10+
```

```toml
nopaldb = "0.10"
```

## 1. What does not change

- **Databases written by 0.9.4 open as they are.** The on-disk format of
  nodes, edges, indexes and the WAL is the same; a pending WAL left by a
  0.9.4 crash is replayed correctly by 0.10.
- The Python API only grew (section 4).

## 2. Rust code that may stop compiling or behave differently

- **`GCConfig` has a new public field, `min_age_ms`**
  ([#310](https://github.com/sharop/nopaldb/issues/310)). A struct literal
  that names every field without `..Default::default()` stops compiling. So
  does building `GcAutoSummary` or `GcSection` by hand: both gained fields.
  Use the constructors (`GCConfig::older_than_days(7)`,
  `GCConfig::before_timestamp(ts)`) or `..Default::default()`.
- **`get_node_at` and `as_of(ts)` return errors instead of the current
  node** ([#311](https://github.com/sharop/nopaldb/issues/311)). Until 0.9.4
  they returned the node as it is now when they found no version valid at
  `ts`, which was a silent wrong answer.

  | `graph.get_node_at(id, ts)` / `graph.as_of(ts).get_node(id)` | 0.10 |
  |---|---|
  | a version valid at `ts` exists | that version |
  | the node did not exist at `ts` (created later, or already deleted) | `NopalError::NodeNotFound` |
  | the node has no history (written with direct `add_node`), or `ts` is below the GC horizon | `NopalError::HistoryUnavailable` (new) |

  `GraphSnapshot::get_nodes` skips only `NodeNotFound` and returns any other
  error; it used to swallow every error. `NopalError` is `#[non_exhaustive]`,
  so a `match` already has a `_` arm.

## 3. Behaviour that changed

### GC by age keeps recent history ([#310](https://github.com/sharop/nopaldb/issues/310))

Until 0.9.4, `GCConfig::older_than_ms/hours/days` and `Graph::gc_default`
deleted **all** closed history: they compared Unix milliseconds with the
logical timestamps of the versions. History already deleted that way cannot
come back.

0.10 translates the age to the logical clock with wall-clock marks the
database now stores, at most one per second. The translation is
conservative: it only deletes what the database can prove was closed at
least that long ago.

**Consequence for existing databases: GC by age reclaims nothing at first.**
Nothing is reclaimed until the requested age has passed since the first write
made with 0.10, because there are no marks before that. If you rely on GC to
free space, expect no versions removed during that first period.
`GCConfig::before_timestamp(ts)` deletes by logical timestamp right away.

### Deleting a node closes its history ([#309](https://github.com/sharop/nopaldb/issues/309))

Every delete path closes the node's version, and the versions of the edges
deleted with it, at the delete's timestamp: direct `delete_node`,
transactions, NQL `delete`, Python `delete(label, key, value)` and WAL
replay. Re-creating a node with the same id continues its version chain.

WAL replay now re-applies every write with the timestamp recorded in the WAL.
Until 0.9.4, edges and node deletes got new timestamps on replay.

**Nodes deleted before 0.10 keep an open history.** There is no recorded
delete timestamp to repair them with, so `as_of(ts)` after their delete still
finds them. Only deletes made with 0.10 are closed.

### NQL `update … set` is a transaction ([#312](https://github.com/sharop/nopaldb/issues/312))

Like `add` and `delete`, each `update` statement is now one transaction:
- it is written to the WAL, so an acknowledged update survives a crash (until
  0.9.4 it could be lost before the next checkpoint);
- it leaves an MVCC version;
- it keeps the hash/btree/full-text indexes created with `create_index` up to
  date (a hash index used to keep finding the node by its old value);
- it is atomic: a failure in the middle writes nothing.

Under `Serializable`, an `update` now takes the write lock of each node it
changes. Rewriting an edge's properties still replaces its version instead of
chaining a new one.

## 4. New

- `Graph::last_commit_timestamp()` (Rust and Python): the logical timestamp
  of the latest applied write, the value to pass to `as_of` / `get_node_at`.
  `begin_transaction`, rollbacks and empty commits do not move it.
- `Graph::gc_horizon()` (Rust and Python; also `stats().gc.horizon`,
  `get_stats()["gc"]["horizon"]`): how far the GC collected. History from
  that timestamp on is complete.
- `GCConfig::before_timestamp(ts)`: a GC cutoff in logical time.

## 5. Still limited

- **No history for direct `add_node`, the BulkLoader and the Turtle/OWL
  import.** These writes still leave no version, so time travel over them
  answers `HistoryUnavailable`. Versioning every write is the next step.
- **NQL `find … at timestamp <n>` is still a named error**
  ([#298](https://github.com/sharop/nopaldb/issues/298)). Point-in-time
  reads are the Rust API above; Python has none yet.

## 6. Going back to 0.9.4

0.9.4 opens a database written by 0.10, because the format is the same. The
wall-clock marks are a catalog entry that 0.9.4 ignores. Keep a copy before
downgrading, as always.

## 7. What to test

On a copy of your database:
1. it opens and the counts match;
2. a GC by age you run regularly removes what you expect, given the warm-up
   above;
3. code that calls `get_node_at` / `as_of` handles `NodeNotFound` and
   `HistoryUnavailable`;
4. your NQL `update` statements still change exactly what you expect.
