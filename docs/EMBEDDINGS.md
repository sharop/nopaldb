# Embeddings in NopalDB

NopalDB stores dense vector embeddings alongside graph nodes. This lets you combine
structural graph queries with semantic similarity search — without leaving the database.

**Feature flag:** `embeddings` (included in `core`, `semantic`, and `full` tiers)

```toml
# Cargo.toml
nopaldb = { features = ["embeddings"] }
```

---

## The core idea

An embedding is a fixed-length array of `f32` values that captures the semantic meaning
of a node as produced by an external model (BERT, OpenAI Ada, MiniLM, etc.).

```
Node (graph)  ──►  Embedding (vector)
"Article: NopalDB comparison notes"  ──►  [0.12, -0.87, 0.34, ..., 0.05]  (768 dims)
```

Once vectors are stored you can:

- Find nodes that are *semantically similar* to a query vector.
- Combine similarity with graph structure: "documents similar to X that were
  written by authors in the same community as Y."
- Run ranking pipelines entirely in Rust without a separate vector store.

---

## API

### `Embedding` struct

```rust
pub struct Embedding {
    pub node_id: NodeId,   // UUID of the graph node
    pub vector:  Vec<f32>, // Dense vector (any dimension)
    pub model:   String,   // Model name used to generate it
    pub version: u32,      // Increment to invalidate cached embeddings
}
```

One node can have multiple embeddings — one per model — keyed by `(node_id, model)`.

### Graph methods

```rust
// Store a vector for a node
graph.add_node_embedding(node_id, vector, "minilm").await?;

// Retrieve it later
let emb = graph.get_node_embedding(node_id, "minilm").await?;
```

`add_node_embedding` returns `NodeNotFound` if `node_id` does not exist in the graph,
so the vector store and the graph always stay in sync.

### Similarity helpers on `Embedding`

```rust
// Cosine similarity — bounded [0, 1] for normalized vectors; [-1, 1] otherwise
let score = emb_a.cosine_similarity(&emb_b);

// Euclidean distance — lower = more similar
let dist = emb_a.euclidean_distance(&emb_b);
```

Cosine similarity is the right choice for text embeddings from transformer models because
those vectors point in a direction, not to a magnitude. Euclidean distance is better for
embeddings where absolute scale carries meaning (image descriptors, sensor data).

---

## Quick example — semantic document search

```rust
use nopaldb::{Graph, NopalError};
use nopaldb::types::{Node, PropertyValue};
use nopaldb::embeddings::Embedding;
use uuid::Uuid;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // --- 1. Create article nodes ----------------------------------------
    let articles = vec![
        ("Rust memory safety guide",        vec![0.9_f32, 0.1, 0.05]),
        ("Introduction to graph databases", vec![0.1,     0.9,  0.2 ]),
        ("BERT and sentence embeddings",    vec![0.15,    0.2,  0.95]),
        ("Zero-copy data with Apache Arrow",vec![0.8,     0.3,  0.4 ]),
    ];

    let mut node_ids = Vec::new();

    for (title, vector) in &articles {
        let node = Node::new("Article")
            .with_property("title", PropertyValue::String(title.to_string()));
        graph.add_node(node.clone()).await?;
        graph.add_node_embedding(node.id, vector.clone(), "minilm").await?;
        node_ids.push((node.id, title.to_string()));
    }

    // --- 2. Query vector (simulates embedding a user's search phrase) ----
    let query_vector = vec![0.85_f32, 0.15, 0.1]; // "Rust performance and safety"

    // --- 3. Load all embeddings and rank by cosine similarity -----------
    let query_emb = Embedding::new(Uuid::new_v4(), query_vector, "minilm");

    let mut results: Vec<(f32, String)> = Vec::new();

    for (node_id, title) in &node_ids {
        let emb = graph.get_node_embedding(*node_id, "minilm").await?;
        let score = query_emb.cosine_similarity(&emb);
        results.push((score, title.clone()));
    }

    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    println!("Results for query \"Rust performance and safety\":");
    for (score, title) in &results {
        println!("  {:.4}  {}", score, title);
    }
    //  1.0000  Rust memory safety guide
    //  0.9964  Zero-copy data with Apache Arrow
    //  0.3482  Introduction to graph databases
    //  0.1920  BERT and sentence embeddings

    Ok(())
}
```

---

## Combining with graph queries

The real power comes when you use the graph to *filter* before comparing vectors.
Instead of scanning all embeddings, restrict candidates first:

```rust
// Step 1 — narrow candidates with NQL
let candidates = graph.execute_nql(
    "find doc from (doc:Article) -> [:CITED_BY] -> (popular:Article)
     where popular.citations > 500"
).await?;

// Step 2 — rank by semantic similarity only within those candidates
let mut ranked: Vec<(f32, Node)> = Vec::new();
for node in candidates.nodes {
    if let Ok(emb) = graph.get_node_embedding(node.id, "minilm").await {
        ranked.push((query_emb.cosine_similarity(&emb), node));
    }
}
ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
```

This pattern — **graph filter → vector rank** — is significantly faster than ANN
(approximate nearest-neighbor) over the full corpus when your graph structure can
eliminate most candidates upfront.

---

## Storage details

Embeddings are persisted in a dedicated Sled tree (`embeddings`), separate from node
and edge data. The key is `{node_id}:{model}` and the value is the `Embedding` struct
serialized with MessagePack.

| Property | Value |
|----------|-------|
| Storage tree | `embeddings` (isolated from nodes/edges) |
| Key format | `UUID:model_name` |
| Serialization | MessagePack (rmp-serde) |
| Multiple models per node | Yes — one entry per `(node_id, model)` pair |
| Referential integrity | `add_node_embedding` validates the node exists; `delete_node` purges every vector of that node |

### Lifecycle: what the engine keeps in step, and what it does not

Deleting a node deletes its vectors, for every model. This is not just space
hygiene: the HNSW index is rebuilt from this keyspace, so a vector left behind
would put the deleted node back into search results after the next rebuild.

Overwriting a node's vector for the same model replaces it in storage **and in
the cached HNSW index** (the old point stays as a tombstone; see below). Nothing
is rebuilt.

What the engine does **not** do is notice that the *text* changed. Nothing
re-runs your embedding model, so a node whose content was rewritten keeps
serving the vector of the old content until you replace it. Store what was
embedded — a content hash and the model name, as node properties — and re-embed
when they no longer match what you have; the pattern is written out in
[ADOPTION.md](ADOPTION.md#re-ingesting-a-source-keeping-node-text-and-vector-in-step).

---

## HNSW Index — Approximate Nearest Neighbor Search

**Feature flag:** `embeddings-index` (included in `core` tier)

NopalDB includes a built-in HNSW (Hierarchical Navigable Small World) index for
O(log N) approximate nearest neighbor search, powered by `hnsw_rs 0.3`.

### Building an index

```rust
// Build from all stored embeddings for a model
let index = graph.build_embedding_index("minilm").await?;

// Search the 10 nearest neighbors
let results: Vec<(NodeId, f32)> = index.search_knn(&query_vector, 10)?;

// Filtered search: combine ANN with graph predicates
let results = index.search_knn_filtered(&query_vector, 10, 30, |nid| {
    allowed_node_ids.contains(nid)
})?;

// Same search, with the trace of how it was resolved
let outcome = index.search_knn_filtered_explained(&query_vector, 10, 30, |nid| {
    allowed_node_ids.contains(nid)
})?;
println!("{:?} ef={:?} attempts={} underfilled={}",
    outcome.path, outcome.ef_used, outcome.attempts, outcome.underfilled);
```

#### Filtered search: guarantee vs. recall

Nothing that fails the predicate is ever returned — that holds on every path.
How *completely* the allowed neighbours are found depends on the path:

- **N ≤ `EXACT_SEARCH_THRESHOLD` (1024):** distances against every point, then
  filter. Exact and deterministic.
- **Above the threshold:** the predicate is pushed into the HNSW traversal
  (`hnsw_rs`' native filter), so candidates are explored broadly but only
  allowed points enter the result heap. If fewer than `k` are collected,
  `ef_search` is multiplied by 4 and retried, up to `MAX_FILTERED_EF_SEARCH`
  (4096). Approximate.

`search_knn_filtered_explained` reports which path ran, the effective
`ef_search`, how many escalation attempts happened, and whether the result was
`underfilled` (fewer than `k` hits) — a short result cannot otherwise be told
apart from "there are no more allowed neighbours".

**When the allowed set is small and the index is large**, prefer scoring those
vectors directly (`nopaldb::embeddings::rank_exact`) over a filtered walk: it
is exact and cheaper. The index cannot make that call itself — the predicate is
an opaque closure, so it does not know the allowed cardinality. `search_hybrid`
does know it, and switches automatically; see
[HYBRID_SEARCH.md](HYBRID_SEARCH.md).

### Incremental inserts, updates and deletes

Since 0.5.19 the index the graph keeps in memory is updated in place:

- `add_node_embedding` for a new node **inserts** the vector into the cached
  index (`HnswIndex::insert`, milliseconds) instead of discarding the index;
  before 0.5.19 the next search paid a full rebuild (2.4 s at 10k vectors,
  103 s at 100k, per new embedding — see the bench table below).
- `add_node_embedding` for a node that already has a vector **replaces** it:
  the old point becomes a *tombstone* (hnsw_rs cannot delete from its graph),
  the new one is inserted.
- `delete_node` **removes** the node's points from every cached index the same
  way.
- Tombstones still occupy neighbours during a traversal, so searches over-fetch
  by their count and never return them. When they exceed 20 % of the live
  points (and at least 64), the next `get_or_build_embedding_index` rebuilds
  the index from storage, which resets them.
- If no index is cached yet, nothing happens: it is built whole on the first
  search, as before.

`graph.embedding_index_stats(model)` (Rust and Python) reports `size`,
`tombstones`, `dimension` and `needs_rebuild`.

The index handle is `SharedHnswIndex = Arc<std::sync::RwLock<HnswIndex>>`;
searches take the read lock:

```rust
let index = graph.get_or_build_embedding_index("minilm").await?;
let hits = index.read().unwrap().search_knn(&query, 10)?;
```

The programmatic API is unchanged for a standalone index:

```rust
let mut index = HnswIndex::new("minilm", 384, 100_000);
index.insert(node_id, vector)?;   // no rebuild needed
index.remove(node_id);            // logical delete: one tombstone
index.needs_rebuild();            // true past the tombstone ratio
```

### Persistence across reopens

Since 0.5.20 the HNSW graph survives a restart. The index is written with
`hnsw_rs`'s native dump to `<data_dir>/hnsw/`, three files per model:
`<base>.hnsw.graph`, `<base>.hnsw.data` and `<base>.meta` (`<base>` is the
model name made filesystem-safe plus a short hash). The `.meta` file carries
what the graph does not know: the `DataId → NodeId` map, the tombstone count,
a FNV-1a fingerprint of the model's embeddings as stored in the KV engine, and
the length and hash of the two dump files.

When it is written:

- after `get_or_build_embedding_index` builds an index from storage, and
- in `close()` (or `persist_embedding_indices()`, which returns the models it
  wrote) when the cached index has inserts or removals since its last dump.

Not on every insert: a process that dies without `close()` changes the
fingerprint, so the next open rebuilds anyway, and per-insert dumps would only
add cost. Every write goes to a temporary name and is renamed; the `.meta`
goes last, so an interrupted write leaves either the previous complete set or
a `.meta` that does not match its files.

When it is read: the first `get_or_build_embedding_index` after opening
fingerprints the embeddings in storage and loads the dump only if the
fingerprint matches. Otherwise it logs why and rebuilds, then rewrites the
dump:

| situation | outcome |
|---|---|
| no dump | build from storage |
| embeddings written without `close()` (crash, `drop` without close) | `Stale` → rebuild |
| dump files changed or truncated | `Corrupt` → rebuild, never handed to `hnsw_rs` (its loader panics on bad input) |
| fewer than 1024 live points (`EXACT_SEARCH_THRESHOLD`) | not persisted at all: the rebuild costs milliseconds and the exact path keeps its own vectors |
| in-memory graph, read-only handle | loads if a dump exists (read-only), never writes |

`embedding_index_stats(model)` reports `persisted` (the current in-memory
state is on disk, i.e. reopening would load it) and `loaded_from_disk_ms`
(`None` when the index was built from storage). A failed dump in `close()` is
a warning, not an error: the index is a derived cache and storage remains the
source of truth.

Layout note: the fingerprint hashes keys **and values** because
`Embedding::version` does not change when a vector is replaced. It is a full
scan of the model's embeddings at open (the same read the rebuild would do,
minus deserialising and indexing).

### NQL: `similar_to()` in WHERE

```sql
-- Find the 10 companies most similar to "Atlas Fiduciary Group"
find n.name from (n:Company)
where similar_to(n, "Atlas Fiduciary Group", "minilm")
limit 10

-- Combine with graph predicates
find n.name from (n:Company)
where similar_to(n, "Atlas Fiduciary Group", "minilm") and n.sector = "offshore"
limit 10
```

`similar_to(n, "reference_name", "model")` pre-computes the HNSW search before
streaming. The `LIMIT` clause controls how many neighbors to retrieve (default: 10).
Other WHERE predicates are applied as post-filters on the HNSW result set.

### Other NQL embedding functions

```sql
-- Filter: only nodes that have an embedding
find n.title from (n:Article) where has_embedding(n, "minilm")

-- Projection: cosine similarity score
find n.title, embedding_similarity(n, "uuid-of-ref-node", "minilm") as sim
from (n:Article)

-- Aggregation: k nearest neighbors as JSON array
find n.title, knn_nodes(n, 5, "minilm") as neighbors
from (n:Article)
```

### Parameters

| Parameter | Default | Purpose |
|-----------|---------|---------|
| M (max connections) | 24 | Edges per node per layer. Higher = better recall, more memory |
| ef_construction | 400 | Beam width during index construction |
| ef_search | 30 | Beam width during search. Higher = better recall, slower |

### Distance metric

The index uses **cosine distance** (1 - cosine_similarity). Range: 0 (identical) to 2 (opposite).
For best results, normalize vectors to unit length before storing.

See `docs/HNSW_ALGORITHM.md` for a deep dive into how HNSW works.

### Measuring it: the `hnsw_ops` bench

`make bench BENCH=hnsw_ops` measures the index on synthetic 384-dimensional
vectors (fixed seed; sizes via `NOPALDB_HNSW_N`, default `10000,100000`). The
target wraps `cargo bench` with `CARGO_PROFILE_RELEASE_PANIC=unwind`: the
workspace's release profile aborts on panic, and bench harnesses need unwind. Orders of magnitude on an Apple Silicon laptop,
0.5.18, k = 10:

| what | N = 10k | N = 100k |
|---|---|---|
| `build_batch` (index from scratch) | 2.35 s | 90.7 s |
| `rebuild_after_insert` (one new embedding + full rebuild: today's cost) | 2.38 s | 102.7 s |
| `insert_incremental` (one `HnswIndex::insert` on a live index) | 4.1 ms | 9.6 ms |
| `search_knn`, `ef_search` 30 / 60 | 0.55 ms / 1.1 ms | 0.95 ms / 1.7 ms |
| `search_knn_filtered`, filter passes 1 % / 10 % / 100 % of ids | 12.1 / 6.0 / 2.5 ms | 63.6 / 27.6 / 4.6 ms |
| `open_first_search` (open a persisted database + first search), rebuild from storage (`NOPALDB_HNSW_COLD=1`; the only path before 0.5.20) | 2.71 s | 107 s |
| `open_first_search`, loading the dump written by the previous `close()` (0.5.20) | 108 ms | 1.02 s |

What the numbers say:

- A new embedding currently costs a full rebuild on the next search
  (`add_node_embedding` invalidates the cached index): ~580× the cost of the
  incremental insert the index already supports at 10k, ~10 000× at 100k. That
  gap is what [#113](https://github.com/sharop/nopaldb/issues/113) closes.
- Opening a database and searching once used to pay the whole build, because
  the HNSW graph lived only in RAM: almost two minutes at 100k vectors. With
  the dump ([#114](https://github.com/sharop/nopaldb/issues/114)) the same
  open + first search takes about a second at 100k, which is mostly the
  fingerprint scan of the stored embeddings plus reading 240 MB of dump.
- `search_knn_filtered` is the index's native filtered traversal, which
  escalates `ef_search` while the filter starves it; that is why a 1 % filter
  is the most expensive row here. Graph-level searches do not take this path
  for very selective filters: the planner ranks the allowed set exactly when it
  is small (≤ 1024 candidates), see `HYBRID_SEARCH.md`.

---

## Current boundaries

- **Batch upsert** — `add_node_embedding` is one-at-a-time.
- **HNSW dumps are per process lifecycle** — the graph is written after a
  full build and in `close()`, not per insert; a crash before `close()` means
  a rebuild on the next open (correct, just slow at 100k). Indexes under 1024
  points are never persisted. No `mmap` reload yet: the whole graph is loaded
  into RAM.
- **Edge embedding HNSW** — `EdgeEmbedding` storage exists but the HNSW index only
  covers node embeddings currently.
- **Automatic invalidation** — updating a node's properties does not invalidate its
  embedding. Use the `version` field to track staleness in your application.
