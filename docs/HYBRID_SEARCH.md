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

RRF is **rank-based**, which is what lets it fuse two branches whose numbers
are not comparable: a BM25 score and a cosine distance live on different
scales, but their rank orders do not. A node that ranks well in *both* paths
outranks one strong in only one. The raw scores are still available for
inspection — see [Explaining a result](#explaining-a-result).

## API

- **Rust:** `Graph::search_hybrid(HybridQuery) -> Vec<HybridHit>`.
- **Python:** `graph.search_hybrid(text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0)` → `list[dict]` of `{node_id, score, text_rank, vector_rank}`.
- **Rust:** `Graph::search_hybrid_explain(HybridQuery) -> HybridExplain` — same search, plus why each hit ranked where it did.
- **Python:** `graph.search_hybrid_explain(...)` with the same arguments → `dict`.
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

## Explaining a result

The RRF score orders results but does not explain them. It is a sum of
reciprocal *ranks*, so it cannot tell you whether a document rose through
text, through vectors, or both — nor how strongly. `search_hybrid_explain`
runs the same search and keeps the raw numbers the fusion consumes and throws
away.

```python
e = graph.search_hybrid_explain(text="cactus", vector=[...], model="e5-large", k=5)

for h in e["hits"]:
    print(h["node_id"], h["text_rank"], h["text_score"], h["vector_rank"], h["vector_distance"])

print(e["vector_path"], e["text"], e["vector"])
```

Per hit: `text_score` is the raw BM25 score, `vector_distance` the cosine
distance (0 = identical). Each is `None` when the hit did not come through
that branch — "no score" rather than a zero that would read as "scored badly".

Globally it reports the configuration that was actually **used**, including
what you did not choose: the resolved `text_index`, the effective `ef_search`,
`candidates` (= `k × overfetch`) and `allowed_set_size` when a filter applied.

Each branch reports `{requested, returned, underfilled}`. Underfill is the
question a short result cannot answer on its own — and `vector_path` is what
decides how to read it:

| `vector_path` | Meaning of a short result |
|---|---|
| `unfiltered` / `exact_over_allowed` | exact: there genuinely are no more |
| `hnsw_filtered` | approximate: no more were *found*, which is not the same |

`search_hybrid` is the same computation with the trace dropped — it delegates
to the explaining version rather than duplicating it, so asking for the
explanation cannot change the result.

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

- The fusion itself uses rank, not score — that is what makes RRF work across
  two branches whose numbers are not comparable. The raw scores are still
  available for inspection via `search_hybrid_explain`. Choose the right
  property when creating the full-text index (it is per-property).
- `search_hybrid` sees **committed** state; a freshly added embedding is visible
  after its `add_node_embedding` invalidates the cached HNSW index.
- Follow-ups: per-path weights; range/OR filters; wiring `hybrid()`'s
  parameters in NQL, which are still hardcoded.
