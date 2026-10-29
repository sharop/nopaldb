# NopalDB Python API cheat sheet

One page, one line per operation, signatures copied from `nopaldb/python/nopaldb/nopaldb.pyi` (the stub that `check_stubs.py` keeps in sync with the extension). Everything below is a method of `nopaldb.Graph` unless it is a static constructor or the row says otherwise (`add_node` / `add_edge` belong to a `Transaction` or a `BulkLoader`). A CI guard (`check_cheatsheet.py`) fails if a name here stops existing.

```python
import nopaldb
g = nopaldb.Graph.open("./data")
```

## Open and close

| method | what it does |
|---|---|
| `open(path: str) -> "Graph"` | Open or create a database directory (redb by default). |
| `open_with_profile(path: str, profile: str = "default") -> "Graph"` | Open with a profile: `"default"`, `"mobile"` or `"server"` (redb page cache 1 GiB / 16 MiB / 256 MiB). |
| `in_memory() -> "Graph"` | Ephemeral graph for tests. |
| `migrate(src: str, dst: str, src_engine: str = "auto", dst_engine: str = "auto", profile: str = "default") -> dict[str, Any]` | Verified copy of a closed database directory; returns the report with `"verified"`. |
| `checkpoint() -> None` | Force the durable checkpoint. |
| `close() -> None` | Close cleanly (also via `with`): checkpoint, HNSW dump, full-text commit and its merges joined; nothing writes to the directory after it returns. |

## Write

| method | what it does |
|---|---|
| `add_node(label: str, properties: Props) -> str` | `Transaction` / `BulkLoader`: one node; returns its id. `properties` is required (`{}` for none). |
| `add_edge(source: str, target: str, edge_type: str, properties: Optional[Props] = None) -> str` | `Transaction` / `BulkLoader`: one edge between existing nodes; returns its id. |
| `upsert(label: str, key: str, props: Props, vector: Optional[list[float]] = None, model: Optional[str] = None, links: Optional[list[dict[str, Any]]] = None,) -> tuple[str, str]` | Idempotent write by `(label, key)`; `vector`/`model` attach an embedding, `links` create edges in the same call. Each link is `{"type": str, "target_label": str, "target_key": str, "target_key_value": Any, "props": dict?, "stub": bool?}`; with `stub` a missing target is created as `{target_key: value}` and filled by its own upsert later. Returns `(outcome, node_id)`, outcome `"created" / "updated" / "unchanged"`. |
| `upsert_many(requests: list[dict[str, Any]]) -> list[tuple[str, str]]` | Many upserts, one transaction per chunk of 1 024; each request is `{"label", "key", "props", "vector"?, "model"?, "links"?}` (the arguments of `upsert` as keys); returns `(outcome, node_id)` per request, in order. The GraphRAG [recipe](../GRAPHRAG.md#recipe-end-to-end) ingests with it. |
| `delete(label: str, key: str, value: Property) -> Optional[str]` | Delete the node of `label` whose property `key` equals `value`; returns its id, `None` if none. |
| `begin_transaction(isolation: Optional[str] = None) -> "Transaction"` | Explicit transaction (`tx.add_node`, `tx.add_edge`, `tx.commit()`, `tx.rollback()`). Isolation: `"read_committed"` (default), `"read_uncommitted"`, `"repeatable_read"`, `"serializable"`. |

## Embeddings and indexes

| method | what it does |
|---|---|
| `add_node_embedding(node_id: str, vector: list[float], model: str) -> None` | One vector for a node and model; the HNSW index is built or loaded on the first search. |
| `add_node_embeddings(model: str, items: list[tuple[str, list[float]]]) -> int` | Many vectors in one call (batch, much faster). |
| `get_node_embedding(node_id: str, model: str) -> list[float]` | Read a stored vector. |
| `create_index(label: str, property: str, index_type: str = "hash", analyzer: dict[str, Any] | None = None,) -> str` | Property index: `"hash"`, `"btree"` or `"fulltext"` (with `analyzer={"language": ...}`). |
| `drop_index(index_name: str) -> None` | Remove an index. |
| `list_indexes() -> list[tuple[str, str, str, str]]` | What exists. |
| `embedding_index_stats(model: str) -> HnswStats | None` | Size, tombstones and load time of the HNSW index. |

## Search

| method | what it does |
|---|---|
| `knn_nodes(query_vector: list[float], k: int, model: str, ef_search: Optional[int] = None, hydrate: Literal[False] = False) -> list[tuple[str, float]]` | Nearest neighbours by vector. |
| `search_hybrid(text: Optional[str] = None, vector: Optional[list[float]] = None, model: Optional[str] = None, k: int = 10, ef: Optional[int] = None, label: Optional[str] = None, props: Optional[Props] = None, text_index: Optional[str] = None, rrf_k: float = 60.0, hydrate: bool = False, candidates: Optional[list[str]] = None,) -> list[dict[str, Any]]` | BM25 + vector fused with RRF; `label`/`props` filter; `candidates=[ids]` is the universe to search in (#214). |
| `search_hybrid_explain(text: Optional[str] = None, vector: Optional[list[float]] = None, model: Optional[str] = None, k: int = 10, ef: Optional[int] = None, label: Optional[str] = None, props: Optional[Props] = None, text_index: Optional[str] = None, rrf_k: float = 60.0, candidates: Optional[list[str]] = None,) -> dict[str, Any]` | Same search plus the trace: per-hit raw scores and ranks, `text_path`, `vector_path`, `vector_exact`, `candidate_set`. |

## GraphRAG retrieval

| method | what it does |
|---|---|
| `get_node(id: str) -> NodeDict | None` | Point read by id; `None` if missing. |
| `get_nodes(ids: list[str]) -> list[NodeDict | None]` | Many point reads, input order kept. |
| `neighborhood(ids: list[str], depth: int = 1, direction: str = "out", edge_types: Optional[list[str]] = None, labels: Optional[list[str]] = None, max_nodes: int = 1000, max_edges_per_node: Optional[int] = None, rank: str = "bfs", alpha: float = 0.15, iterations: int = 20, candidate_factor: int = 5, seed_weights: Optional[dict[str, float]] = None, within: Optional[list[str]] = None,) -> NeighborhoodResult` | BFS expansion from seed ids with depth, direction, edge-type and label filters, `max_nodes`; `within=[ids]` restricts the walk to a set. |
| `community_members(community_key: str, via: Optional[str] = None) -> list[str]` | Ids of a materialized community; with `via="MENTIONS"`, the chunks that mention its members. Pass as `search_hybrid(candidates=…)` to search inside the community. |
| `export_graph(label: Optional[str] = None) -> dict[str, list[dict[str, Any]]]` | The graph (or the subgraph induced by `label`) as `{"nodes", "edges"}` with native property types, read as one consistent state. `nopaldb.networkx.to_networkx(graph)` builds a `MultiDiGraph` on it ([NETWORKX.md](NETWORKX.md)). |
| `neighbors(id: str, direction: str = "out", edge_types: Optional[list[str]] = None) -> list[NodeDict]` | Direct neighbours. |
| `degree(id: str, direction: str = "both") -> int` | Degree of a node. |

## NQL

| method | what it does |
|---|---|
| `execute_nql(query: str) -> "NqlResult"` | Run any NQL statement; the result has `.kind`, `.query` (rows), `.write`, `.explain`, `.profile`, `.message`, `.summary` and, since 0.9.4, `.data` (the CSV/JSON text of an `export`, or the path it wrote for Arrow/Parquet). |

## Operations

| method | what it does |
|---|---|
| `get_stats() -> Stats` | Structured stats of storage, WAL, indexes, graph. |
| `last_commit_timestamp() -> int` | Logical timestamp of the latest applied write (0.10). Pass it to NQL `find … at timestamp <ts>` to read the graph as it was then (0.10.2). |
| `gc_horizon() -> int` | How far the MVCC GC collected; history from here on is complete (0.10). |
| `get_schema() -> dict[str, Any]` | Labels, edge types and their properties. |
| `to_arrow(label: str | None = None) -> bytes` | Nodes as an Arrow IPC stream; properties only with `label` ([ARROW_EXPORT.md](ARROW_EXPORT.md)). |
| `to_arrow_complete(label: str | None = None) -> tuple[bytes, bytes]` | Nodes and edges with properties; with `label`, the induced subgraph. |
| `bulk_loader(batch_size: int) -> "BulkLoader"` | Batched loader for large ingestions. |
| `storage_engine() -> str` | Which engine the open database uses. |

## Ontology reasoning (`nopaldb.ELReasoner`)

Methods of `ELReasoner`, not of `Graph`. Ids are class UUID strings; ⊥ is `ELReasoner.NOTHING`.

| method | what it does |
|---|---|
| `from_graph(graph: Graph) -> ELReasoner` | Reasoner over the graph's classes, `subClassOf` and `disjointWith` edges (what `import_turtle` writes). |
| `assert_subclass(sub: str, super_class: str) -> list[Inference]` | `sub ⊑ super_class`; with `NOTHING` as the superclass, declares `sub` unsatisfiable. |
| `assert_disjoint(classes: list[str]) -> list[Inference]` | Pairwise disjoint classes (`owl:AllDisjointClasses`). |
| `classify_all() -> list[Inference]` | Saturate CR1–CR4; returns what is new. |
| `unsatisfiable_classes() -> list[str]` | Every class below ⊥. |
| `why_unsatisfiable(node: str) -> dict[str, Any] | None` | The cause: the disjoint pair, an asserted ⊥, or an existential. |

## Names that do NOT exist

Assistants tend to invent these. Use the real one:

| not a method | use instead |
|---|---|
| `execute(...)`, `query(...)`, `run(...)` | `execute_nql(query)` |
| `upsert_node(...)` | `upsert(label, key, props, vector=, model=, links=)` |
| `upsert_edge(...)` | `upsert(..., links=[{...}])`, or `add_edge(...)` |
| `create_embedding_index(...)`, `build_index(...)` | nothing to call: the HNSW index is built or loaded on the first `knn_nodes` / `search_hybrid` |
| `add_embedding(...)`, `set_embedding(...)` | `add_node_embedding(node_id, vector, model)` / `add_node_embeddings(model, items)` |
| `search(...)`, `vector_search(...)` | `knn_nodes(...)` or `search_hybrid(...)` |
| `expand(...)`, `traverse(...)` | `neighborhood(ids, depth, ...)` |
| `open_read_only(...)` (Python) | Rust only today; from Python copy the database cold and open the copy |

## See also

- [QUICKSTART](QUICKSTART.md) · [API reference](API_REFERENCE.md) · [NQL guide](NQL_GUIDE.md)
- [NQL reference (EN)](../en/NQL_REFERENCE.md) · [Referencia NQL (ES)](../es/NQL_REFERENCIA.md)
- [Hybrid search](../HYBRID_SEARCH.md) · [GraphRAG](../GRAPHRAG.md) · [Embeddings](../EMBEDDINGS.md)
