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
| Referential integrity | `add_node_embedding` validates the node exists |

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
```

### Incremental inserts

```rust
let mut index = HnswIndex::new("minilm", 384, 100_000);
index.insert(node_id, vector)?;  // No rebuild needed
```

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

---

## Current boundaries

- **Batch upsert** — `add_node_embedding` is one-at-a-time.
- **Persistent HNSW graph** — the HNSW index lives in RAM and rebuilds from Sled on
  startup.
- **Edge embedding HNSW** — `EdgeEmbedding` storage exists but the HNSW index only
  covers node embeddings currently.
- **Automatic invalidation** — updating a node's properties does not invalidate its
  embedding. Use the `version` field to track staleness in your application.
