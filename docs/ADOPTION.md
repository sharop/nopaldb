# Adopting NopalDB

The fastest path into NopalDB for each kind of user, plus the operational rules
that are easy to miss. Everything here reflects what actually ships today.

## 1. Rust (5 minutes)

```toml
[dependencies]
nopaldb = { version = "0.4", features = ["core"] }
```

| Tier | What you get |
|------|--------------|
| *default* | Property graph + NQL + MVCC + WAL (Sled storage) |
| `core` | + Arrow/Parquet export, graph algorithms, embeddings + HNSW, full-text search, ML helpers |
| `semantic` | + OWL-EL reasoner, Turtle import/export, SHACL validation |
| `full` | + `full-isolation`: isolation levels, per-node lock manager, deadlock detection |

```rust
use nopaldb::{Edge, Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./data.db").await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))).await?;
    let b = tx.add_node(Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))).await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.commit().await?;

    let result = graph.execute_nql("find p.name from (p:Person)").await?;
    println!("{}", result.summary());
    Ok(())
}
```

With `full-isolation`: `graph.begin_transaction().await?.with_isolation(IsolationLevel::Serializable)`.

## 2. Python (5 minutes)

```bash
pip install nopaldb
```

Prebuilt wheels for Linux/macOS/Windows, CPython 3.10+. The bindings release
the GIL during database calls, so Python threads get real parallelism.

```python
import nopaldb

graph = nopaldb.Graph.open("./data.db")
tx = graph.begin_transaction(isolation="serializable")  # kwarg optional
alice = tx.add_node("Person", {"name": "Alice", "age": 30})
tx.commit()

for row in graph.execute_nql("find p.name from (p:Person)"):
    print(row["p.name"])
```

Arrow export for ML: `graph.to_arrow(label="Person")` → `pyarrow` → Pandas/Polars/PyTorch.
Building from source instead: `pip install maturin && maturin develop --release --features python-full` (from `nopaldb/`).

## 3. LLM agents via MCP (15 minutes)

The `nopaldb-mcp` binary exposes the graph to Claude Desktop / Claude Code /
any MCP client — natural-language querying over your data:

```bash
cargo install --path nopaldb-mcp   # or build from the repo
nopaldb-mcp --db ./data.db --readonly
```

`--readonly` blocks write statements at the NQL level. See
[MCP_CLAUDE_DESKTOP.md](MCP_CLAUDE_DESKTOP.md) for client configuration.

## 4. Exploring visually: NDBStudio

```bash
cargo run -p ndbstudio -- --web --db ./data.db
```

Local web workbench: schema tree, NQL editor, graph visualization, session
history. TUI mode without `--web`. See [ndbstudio/web_quickstart.md](ndbstudio/web_quickstart.md).

## Operational rules (read before production)

1. **One process per data directory.** The storage engine holds a file lock;
   a second process opening the same path fails with "could not acquire lock".
   Close the other consumer (app, MCP server, studio) first. To share one
   database across clients, put the MCP server (or your own service) in front.
2. **Within a process, share by cloning.** `Graph` is `Clone + Send + Sync`
   (cheap, `Arc`-backed): clone the handle into every thread/task. All writes
   are serialized through a single-writer applier; concurrent commits share
   WAL fsyncs (group commit).
3. **Durability:** committed transactions survive `SIGKILL`; direct
   (non-transactional) writes have weaker guarantees — see
   [DURABILITY.md](DURABILITY.md). Use transactions when each operation must
   be durable.
4. **Isolation:** default is ReadCommitted. RepeatableRead/Serializable (with
   conflict detection and deadlock aborts) require the `full-isolation`
   feature — semantics in [ISOLATION_LEVELS.md](ISOLATION_LEVELS.md). On
   conflict (`TransactionConflict`/`Deadlock`/`ConcurrencyError`), retry the
   transaction.
5. **Bulk ingestion:** use `graph.bulk_loader(batch_size)` instead of
   per-item transactions.
6. **Update-heavy datasets:** enable MVCC garbage collection
   (`graph.start_auto_gc(config)`), or old versions accumulate. GC never
   removes versions still readable by open transactions.
