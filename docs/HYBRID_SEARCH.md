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
allowed-set (label scan ∩ property-index lookups) that both paths intersect.
v1 supports label + equality; ranges/OR are a follow-up.

**Nothing outside the allowed-set is ever returned**, at any selectivity. That
is a hard guarantee, independent of how the vector path resolved the query.

### How the vector path applies the filter

Recall — *finding all the allowed neighbours that exist* — depends on which
branch runs. The planner picks one from the allowed-set cardinality and the
index size:

| Situation | Branch | Recall |
|---|---|---|
| index ≤ 1024 vectors | exact scan over every point, then filter | exact |
| allowed-set ≤ 1024 within a larger index | exact cosine over just the allowed vectors | exact |
| larger allowed-set | HNSW walk with the predicate applied natively during traversal, escalating `ef_search` until `k` allowed hits are collected (capped) | approximate |

The middle row is the important one: with a selective filter, scoring the
allowed vectors directly is both exact *and* cheaper than walking a large
graph, so a narrow query does not pay for the index being big.

Only the last row is approximate. If it collects fewer than `k` allowed hits it
stops at the escalation cap rather than walking the whole index — so a result
shorter than `k` means "no more allowed neighbours were found", which is not
always the same as "no more exist".

### Cost

A filtered HNSW walk gets *more* expensive as the filter gets *more*
selective: fewer allowed points means the traversal explores further before its
result heap fills. Measured on a 100k-vector index (dim 32, `k=10`,
`ef_search=30`, release build), recall was 10/10 at every selectivity below:

| allowed-set | share of index | per query |
|---:|---:|---:|
| 105 | 0.1% | 74 ms |
| 988 | 1% | 14 ms |
| 1,109 | 1.1% | 15 ms |
| 9,927 | 10% | 2.2 ms |
| 100,000 | 100% | 0.5 ms |

This is why the exact branch exists, and why its cutoff sits where it does:
below ~1024 allowed nodes, reading those vectors and scoring them directly
costs about the same as the walk — and is exact instead of approximate. Above
it the walk is the cheaper option. Queries whose allowed-set lands just over
the cutoff on a very large index are the costliest case; partitioning the index
by the filtering property is the answer there.

> **Changed in 0.5.5.** Before, the vector path fetched `k × 4` *global*
> neighbours and filtered them afterwards. With a selective filter the allowed
> neighbours were often not among those candidates, so the search silently
> returned fewer results than it should have — measured recall dropped to
> **0** at ~1% selectivity while the exact answer had `k` hits available.
> Output was never wrong (nothing disallowed leaked), just incomplete.

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
