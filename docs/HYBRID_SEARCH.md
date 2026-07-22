# Hybrid search

`search_hybrid` fuses the two retrieval paths NopalDB already has —full-text
(tantivy) and vector (HNSW)— with **Reciprocal Rank Fusion**, plus an optional
label/property filter. It is how you query a second brain that was written with
[`upsert`](UPSERT.md): text relevance and semantic similarity in one ranked list.

## How RRF works

Each path returns candidates in rank order. A document's fused score is:

```
score(d) = Σ_path 1 / (rrf_k + rank_path(d))        # rrf_k default 60
```

RRF is **rank-based**, which fits both indexes: the full-text index returns node
ids ordered by relevance (no raw scores), and HNSW returns them ordered by
distance. A node that ranks well in *both* paths outranks one strong in only one.

## API

- **Rust:** `Graph::search_hybrid(HybridQuery) -> Vec<HybridHit>`.
- **Python:** `graph.search_hybrid(text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0)` → `list[dict]` of `{node_id, score, text_rank, vector_rank}`.
- **MCP:** the `search_hybrid` tool (read-only).
- **NQL:** `where hybrid(n, "text", "ref_name", "model")` — see below.

Provide `text`, `vector`, or both. `vector` requires `model`. The full-text path
needs an index created with `create index on <Label>(<property>) type fulltext`;
if `text_index` is omitted it is auto-discovered (preferring one whose label
matches the filter).

```python
hits = graph.search_hybrid(
    text="graph memory", vector=[...], model="e5-large",
    k=10, label="Chunk", props={"kind": "book"},
)
for h in hits:
    print(h["node_id"], h["score"], h["text_rank"], h["vector_rank"])
```

## Filter

`filter = {label?, props: [equalities]}` (AND). It is applied as a precomputed
allowed-set (label scan ∩ property-index lookups) so the vector path filters
*before* fetching, not after. v1 supports label + equality; ranges/OR are a
follow-up.

## NQL `hybrid()`

`hybrid(n, "text", "ref_name", "model")` in a WHERE clause filters the pattern to
the top-K hybrid results. The vector is the embedding of the reference node
resolved by its `name` property (the same convention as `similar_to`); K comes
from the query `LIMIT` (default 10). The FROM pattern's own label filter narrows
the result downstream.

```nql
find n.name, n.body
from (n:Chunk)
where hybrid(n, "graph memory", "current_query", "e5-large")
limit 10
```

## Limits & notes (v1)

- Full-text exposes rank, not raw score — RRF only needs rank, so this is fine,
  but choose the right property when creating the index (it is per-property).
- `search_hybrid` sees **committed** state; a freshly added embedding is visible
  after its `add_node_embedding` invalidates the cached HNSW index.
- Follow-ups: per-path weights + raw full-text score (M1-5b); range/OR filters
  (M1-5c).
