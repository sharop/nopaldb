# NopalDB Configuration Guide

Currently, NopalDB aims for a **zero-configuration** experience. It provides sane defaults optimized for general-purpose workloads, balancing write throughput and read latency.

---

## Default Settings

When you initialize a graph with `Graph.open("path")`, the following configuration is applied automatically:

### 💾 Storage Engine (redb)
- **Backend**: redb, an embedded copy-on-write B-tree (the only engine since 0.8.0).
- **Caching**: redb's page cache, sized by the profile (`default` 1 GiB, `mobile` 16 MiB, `server` 256 MiB).
- **Compression**: none.
- **Durability**: every write goes to NopalDB's WAL first; the engine commits in short windows and a checkpoint makes them durable (see [DURABILITY.md](../DURABILITY.md)).

### 🔄 Concurrency (MVCC)
- **Isolation Level**: Read Committed by default. Choose another per transaction:
  `graph.begin_transaction("read_uncommitted" | "read_committed" | "repeatable_read" | "serializable")`
  (any other string is a `ValueError`). The locks a `serializable` transaction
  takes are released on commit and, since 0.9.4 (#296), also on rollback and
  when the transaction object is dropped without either.
- **Reads**: queries read MVCC versions and take no write locks. The exceptions
  are the consistent whole-graph reads (`export_graph`, `nopaldb.networkx.to_networkx`,
  `to_arrow_complete(label)`), which make writers wait while they read.
- **Timestamping**: 64-bit monotonic timestamps.
- **Garbage Collection** of old versions: Rust only (`graph.gc(...)` on demand or
  `start_auto_gc(AutoGcConfig)` in the background; see `docs/ARCHITECTURE.md` § GC).
  Python has no GC API; `get_stats()["gc"]` reports its state.

### 🪵 Durability (WAL)
- **Write-Ahead Log**: Enabled. All transactions are appended to the WAL before commit.
- **Checkpoint**: automatic when the WAL passes 16 MiB
  (`get_stats()["wal"]["checkpoint_threshold_bytes"]`) and on `close()`;
  `graph.checkpoint()` forces one.
- **Recovery**: Automatic crash recovery on restart.

---

## Tuning Performance

While there are no config files (`nopal.conf`), you can optimize performance via your usage patterns:

### 1. Batch Writes
Wrap multiple operations in a single transaction to reduce disk sync overhead.

```python
# ✅ FAST: One transaction, multiple writes
tx = graph.begin_transaction()
for i in range(1000):
    tx.add_node("Item", {"id": i})
tx.commit()

# ❌ SLOW: 1000 transactions
for i in range(1000):
    tx = graph.begin_transaction()
    tx.add_node("Item", {"id": i})
    tx.commit()
```

### 2. Indexes
Adjacency indices (who connects to whom) are automatically maintained.
Property indices are **explicit**: nothing is indexed until you declare it.

```python
graph.create_index("Item", "sku", "hash")         # equality
graph.create_index("Item", "price", "btree")      # ranges
graph.create_index("Note", "body", "fulltext", analyzer={"language": "english"})
graph.list_indexes()   # [(name, label, property, type), ...]
```

### 3. Memory Usage
For large imports (millions of nodes), use the bulk loader: it buffers
`batch_size` rows and writes each batch at once, instead of holding a whole
transaction in memory.

```python
with graph.bulk_loader(10_000) as loader:
    a = loader.add_node("Item", {"sku": "A-1"})
    b = loader.add_node("Item", {"sku": "B-2"})
    loader.add_edge(a, b, "RELATED")
graph.rebuild_indexes()   # bulk batches do not populate user indexes
```

---

## Profiles

The only knob at open time is the profile. It sets the redb page cache:

| profile | redb page cache |
|---|---|
| `"default"` (what `Graph.open` uses) | 1 GiB |
| `"mobile"` | 16 MiB |
| `"server"` | 256 MiB |

Any other name is a `ValueError`.

```python
graph = nopaldb.Graph.open_with_profile("data/my_graph.db", "mobile")
graph = nopaldb.Graph.in_memory_with_profile("server")
graph.get_stats()["storage"]["profile"]   # "mobile"
```

`Graph.open_with_options(path, engine="auto", profile="default", on_progress=None)`
takes the same profile names. Thread pools, the WAL checkpoint threshold and
compression are not configurable from Python.
