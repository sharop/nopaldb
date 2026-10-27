# NopalDB Python API Reference

> En español: [docs/es/API_PYTHON.md](../es/API_PYTHON.md). Exact signatures
> of every method: [API cheat sheet](API_CHEATSHEET.md) and the stub
> `nopaldb/python/nopaldb/nopaldb.pyi`.

### Graph Class

Main entry point for working with NopalDB.

#### Constructor Methods

##### `Graph.open(path: str) -> Graph`

Open or create a persistent graph database.

```python
graph = nopaldb.Graph.open("data/my_graph.db")
```

**Parameters:**
- `path` (str): Path to database directory

**Returns:** Graph instance

**See also:** [Configuration Guide](CONFIGURATION.md) for details on default settings.

---

##### `Graph.open_with_options(path: str, engine: str = "auto", profile: str = "default", on_progress=None) -> Graph`

Open a database with an explicit storage backend and tuning profile.

```python
graph = nopaldb.Graph.open_with_options("data/my_graph.db", engine="redb", profile="default")
```

**Parameters:**
- `path` (str): Path to database directory
- `engine` (str): storage backend — see the table below
- `profile` (str): `"default"` | `"mobile"` | `"server"`

**Engines (0.8.0):** `"auto"` (default, what `Graph.open` uses) and `"redb"`
are the same thing: redb is the only engine. sled was removed in 0.8.0 (#141):
`engine="sled"` raises `ValueError`, and opening a database created with 0.5.x
raises an error that says it is a sled database and how to migrate it (with
0.7: `pip install "nopaldb==0.7.*"`, then `Graph.migrate`). See
[MIGRATION_0.6.md](../MIGRATION_0.6.md). `graph.storage_engine()` returns
`"redb"`.

**Returns:** Graph instance

---

##### `Graph.migrate(src: str, dst: str, src_engine: str = "auto", dst_engine: str = "auto", profile: str = "default") -> dict`

Copy a **closed** database directory, byte for byte, and verify the copy (counts and checksums per keyspace, re-scanned on the
destination; size and checksum per file for `indexes/` and `hnsw/`).
Time-travel, embeddings, clocks, user indexes with their analyzers and the
HNSW dump travel with it (indexes and HNSW since 0.6.3).

```python
report = nopaldb.Graph.migrate("data/kb.db", "data/kb_copy.db")
assert report["verified"]
graph = nopaldb.Graph.open("data/kb_copy.db")
```

Preconditions: no open `Graph` on either directory; the source was opened and
closed at least once with NopalDB (its WAL is applied); the destination is
empty or absent. A failed verification raises and the destination must not be
used. Returns a dict with three sections:

```python
{
  "keyspaces": [{"name": str, "pairs": int, "bytes": int}, ...],
  "verified": bool,                       # KV counts/checksums and sidecar files match
  "indexes": [{"name": str, "label": str, "property": str,
               "type": "Hash" | "BTree" | "FullText" | "Taxonomy",
               "analyzer": str | None}],  # user indexes that travelled
  "sidecars": [{"dir": "indexes" | "hnsw", "files": int, "bytes": int}],
  "hnsw_copied": bool,                    # False: no dump in the source; rebuilt on first search
}
```

See [MIGRATION_0.6.md](../MIGRATION_0.6.md) for what travels and what is rebuilt.

##### `graph.checkpoint() -> None`

Make everything applied so far durable in the storage engine and truncate the
WAL, so the next `Graph.open` replays nothing. It runs on its own when the WAL
passes 16 MiB and on `close()`; call it after a large load if you want the
reopen to be instant. Every acknowledged write is recoverable with or without
it (see [DURABILITY.md](../DURABILITY.md) § From Python). Raises on a
read-only graph. `graph.get_stats()["wal"]["bytes"]` reports the current WAL size.

##### `graph.get_stats() -> dict`

Operational state of the database in one call: `graph` (counts, per label
and type), `storage` (engine, profile, directory, read-only), `wal` (bytes,
checkpoint threshold, checkpoints this session, last checkpoint), `recovery`
(what the open that created this handle did: WAL records read, operations
replayed, crash recovery, adjacency rebuilt, milliseconds per phase),
`indexes` (name, label, property, type, size, analyzer), `hnsw` (one entry
per vector index in cache) and `gc` (auto scheduler state, last run). Native
types; typed as `nopaldb.Stats` in the stub. Cheap: nothing is scanned. The
0.6.x flat string keys (`total_nodes`, `total_edges`, `avg_degree`,
`storage_engine`, `wal_bytes`) stay at the top level for one more minor.

```python
s = graph.get_stats()
s["recovery"]["operations_replayed"], s["recovery"]["open_ms"]["total"]
[(ix["name"], ix["size"], ix["analyzer"]) for ix in s["indexes"]]
```

What to read for each symptom: [OPERATIONS.md](../OPERATIONS.md).

##### `graph.set_progress_callback(callback) -> None`

Register (or remove with `None`) a callable that receives
`{"phase": str, "done": int, "total": int | None}` while `create_index`,
`upsert_many` and `BulkLoader` run, every ~1000 items or ~250 ms. The same
callable passed as `on_progress` to `Graph.open_with_options` also sees the
open's `wal_replay`, `adjacency_rebuild` and `index_load`. It runs on a
worker thread: keep it cheap, do not call the graph from it. An exception it
raises is logged and ignored.

##### `graph.node_count() -> int` / `graph.edge_count() -> int`

Exact counts. Both walk the storage keys without deserializing a single
node or edge: O(N) in keys, allocation-free, and independent of the schema
cache (until 0.6.5 `node_count` materialized every node). For counts per
label or type read `get_stats()["graph"]` or `get_label_count(label)`.

##### `graph.bulk_loader(batch_size: int) -> BulkLoader`

High-throughput ingestion: rows are buffered and written `batch_size` at a
time, bypassing the transactional path. See [BulkLoader Class](#bulkloader-class).

##### `Graph.rebuild_indexes() -> int`

Rebuild the user indexes from `<dir>/indexes/metadata.bin` and the current
nodes, exactly as `Graph.open` does, and return how many are loaded. For
databases copied with your own tools (`Graph.migrate` already carries the
catalog): put `indexes/` in place, then call this instead of reopening.
Returns 0 when there is no catalog: indexes are declared with `create_index`,
never inferred.

---

##### `Graph.storage_engine() -> str`

`"redb"`: the engine this handle opened (the only one since 0.8.0).

---

##### `Graph.in_memory_with_options(engine: str = "auto", profile: str = "default") -> Graph`

Same options as `open_with_options`, without persistence.

---

##### `Graph.in_memory() -> Graph`

Create an in-memory graph (non-persistent).

```python
graph = nopaldb.Graph.in_memory()
```

**Returns:** Graph instance

**Note:** Data is lost when the process ends.

---

#### Transaction Methods

##### `begin_transaction(isolation: str | None = None) -> Transaction`

Start a new transaction.

```python
tx = graph.begin_transaction()                  # Read Committed
# ... add nodes/edges ...
tx.commit()

tx = graph.begin_transaction("serializable")
```

**Parameters:**
- `isolation` (str, optional): `"read_uncommitted"`, `"read_committed"`
  (the default), `"repeatable_read"` or `"serializable"`. Anything else raises
  `ValueError`. See [Configuration](CONFIGURATION.md) § Concurrency.

**Returns:** Transaction instance

**Important:** Always commit or rollback transactions.

---

#### Query Methods

##### `execute_nql(query: str) -> NqlResult`

Execute any NQL statement and return a unified `NqlResult`.

```python
# Read
result = graph.execute_nql("find p.name from (p:Person)")
rows = result.query

# Write
write_result = graph.execute_nql("add (p:Person {name: 'Alice'})")
counts = write_result.write

# Profile
profile = graph.execute_nql("profile find p.name from (p:Person)")
stats = profile.profile
```

**Parameters:**
- `query` (str): NQL query string

**Returns:** `NqlResult`

**Raises:** `RuntimeError` if the statement does not parse, fails
validation (for example a bare property as a condition, `where p.active`,
since 0.9.4) or fails to execute.

---

#### Export Methods

##### `to_arrow(label: str = None) -> bytes`

Export nodes to Apache Arrow format.

```python
nodes_bytes = graph.to_arrow(label="Person")
```

**Returns:** Arrow IPC stream (bytes). With `label`: `id`, `label` and one
column per property of those nodes. Without it: only `id`, `label` and
`property_count`, and an empty graph raises `RuntimeError`. Columns and empty
cases of every export method: [ARROW_EXPORT.md](ARROW_EXPORT.md).

---

##### `edges_to_arrow() -> bytes`

Export edges to Apache Arrow format.

```python
edges_bytes = graph.edges_to_arrow()
```

**Returns:** Arrow IPC stream (bytes). A graph without edges yields an empty
batch with the columns `id`, `source`, `target`, `edge_type` (the same
schema `to_arrow_complete` returns for that case); check `num_rows == 0`.
Until 0.6.5 this raised `ValueError`.

---

##### `to_arrow_complete(label: str = None) -> tuple[bytes, bytes]`

Export complete graph (nodes + edges, with their properties) to Arrow. With
`label`, the subgraph induced by that label: its nodes and only the edges whose
two endpoints have it, read as one consistent state (writers wait meanwhile).

```python
nodes_bytes, edges_bytes = graph.to_arrow_complete()
people, knows = graph.to_arrow_complete("Person")
```

**Returns:** Tuple of (nodes_bytes, edges_bytes)

---

### BulkLoader Class

Created with `graph.bulk_loader(batch_size)`. Buffers nodes and edges and
writes each buffer in one batch; use it as a context manager so `finish()`
runs on exit. Property values are the same as everywhere else in the API
(`str`, `int`, `float`, `bool`, `bytes`, `None` stored as null, nested
lists/tuples/dicts) and go through the one shared converter, so a row loaded
here reads back exactly like one written in a transaction.

```python
with graph.bulk_loader(10_000) as loader:
    alice = loader.add_node("Person", {"name": "Alice", "tags": ["a", "b"]})
    bob = loader.add_node("Person", {"name": "Bob"})
    edge_id = loader.add_edge(alice, bob, "KNOWS", {"since": 2020})

loader = graph.bulk_loader(10_000)      # without the context manager
loader.add_node("Person", {"name": "Carol"})
stats = loader.finish()                 # {"nodes", "edges", "duration_secs", "nodes_per_second"}
```

##### `add_node(label: str, properties: dict) -> str`

Returns the node UUID.

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`

Returns the edge UUID. Until 0.6.5 it took no properties and returned `None`.

##### `finish() -> dict`

Flushes what is buffered and makes the load durable. A finished loader
refuses more rows (`RuntimeError`). Bulk batches do not populate user
indexes; call `rebuild_indexes()` after a load if you have any.

---

### Transaction Class

Manages atomic operations on the graph.

#### Methods

##### `add_node(label: str, properties: dict) -> str`

Add a node to the graph; returns its UUID. `properties` is required (pass
`{}` for none).

```python
tx.add_node("Person", {"name": "Alice"})
```

**Properties Support:**
- `str`, `int`, `float`, `bool`, `bytes`, `None` (stored as null)
- `list`, `tuple` (stored as a list) and `dict`, nested to any depth

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`

Add an edge between two nodes.

```python
tx.add_edge(id1, id2, "KNOWS", {"since": 2024})
```

##### `commit()` / `rollback()`
Finalize or cancel the transaction.

A transaction only adds. To delete, use NQL
(`graph.execute_nql('delete (p:Person) where p.name = "Alice"')`, which also
removes the node's edges) or `graph.delete(label, key, value)`, which deletes
the node whose property `key` equals `value` and returns its id (`None` if
there is none).

---

### NqlResult Class

- `kind`: `"query" | "write" | "index" | "explain" | "profile" | "export" | "message"`
- `query`: `QueryResult | None`
- `write`: `dict | None`
- `explain`: `str | None`
- `profile`: `ProfileResult | None`
- `message`: `str | None`
- `data`: `str | None` (since 0.9.4) — the content of an `export`
  statement: the CSV/JSON text, or the path written for Arrow/Parquet
- `summary`: human-readable summary of the result

### QueryResult Class

Result of a read query (`FIND`).

#### Methods
- `__len__()`: Row count
- `__iter__()`: Iterator over rows
- `__getitem__(i)`: Get row at index
- `columns`: List of column names

### ProfileResult Class

Structured result for `PROFILE <query>`.

- `plan`
- `statement_type`
- `execution_ms`
- `rows_returned`
- `columns`
- `path_query`
- `path_metrics`

---

## Other methods

This page covers the core: opening, transactions, NQL, Arrow export, bulk
loading and the result classes. Everything else has a one-line entry in the
[API cheat sheet](API_CHEATSHEET.md), which CI checks against the stub
(`nopaldb.pyi`), and a topic page:

| area | methods | see |
|---|---|---|
| Open / close | `open_with_profile`, `in_memory_with_profile`, `close`, `with nopaldb.Graph.open(...) as g:` | [Configuration](CONFIGURATION.md) |
| Point reads and neighbours | `get_node(s)`, `get_edge(s)`, `neighbors`, `degree`, `neighborhood` | [GraphRAG](../GRAPHRAG.md) |
| Idempotent writes | `upsert`, `upsert_many`, `delete(label, key, value)` | [API cheat sheet](API_CHEATSHEET.md) |
| Indexes | `create_index`, `drop_index`, `list_indexes`, `describe_index` | [Configuration](CONFIGURATION.md) |
| Embeddings and search | `add_node_embedding(s)`, `knn_nodes`, `search_hybrid`, `search_hybrid_explain` | [Embeddings](../EMBEDDINGS.md), [Hybrid search](../HYBRID_SEARCH.md) |
| Communities | `leiden`, `leiden_hierarchy`, `materialize_communities`, `community_members`, `upsert_community_report`, `stale_reports` | [GraphRAG](../GRAPHRAG.md) |
| Schema | `get_schema`, `get_labels`, `get_edge_types`, `get_label_properties`, … | [Schema inspection](SCHEMA_INSPECTION.md) |
| Whole-graph export | `export_graph`, `nopaldb.networkx.to_networkx` | [NetworkX](NETWORKX.md) |
| Semantic | `import_turtle`, `export_turtle`, `validate_shapes`, `nopaldb.ELReasoner` | [API cheat sheet](API_CHEATSHEET.md) |

---

**Español:** la referencia en español es [docs/es/API_PYTHON.md](../es/API_PYTHON.md).
