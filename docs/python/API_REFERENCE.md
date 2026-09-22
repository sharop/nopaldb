# NopalDB Python API Reference
# Referencia del API de NopalDB Python

**[English](#english)** | **[Español](#español)**

---

<a name="english"></a>
## 🔷 English

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

**Backends and availability (0.6.0):**

| `engine` | In the PyPI wheel? | Meaning |
|---|---|---|
| `"auto"` (default) | — | the engine of the database already at `path`; redb for a new directory. What `Graph.open` uses |
| `"redb"` | yes | the default engine since 0.6.0 |
| `"sled"` | yes | the 0.5.x engine; kept to open and migrate existing databases (at least through 0.7) |

A database created with 0.5.x opens unchanged: `"auto"` sees the sled files
and uses sled (a warning suggests migrating). Asking explicitly for the other
engine on an existing database raises an error that says how to migrate;
asking for a backend this build does not contain raises `ValueError` naming
what is available. `graph.storage_engine()` returns `"redb"` or `"sled"`.

**Returns:** Graph instance

---

##### `Graph.migrate(src: str, dst: str, src_engine: str = "auto", dst_engine: str = "auto", profile: str = "default") -> dict`

Copy a **closed** database directory to another engine, byte for byte, and
verify the copy (counts and checksums per keyspace, re-scanned on the
destination; size and checksum per file for `indexes/` and `hnsw/`).
Time-travel, embeddings, clocks, user indexes with their analyzers and the
HNSW dump travel with it (indexes and HNSW since 0.6.3).

```python
report = nopaldb.Graph.migrate("data/old_sled.db", "data/new_redb.db")
assert report["verified"]
graph = nopaldb.Graph.open("data/new_redb.db")   # storage_engine() == "redb"
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

`"redb"` or `"sled"`: the engine this handle actually opened.

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

##### `begin_transaction() -> Transaction`

Start a new transaction.

```python
tx = graph.begin_transaction()
# ... add nodes/edges ...
tx.commit()
```

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

**Raises:** RuntimeError if query fails

---

#### Export Methods

##### `to_arrow(label: str = None) -> bytes`

Export nodes to Apache Arrow format.

```python
nodes_bytes = graph.to_arrow(label="Person")
```

**Returns:** Arrow IPC stream (bytes)

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

Export complete graph (nodes + edges) to Arrow.

```python
nodes_bytes, edges_bytes = graph.to_arrow_complete()
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

##### `add_node(label: str, properties: dict = None) -> str`

Add a node to the graph.

```python
tx.add_node("Person", {"name": "Alice"})
```

**Properties Support:**
- `str`, `int`, `float`, `bool`, `None`, `bytes`

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`

Add an edge between two nodes.

```python
tx.add_edge(id1, id2, "KNOWS", {"since": 2024})
```

##### `delete_node(id: str)`
Mark a node for deletion.

##### `delete_edge(id: str)`
Mark an edge for deletion.

##### `commit()` / `rollback()`
Finalize or cancel the transaction.

---

### NqlResult Class

- `kind`: `"query" | "write" | "index" | "explain" | "profile" | "export" | "message"`
- `query`: `QueryResult | None`
- `write`: `dict | None`
- `explain`: `str | None`
- `profile`: `ProfileResult | None`
- `message`: `str | None`
- `summary`: resumen legible del resultado

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

<a name="español"></a>
## 🔷 Español

### Clase Graph

Punto de entrada principal para trabajar con NopalDB.

#### Métodos de Consulta

##### `execute_nql(query: str) -> NqlResult`

Ejecuta una consulta NQL. Soporta tanto lectura (`FIND`) como escritura (`ADD`, `UPDATE`, `DELETE`).

```python
# Lectura
res = graph.execute_nql("find p.nombre from (p:Persona)")
for row in res.query:
    print(row["p.nombre"])

# Escritura
res = graph.execute_nql("add (p:Persona {nombre: 'Alice'})")
print(res.write)
```

**Parámetros:**
- `query` (str): Cadena de consulta NQL

**Ver también:** [Guía NQL](NQL_GUIDE.md)

---

#### Métodos de Exportación

##### `to_arrow(label: str = None) -> bytes`

Exporta nodos a formato Apache Arrow.

##### `edges_to_arrow() -> bytes`

Exporta aristas a formato Apache Arrow.

---

### Clase Transaction

#### Métodos

##### `add_node(label: str, properties: dict = None) -> str`
Agrega un nodo en la transacción actual.

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`
Agrega una arista. Soporta propiedades en la arista.

---

### Reference / Referencia

**Version:** 0.2.0  
**Updated:** January 2026
