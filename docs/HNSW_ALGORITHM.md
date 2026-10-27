# HNSW In NopalDB

HNSW (Hierarchical Navigable Small World) is the approximate nearest-neighbor
index used by NopalDB embedding features. It is useful when you need top-k
similar vectors without scanning every stored embedding.

## Problem

Given many vectors, find the `k` nearest vectors to a query vector.

Exact search compares the query with every vector. That is simple and accurate,
but it scales linearly with the number of embeddings. HNSW trades exactness for
fast approximate search.

## How HNSW Works

HNSW stores vectors in a multi-layer graph:

```
layer 2:      A -------- D
              |          |
layer 1:      A --- B -- D
              |     |    |
layer 0:      A  B  C  D  E  F
```

Upper layers contain fewer nodes and longer links. Search starts at the highest
layer, moves greedily toward the query region, then performs a wider search in
layer 0.

The main tuning parameters are:

| Parameter | Meaning |
|-----------|---------|
| `M` | Maximum graph neighbors per node |
| `ef_construction` | Candidate width while building the index |
| `ef_search` | Candidate width while querying |

Higher values generally improve recall and increase memory or CPU cost.
NopalDB's defaults are `M` = 24, `ef_construction` = 400 and `ef_search` = 30.

## What NopalDB adds around HNSW

- **Exact path for small indexes:** with at most 1024 points
  (`EXACT_SEARCH_THRESHOLD`) a search compares the query with every point.
  Exact, deterministic, and cheaper than a graph walk at that size; such
  indexes are not persisted (rebuilding them costs milliseconds).
- **Parallel build:** `build_batch` inserts in parallel from 128 points up;
  below that it inserts serially so the graph does not depend on the number
  of threads.
- **Orphan repair:** HNSW pruning can leave a point no search reaches. After
  a build and after each insert every new point searches for itself; the ones
  that do not find themselves are compared directly on every search and saved
  in the dump.
- **Internal `hnsw_rs` 0.3.4 copy** (`src/embeddings/hnsw_rs`): since 0.9.3
  it carries a lock-order fix for a deadlock in parallel builds (#288). The
  dump format is the crate's.

Details, measurements and persistence rules are in
[EMBEDDINGS.md](EMBEDDINGS.md#hnsw-index--approximate-nearest-neighbor-search).

## Distance

NopalDB embedding search uses cosine distance:

```
distance(a, b) = 1 - cosine_similarity(a, b)
```

Cosine distance works well for many text embeddings because it compares vector
direction rather than raw magnitude.

## Rust API

The low-level HNSW wrapper lives under embedding features.

```rust
let mut index = HnswIndex::new("demo-model", 384, 100_000);

index.insert(node_id, vector)?;

let matches = index.search_knn(&query_vector, 10)?;
```

Filtered search combines vector similarity with graph predicates supplied by the
caller:

```rust
let matches = index.search_knn_filtered(
    &query_vector,
    10,
    30,
    |node_id| allowed_nodes.contains(node_id),
)?;
```

## Python API

The wheel on PyPI (`pip install nopaldb`) includes it. To build the wrapper
from source, use the `python-full` feature:

```bash
cd nopaldb
maturin develop --release --features python-full
```

```python
graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "demo-model")

matches = graph.knn_nodes(
    [0.1, 0.2, 0.3],
    10,
    "demo-model",
)
```

## Operational Notes

- Keep vector dimensions consistent per model name.
- Store the model name with every embedding so mixed-model data does not share
  one vector space accidentally.
- Larger `ef_search` values can improve result quality at query time.
- Rebuild or reload the index when changing embedding model or dimensions.

## Related Docs

- [Embeddings](EMBEDDINGS.md)
- [Feature Tiers](FEATURE_TIERS.md)
