# NopalDB Python API cheat sheet

One page, one line per operation, signatures copied from `nopaldb/python/nopaldb/nopaldb.pyi` (the stub that `check_stubs.py` keeps in sync with the extension). Everything below is a method of `nopaldb.Graph` unless it is a static constructor. A CI guard (`check_cheatsheet.py`) fails if a name here stops existing.

```python
import nopaldb
g = nopaldb.Graph.open("./data")
```

## Open and close

| method | what it does |
|---|---|
| `open(path: str) -> "Graph"` | Open or create a database directory (redb by default). |
| `in_memory() -> "Graph"` | Ephemeral graph for tests. |
| `migrate(src: str, dst: str, src_engine: str = "auto", dst_engine: str = "auto", profile: str = "default") -> dict[str, Any]` | Copy a database between engines, verified. |
| `checkpoint() -> None` | Force the durable checkpoint. |
| `close() -> None` | Close cleanly (also via `with`): checkpoint, HNSW dump, full-text commit and its merges joined; nothing writes to the directory after it returns. |

## Write

| method | what it does |
|---|---|
| `add_node(label: str, properties: Props) -> str` | One node; returns its id. |
| `add_edge(source: str, target: str, edge_type: str, properties: Optional[Props] = None) -> str` | One edge between existing nodes. |
| `upsert(label: str, key: str, props: Props, vector: Optional[list[float]] = None, model: Optional[str] = None, links: Optional[list[dict[str, Any]]] = None,) -> tuple[str, str]` | Idempotent write by `(label, key)`; `vector`/`model` attach an embedding, `links` create edges in the same call. Each link is `{"type": str, "target_label": str, "target_key": str, "target_key_value": Any, "props": dict?, "stub": bool?}`; with `stub` a missing target is created as `{target_key: value}` and filled by its own upsert later. Returns `(outcome, node_id)`, outcome `"created" / "updated" / "unchanged"`. |
| `upsert_many(requests: list[dict[str, Any]]) -> list[tuple[str, str]]` | Many upserts, one transaction per chunk of 1 024; each request is `{"label", "key", "props", "vector"?, "model"?, "links"?}` (the arguments of `upsert` as keys); returns `(outcome, node_id)` per request, in order. The GraphRAG [recipe](../GRAPHRAG.md#recipe-end-to-end) ingests with it. |
| `delete(label: str, key: str, value: Property) -> Optional[str]` | Delete by `(label, key, value)`. |
| `begin_transaction(isolation: Optional[str] = None) -> "Transaction"` | Explicit transaction with an isolation level. |

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
| `execute_nql(query: str) -> "NqlResult"` | Run any NQL statement; the result has `.query` (rows), `.write`, `.explain`, `.profile`. |

## Operations

| method | what it does |
|---|---|
| `get_stats() -> Stats` | Structured stats of storage, WAL, indexes, graph. |
| `get_schema() -> dict[str, Any]` | Labels, edge types and their properties. |
| `to_arrow(label: str | None = None) -> bytes` | Nodes as an Arrow table. |
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
