# NopalDB 🌵

High-performance embedded **graph database** for Python, written in Rust:
ACID transactions, MVCC time-travel, a Cypher-like query language (NQL),
vector search, and zero-copy Apache Arrow export for ML pipelines.

Runs **in-process** — no server to deploy. One `pip install`, one file on disk.

## Install

```bash
pip install nopaldb
```

Prebuilt wheels for Linux (x86_64/aarch64), macOS (Intel/Apple Silicon) and
Windows, for CPython 3.10+ (abi3: one wheel per platform covers all versions).

## Quickstart

```python
import nopaldb

graph = nopaldb.Graph.open("./data.db")

# ACID transactions
tx = graph.begin_transaction()
alice = tx.add_node("Person", {"name": "Alice", "age": 30})
bob = tx.add_node("Person", {"name": "Bob", "age": 25})
tx.add_edge(alice, bob, "KNOWS")
tx.commit()

# Query with NQL
result = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")
for row in result:
    print(f"{row['p.name']}: {row['p.age']}")

graph.close()
```

### Isolation levels

```python
tx = graph.begin_transaction(isolation="serializable")
# read_committed (default) | repeatable_read | serializable | read_uncommitted
```

Serializable transactions detect write conflicts and deadlocks; see the
[durability guarantees](https://github.com/sharop/nopaldb/blob/main/docs/DURABILITY.md)
and [isolation guide](https://github.com/sharop/nopaldb/blob/main/docs/ISOLATION_LEVELS.md).

### Arrow / ML pipelines

```python
import pyarrow as pa

arrow_bytes = graph.to_arrow(label="Person")
batch = pa.ipc.open_stream(arrow_bytes).read_next_batch()
df = batch.to_pandas()   # zero-copy into Pandas / Polars / PyTorch pipelines
```

### Good to know

- **One process per data directory** (embedded file lock). Threads within the
  process run in parallel — the bindings release the GIL during database calls.
- Use `graph.bulk_loader(batch_size)` for large ingestions.
- Time-travel: MVCC version history is queryable; old versions are garbage
  collected on demand.

## Links

- **Repository & docs:** <https://github.com/sharop/nopaldb>
- **Adoption guide:** <https://github.com/sharop/nopaldb/blob/main/docs/ADOPTION.md>
- **NQL reference:** <https://github.com/sharop/nopaldb/blob/main/docs/en/NQL_REFERENCE.md>
- **Rust crate:** <https://crates.io/crates/nopaldb>

License: AGPL-3.0-only. NopalDB™ is a trademark of Sergio Haro Pérez.
