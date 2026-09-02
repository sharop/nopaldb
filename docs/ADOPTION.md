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
    for row in result.rows() {
        println!("{:?}", row.get("p.name"));
    }
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

The MCP server exposes the graph to Claude Desktop / Claude Code /
any MCP client — natural-language querying over your data, with a
`--readonly` mode that blocks write statements at the NQL level.

The server lives in its own repository: build/run instructions and client
configuration at https://github.com/Anxious-Mind-Group/nopaldb-mcp.

## 4. Exploring visually: NDBStudio

Local TUI/web workbench: schema tree, NQL editor, graph visualization,
session history.

NDBStudio lives in its own repository: build/run instructions at
https://github.com/Anxious-Mind-Group/ndbstudio.

## Operational rules (read before production)

1. **One process per data directory.** The storage engine holds a file lock;
   a second process opening the same path fails with an error naming the
   directory and what to do. Close the other consumer (app, MCP server,
   studio) first. To share one database across clients, put the MCP server
   (or your own service) in front.

   To read a database without disturbing its owner, take a cold backup and
   open the copy — and to make sure a process cannot write, open it with
   `Graph::open_read_only`. Both are covered in
   [BACKUP_AND_READ_ONLY.md](BACKUP_AND_READ_ONLY.md); note that read-only
   mode does **not** grant concurrent access.
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

## Interoperating with an RDF store: what the Turtle bridge keeps

`import_turtle` / `export_turtle` (feature `semantic`) are an **ontology
loader**, not an RDF store. If you are deciding whether NopalDB can sit next
to a triple store, this is the contract — the full version, with the parser's
gaps, is in the `nopaldb::rdf_owl` module docs on docs.rs.

**Kept:** `owl:Class` declarations (one node each), `rdfs:subClassOf` (one
edge each, plus the taxonomy that powers `instanceOf`/`subClassOf` in NQL),
individuals of a declared class (one node, with an `iri` property), and their
literal-valued properties (typed by value: `"42"` → int, `"true"` → bool).
Re-importing is idempotent, and instances may come in a later file than their
classes.

**Lost:**

- **IRIs.** Terms are reduced to their local name; prefixes are dropped. Two
  IRIs with the same local name in different namespaces collapse into one node.
- **Relationships between individuals.** A triple whose object is a resource
  (`:x :knows :y`) does not create an edge; the object lands as a string
  property. The only edges the bridge creates are `subClassOf`.
- A second `rdf:type` on an individual, all but the last value of a
  multi-valued predicate, `rdfs:label`/`rdfs:comment` on classes,
  `owl:Ontology` headers, restrictions and equivalence axioms. Each dropped
  triple adds one to `ImportReport::triples_skipped`; imported properties do
  not, so the count is exactly what was lost.
- **Parser coverage.** No `a` keyword, language tags, blank nodes, `@base` or
  collections, and malformed Turtle does not fail — it shows up as a wrong
  count.
- **On export:** every edge except `subClassOf`, nodes without an `iri`
  property, non-scalar properties; local names are sanitized to ASCII and the
  namespace is a fixed `http://example.org/ontology#`.

**What to do with that:** keep the triple store as the system of record for
RDF and materialize into NopalDB the subgraph you want to query, reason over
or embed. NopalDB will not grow SPARQL or named graphs; a faithful bridge
(real grammar with errors, IRI identity, edges for resource-valued triples, a
symmetric exporter) is tracked in the public roadmap — issues
[#69](https://github.com/sharop/nopaldb/issues/69) and
[#70](https://github.com/sharop/nopaldb/issues/70).

## Re-ingesting a source: keeping node, text and vector in step

The common shape for a derived, rebuildable index is: read a source, write one
node per item, re-run whenever the source changes. Two rules make that cheap
and correct.

**1. Address nodes by a business key, not by id.** `upsert_node` keys on
`(label, key_property, value)` — it creates when absent, updates when changed,
and does nothing when identical. Re-running over unchanged data costs zero
writes, so re-ingestion is safe to run on a timer.

**2. Tie the embedding to the content it came from.** The engine keeps the
node's properties, its full-text document and its vector consistent on every
overwrite and delete: a value the node no longer has is retracted from the
indexes, and a deleted node leaves nothing behind. What the engine cannot know
is whether a vector is still *semantically* current — it never re-runs your
embedding model. Record what was embedded:

```python
import hashlib

content = fragment.text
digest = hashlib.sha256(content.encode()).hexdigest()

outcome, node_id = graph.upsert(
    "Fragment", "ref",
    {
        "ref": fragment.ref,
        "body": content,
        "content_hash": digest,
        "embedded_hash": digest,        # what the stored vector was built from
        "embed_model": "e5-large-v2",   # and by which model
    },
    vector=embed(content), model="e5-large-v2",
)
# outcome is "created" | "updated" | "unchanged" — an unchanged re-run writes
# nothing at all, so you can skip embedding entirely when nothing moved.
```

On the next run, re-embed only when `content_hash != embedded_hash` or when
`embed_model` differs from the model you are using now — a plain property
lookup finds the stale ones. Both fields live on the node, so the check
survives a restart and needs no side table.

Changing the embedding model is the same operation with a different trigger:
write vectors under the new model name, and query with that name. Vectors are
stored per `(node, model)`, so both generations coexist until you drop the old
one.

**What the engine does not do:** it does not decide when your content changed,
and it does not re-embed. Everything else in the cycle — retracting stale index
entries on overwrite, replacing the full-text document rather than adding a
second one, purging a deleted node's vectors so a rebuilt HNSW index cannot
resurrect it — is the engine's job and happens on every write path, including
transactional commits and WAL replay after a crash.
