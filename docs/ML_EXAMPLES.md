# Machine Learning Examples on NopalDB

These are small, self-contained Rust examples that implement Graph ML techniques
(GCN, Node2Vec-style embeddings, link prediction, fraud scoring) by hand on top of
the graph API. They are teaching examples on toy graphs (5–10 nodes), not a GNN
API of NopalDB and not benchmarks.

## Overview

| Example | Algorithm | Task | Graph |
|---------|-----------|------|-------|
| **Node Classification** | GCN | Community Detection | 10 nodes |
| **Graph Embeddings** | Node2Vec-style | Similarity Learning | 6 nodes |
| **Link Prediction** | Adamic-Adar | Citation Prediction | 5 nodes |
| **Fraud Detection** | DFS Cycles | Risk Scoring | 5 nodes |

---

## 1. Node Classification (GCN)

**File**: `examples/gnn_node_classification.rs`

### Algorithm: Graph Convolutional Network (GCN)

Learns node representations by aggregating features from neighbors.
```
h'[v] = σ(W · Σ(h[u] / sqrt(deg(v) * deg(u))))
        where u ∈ N(v)
```

### Implementation

- **2-layer GCN** with symmetric normalization
- **Message passing** over graph structure
- **Tanh activation** for non-linearity
- **Degree + bias features** for initialization

> **Label leakage.** The input feature of each node is `degree / 10 ± 0.5`,
> where the sign is the node's *true* community (`extract_features`,
> `+0.5` for members 0–4, `-0.5` for 5–9). The 90 % below therefore shows
> message passing smoothing a feature that already encodes the answer; it is
> not evidence that the GCN discovers communities from structure alone.

### Dataset: Karate Club

- 10 members, 2 communities
- 17 friendship edges
- 1 bridge edge connecting communities

### Results
```
Member 0-4: A → A  ✓ (5/5)
Member 5:   B → A  ✗ (embedding: 0.034, next to the bridge)
Member 6-9: B → B  ✓ (4/4)
Accuracy: 9/10 (90.0%)
```
(Deterministic: no random weights.)

### Run
```bash
cargo run --example gnn_node_classification --release
```

### Key Concepts

- **Message Passing**: Nodes aggregate neighbor features
- **Symmetric Normalization**: Prevents degree bias
- **Self-loops**: Preserve node's own features
- **Layer Stacking**: Captures multi-hop neighborhoods

---

## 2. Graph Embeddings (Node2Vec)

**File**: `examples/gnn_graph_embeddings.rs`

### Algorithm: Node2Vec with Negative Sampling

Learns low-dimensional representations by optimizing random walk co-occurrence.
```
maximize: log σ(emb(u) · emb(v))  for (u,v) in walks
minimize: log σ(-emb(u) · emb(w)) for random w
```

### Implementation

- **Random walks**: 5 walks per node, length 10
- **Skip-gram training**: Co-occurrence optimization
- **Negative sampling**: 3 negatives per positive
- **8-dimensional embeddings**

### Dataset: Citation Network

- 6 papers (ML cluster, Graph cluster)
- 8 citation edges
- 1 bridge paper (GNN)

### Results

The walks and the training are not seeded (`rand::rng()`), so the
similarities change on every run. One run (0.9.3):
```
Machine Learning Basics → Neural Networks 0.394, Deep Learning 0.284
Graph Theory            → Graph Neural Networks 0.819, Graph Algorithms 0.129
Graph Algorithms        → Graph Neural Networks -0.116
```
What tends to hold across runs is the cluster structure (ML papers closer to
each other, Graph Theory close to GNN), not the exact values.

### Run
```bash
cargo run --example gnn_graph_embeddings --release
```

### Key Concepts

- **Random Walks**: Capture graph structure
- **Skip-Gram**: Word2Vec adapted to graphs
- **Negative Sampling**: Prevents embedding collapse
- **Cosine Similarity**: Measures embedding distance

---

## 3. Link Prediction

**File**: `examples/ml_link_prediction.rs`

### Algorithm: Adamic-Adar Index

Predicts future links using common neighbor heuristic.
```
score(u,v) = Σ(1 / log(degree(w)))
             where w ∈ common_neighbors(u,v)
```

### Implementation

- **Common neighbors**: Find shared connections
- **Degree weighting**: Rare neighbors count more
- **Preferential attachment**: Fallback for no common neighbors

### Dataset: Citation Network

- 5 papers with citation relationships
- Predicts next citation for target paper

### Results

Paper A most likely to cite (0.9.3 output, a single candidate):
1. "Network Analysis" (score: 2.000)

### Run
```bash
cargo run --example ml_link_prediction --release
```

---

## 4. Fraud Detection

**File**: `examples/ml_fraud_detection.rs`

### Algorithm: Multi-Feature Risk Scoring

Combines multiple graph patterns to detect fraud.
```
risk_score = circular_transfers * 50
           + high_velocity * 20
           + structuring * 30
```

### Features

1. **Circular Transfers**: DFS cycle detection (3+ nodes)
2. **Transaction Velocity**: High activity rate
3. **Structuring**: Multiple similar amounts (smurfing)

### Dataset: Transaction Network

- 5 accounts
- Normal transactions (Alice ↔ Bob)
- Suspicious circular pattern (Charlie → Dana → Eve → Charlie)

### Results
```
Normal Accounts:
  Alice: 🟢 LOW (0.0)
  Bob: 🟢 LOW (0.0)

Suspicious Accounts (circular transfers):
  Charlie: 🟡 MEDIUM (50.0)
  Dana: 🟡 MEDIUM (50.0)
  Eve: 🟡 MEDIUM (50.0)
```

### Run
```bash
cargo run --example ml_fraud_detection --release
```

---

## What NopalDB provides, and what it does not

- **Provided:** the graph (ACID transactions), stored embeddings with an HNSW
  index, graph algorithms in NQL (see [ALGORITHMS.md](ALGORITHMS.md)) and
  Arrow export. The examples above use only that public API.
- **Not provided:** a GNN layer, a training loop or an autograd engine. The
  GCN and Node2Vec code lives in the example files; for real models export
  the graph and train with a dedicated library (PyG, DGL, …).
- No performance figures are given for these examples: on 5–10 nodes they
  measure nothing useful.

## Moving data to a Python ML stack

Arrow export exists today (it is not zero-copy across the Rust→Python
boundary: Python receives Arrow IPC bytes). See:

- [docs/arrow](arrow/01-OVERVIEW.md) — Rust API (`to_arrow`, `to_arrow_complete`, Parquet)
- [docs/python/ARROW_EXPORT.md](python/ARROW_EXPORT.md) — Python (`graph.to_arrow_complete()` → `pyarrow`)

Embeddings computed outside go back with `graph.add_node_embeddings(model, items)`.

---

## Theory & Resources

### Papers

1. **GCN**: Kipf & Welling (2017)
    - "Semi-Supervised Classification with Graph Convolutional Networks"
    - https://arxiv.org/abs/1609.02907

2. **Node2Vec**: Grover & Leskovec (2016)
    - "node2vec: Scalable Feature Learning for Networks"
    - https://arxiv.org/abs/1607.00653

3. **Word2Vec**: Mikolov et al. (2013)
    - "Efficient Estimation of Word Representations in Vector Space"
    - https://arxiv.org/abs/1301.3781

### Books

1. **Graph Representation Learning** - William L. Hamilton
    - FREE: https://www.cs.mcgill.ca/~wlh/grl_book/

2. **Deep Learning on Graphs** - Ma & Tang

### Courses

1. **Stanford CS224W** - Machine Learning with Graphs
    - http://web.stanford.edu/class/cs224w/

---

## Contributing

To add new ML examples:

1. Create file in `examples/gnn_*.rs`
2. Use existing graph API (no external ML libs needed)
3. Add to this documentation
4. Submit PR

---

No further ML features are scheduled.