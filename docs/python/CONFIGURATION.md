# NopalDB Configuration Guide

Currently, NopalDB aims for a **zero-configuration** experience. It provides sane defaults optimized for general-purpose workloads, balancing write throughput and read latency.

---

## Default Settings

When you initialize a graph with `Graph.open("path")`, the following configuration is applied automatically:

### 💾 Storage Engine (Sled)
- **Backend**: Hybrid Log-Structured Merge Tree (LSM).
- **Caching**: Automatic page cache management. No manual size limit configured yet.
- **Compression**: Enabled by default (Zstd/Snappy depending on build).
- **Flushing**: Asynchronous flushing to disk for high throughput.

### 🔄 Concurrency (MVCC)
- **Isolation Level**: Snapshot Isolation. Readers never block writers, writers never block readers.
- **Timestamping**: 64-bit monotonic timestamps.
- **Garbage Collection**: currently manual via internal APIs (auto-vacuuming planned for future releases).

### 🪵 Durability (WAL)
- **Write-Ahead Log**: Enabled. All transactions are appended to the WAL before commit.
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
Property indices are created automatically for all properties added during node creation.

### 3. Memory Usage
For large batch imports (millions of nodes), consider using the **Bulk Loader API** (if available in your bindings version) or ensuring your machine has sufficient RAM, as transaction buffers exist in memory before commit.

---

## Future Configuration

Upcoming versions (v0.3+) will introduce a `Config` object to tune:
- Cache size limits
- WAL checkpoint frequency
- Thread pool size
- Compression levels

Stay tuned! 🌵
