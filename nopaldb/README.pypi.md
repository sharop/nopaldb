<img width="140" alt="NopalDB logo" src="https://raw.githubusercontent.com/sharop/nopaldb/main/assets/nopaldb_logo.png" />

# NopalDB 🌵

[![PyPI](https://img.shields.io/pypi/v/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![Python](https://img.shields.io/pypi/pyversions/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![PyPI downloads](https://img.shields.io/pypi/dm/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![CI](https://github.com/sharop/nopaldb/actions/workflows/community-ci.yml/badge.svg)](https://github.com/sharop/nopaldb/actions/workflows/community-ci.yml)
[![License](https://img.shields.io/badge/license-MPL--2.0-brightgreen.svg)](https://github.com/sharop/nopaldb/blob/main/nopaldb/LICENSE)

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

### What the wheel contains

These wheels are built with the **`sled` storage backend only**. That is what
`Graph.open(...)` uses, and it is the backend the project supports for
production use.

The engine is selectable in the API — `Graph.open_with_options(path,
engine=...)` — but `engine="redb"` raises `ValueError` on a wheel from PyPI,
because that backend is not compiled in. redb is **experimental**: it exists
behind a Cargo feature, is not enabled by default, and is still being
qualified against the crash-recovery suite. To try it you must build from
source:

```bash
pip install maturin
git clone https://github.com/sharop/nopaldb && cd nopaldb
maturin build --release --features python-full,storage-redb -m nopaldb/Cargo.toml
```

Treat such a build as an experiment, not as a supported configuration. The
wheels on PyPI will ship redb once it clears its qualification gate.

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

License: MPL-2.0 (the `nopaldb` library — this repository in its entirety). The companion ecosystem apps (MCP server, NDBStudio) are AGPL-3.0-only and live in their own repositories: <https://github.com/Anxious-Mind-Group/nopaldb-mcp> · <https://github.com/Anxious-Mind-Group/ndbstudio>. Releases ≤ 0.4.31 were AGPL-3.0-only. NopalDB™ is a trademark of Sergio Haro Pérez.
