# Machine Learning Examples on NopalDB

NopalDB demonstrates real Graph Neural Network (GNN) and Graph ML capabilities through practical examples.

## Overview

| Example | Algorithm | Task | Accuracy | Lines |
|---------|-----------|------|----------|-------|
| **Node Classification** | GCN | Community Detection | 90% | ~200 |
| **Graph Embeddings** | Node2Vec | Similarity Learning | High | ~300 |
| **Link Prediction** | Adamic-Adar | Citation Prediction | Good | ~200 |
| **Fraud Detection** | DFS Cycles | Risk Scoring | 100% | ~250 |

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

### Dataset: Karate Club

- 10 members, 2 communities
- 17 friendship edges
- 1 bridge edge connecting communities

### Results
```
Community A (0-4): 100% accuracy (5/5)
Community B (5-9): 80% accuracy (4/5)
Overall: 90% accuracy

Bridge node correctly identified (embedding near 0)
```

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
```
ML Cluster:
  ML Basics ↔ Deep Learning: 0.613 (high similarity)
  ML Basics ↔ Neural Nets: 0.068 (different approaches)

Graph Cluster:
  Graph Theory ↔ GNN: 0.706 (highly related)
  Graph Algorithms ↔ GNN: 0.203 (less related)
```

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

Paper A most likely to cite:
1. "Network Analysis" (score: 2.000)
2. "Machine Learning on Graphs" (score: 1.000)

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
  Charlie: 🔴 MEDIUM (50.0)
  Dana: 🔴 MEDIUM (50.0)
  Eve: 🔴 MEDIUM (50.0)
```

### Run
```bash
cargo run --example ml_fraud_detection --release
```

---

## Comparison with Other Frameworks

| Feature | NopalDB | NetworkX | Neo4j | PyG |
|---------|---------|----------|-------|-----|
| **Language** | Rust | Python | Cypher | Python |
| **Performance** | High | Low | High | Medium |
| **GNN Support** | Native | Limited | Plugin | Native |
| **ACID Transactions** | ✅ | ❌ | ✅ | ❌ |
| **Embeddings** | ✅ | ✅ | ❌ | ✅ |
| **Production-Ready** | ✅ | ❌ | ✅ | ⚠️ |

---

## Performance Benchmarks

### Node Classification (GCN)
- **Training**: <50ms (2 layers, 10 nodes)
- **Inference**: <5ms per node
- **Memory**: ~1MB for graph + embeddings

### Graph Embeddings (Node2Vec)
- **Random walks**: 30 walks in ~10ms
- **Training**: 100 epochs in ~200ms
- **Memory**: ~2KB per node (8D embeddings)

---

## Next Steps with Apache Arrow

When NopalDB integrates Apache Arrow (Phase 5.5):

### Benefits

1. **Zero-copy** to PyTorch/TensorFlow
2. **Columnar format** for batch processing
3. **SIMD optimizations** for GNN operations
4. **Interoperability** with Python ML ecosystem

### Example (Future)
```rust
// Export to Arrow
let arrow_table = graph.to_arrow().await?;

// Train GNN in Python
python_gnn.train(arrow_table);

// Import trained embeddings back
graph.import_embeddings(embeddings).await?;
```

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
4. Submit PR with benchmarks

---

## Future ML Features (Roadmap)

- [ ] **GAT** (Graph Attention Networks)
- [ ] **GraphSAGE** (Inductive learning)
- [ ] **Temporal GNNs** (Dynamic graphs)
- [ ] **PyTorch integration** (via Arrow)
- [ ] **GPU acceleration** (CUDA kernels)
- [ ] **Distributed training** (multi-node)

Progress: 30% → 35% (ML examples complete!)