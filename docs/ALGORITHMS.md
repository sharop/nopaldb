# Graph Algorithms in NopalDB

NopalDB includes 7 built-in graph algorithms, all integrated into the NQL query language.

---

## 🎯 Available Algorithms

| Algorithm | Complexity | Use Case | NQL Function |
|-----------|-----------|----------|--------------|
| **PageRank** | O(E·k) | Importance ranking | `pagerank(n)` |
| **Betweenness** | O(V·E) | Bridge detection | `betweenness(n)` |
| **Clustering** | O(V·d²) | Community cohesion | `clustering(n)` |
| **Degree** | O(E) | Connectivity | `degree(n)` |
| **Shortest Path** | O(E·log V) | Path finding | Rust API |
| **Community (Louvain)** | O(E·log² V) | Community detection | `community(n)` |
| **Community (Leiden)** | O(V·E·iter) | Well-connected communities | `leiden(n)` |

*k = iterations, d = avg degree, V = nodes, E = edges, iter = outer loop count*

---

## 📊 1. PageRank

**Purpose**: Measure global importance/influence of nodes

### Example
```sql
-- Find most influential people
find n.name, pagerank(n) as rank
from (n:Person)
order by rank desc
limit 10
```

### Parameters
- **Damping**: 0.85 (default)
- **Iterations**: 100 (default)
- **Tolerance**: 1e-6 (default)

### Output
- Score between 0 and 1
- Higher = more influential

### Best For
- Social network analysis
- Web page ranking
- Citation networks
- Fraud detection (find key players)

---

## 🌉 2. Betweenness Centrality

**Purpose**: Identify bridges and bottlenecks in networks

### Example
```sql
-- Find critical connectors
find n.name, betweenness(n) as bc
from (n:Person)
where bc > 0.5
order by bc desc
```

### Algorithm
- Brandes' algorithm
- Counts shortest paths through each node
- Normalized by (n-1)(n-2)/2

### Output
- Score between 0 and 1
- High score = important bridge

### Best For
- Find key intermediaries
- Identify choke points
- Detect money launderers
- Network vulnerability analysis

---

## 🔺 3. Clustering Coefficient

**Purpose**: Measure local cohesion and community density

### Example
```sql
-- Find tightly-knit groups
find n.community, clustering(n) as cohesion
from (n:Person)
group by n.community
```

### Formula
```
C(v) = 2·T(v) / (k(v)·(k(v)-1))
```
- T(v) = triangles containing v
- k(v) = degree of v

### Output
- Score between 0 and 1
- 1.0 = all neighbors connected (clique)
- 0.0 = no neighbors connected

### Best For
- Detect fraud rings
- Find close-knit communities
- Measure network cohesion
- Identify echo chambers

---

## 📈 4. Degree Centrality

**Purpose**: Simple connectivity measure

### Example
```sql
-- Find most connected nodes
find n.name, degree(n) as connections
from (n:Person)
order by connections desc
limit 10
```

### Variants
- **Total Degree**: In + Out (default)
- **In-Degree**: Incoming edges only
- **Out-Degree**: Outgoing edges only

### Output
- Integer count of connections
- Can be normalized to [0,1]

### Best For
- Quick connectivity analysis
- Hub detection
- Baseline metric
- Network statistics

---

## 🛤️ 5. Shortest Path

**Purpose**: Find optimal paths between nodes

### Example (Rust API)
```rust
use nopaldb::algorithms::shortest_path::{ShortestPath, ShortestPathConfig};

// Unweighted (BFS)
let sp = ShortestPath::default();
let result = sp.find_path(&graph, source, target).await?;

// Weighted (Dijkstra)
let config = ShortestPathConfig {
    weight_property: Some("weight".to_string()),
    max_length: None,
};
let sp = ShortestPath::new(config);
let result = sp.find_path(&graph, source, target).await?;

if let Some(path) = result {
    println!("Path: {:?}", path.path);
    println!("Distance: {}", path.distance);
}
```

### Algorithms
- **BFS**: Unweighted graphs, O(V+E)
- **Dijkstra**: Weighted graphs, O(E·log V)

### Output
```rust
PathResult {
    path: Vec<NodeId>,    // Node sequence
    distance: f64,        // Total cost
}
```

### Best For
- Route planning
- Dependency analysis
- Transaction tracing
- Relationship paths

---

## 🏘️ 6. Community Detection (Louvain)

**Purpose**: Detect densely connected groups using modularity optimization.

### Example (Rust API)
```rust
use nopaldb::algorithms::community::{LouvainCommunity, CommunityConfig};

let config = CommunityConfig {
    resolution: 1.0,      // Higher = more communities
    max_iterations: 100,
    min_gain: 0.0001,
};

let louvain = LouvainCommunity::new(config);
let communities = louvain.detect(&graph).await?;

// communities: HashMap<NodeId, usize>
// Maps each node to its community ID (0, 1, 2, ...)
```

### Example (NQL)
```sql
find n.name, community(n) as cluster
from (n)
order by cluster
limit 25
```

### Algorithm
- Louvain method
- Modularity optimization
- Known limitation: can produce internally disconnected communities

### Output
- Community ID per node (0-indexed integers)
- Result cached per topology version

### Best For
- Fraud ring detection
- Market segmentation
- Social group discovery
- Network partitioning

---

## 🏘️ 7. Community Detection (Leiden)

**Purpose**: Detect well-connected communities using the Constant Potts Model (CPM).
Leiden is a direct improvement over Louvain that **guarantees internally connected communities**.

> Traag, V.A., Waltman, L. & van Eck, N.J. (2019).
> *From Louvain to Leiden: guaranteeing well-connected communities.*
> Scientific Reports, 9, 5233. https://doi.org/10.1038/s41598-019-41695-z

### Key Differences vs Louvain

| Property | Louvain | Leiden |
|----------|---------|--------|
| Quality function | Modularity | CPM (Constant Potts Model) |
| Well-connected guarantee | ❌ No | ✅ Yes |
| Resolution limit | Present | Absent (CPM avoids it) |
| Deterministic | ❌ (HashSet iteration) | ✅ (sorted iteration) |
| Separate cache | — | ✅ Independent of community() |
| NQL function | `community(n)` | `leiden(n)` |

### CPM Quality Function

```
H(P) = Σ_C [ e_C − γ · n_C · (n_C − 1) / 2 ]
```

- `e_C` = sum of edge weights inside community C
- `n_C` = number of nodes in C
- `γ` (gamma) = resolution parameter (higher → more, smaller communities)

**Interpretation of γ**: two nodes end up in the same community if and only if the edge density between them exceeds γ. Unlike modularity, γ has a direct density interpretation and is comparable across graphs.

### Phase 1 — Local Moving (CPM gain)

For each node v moving from community s to t:
```
ΔH(v: s→t) = e(v, C_t) − e(v, C_s\{v}) + γ · (|C_s| − 1 − |C_t|)
```

### Phase 2 — Refinement (well-connectedness guarantee)

Within each community C from Phase 1, restarts from singletons and applies restricted merges:
- Node v is **eligible** only if: `e(v, C\{v}) ≥ γ · (|C| − 1)`
- Subset R is **well-connected** in C if: `e(R, C\R) ≥ γ · |R| · (|C| − |R|)`
- Node v can join R only if it is still a singleton AND R is well-connected

This refinement step is the key contribution of the paper — it prevents the disconnected-community pathology of Louvain.

### Example (Rust API)

```rust
use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig};

// Default gamma = 0.1
let leiden = LeidenCommunity::with_defaults();

// Custom gamma
let leiden = LeidenCommunity::with_gamma(0.05);

// Full config
let leiden = LeidenCommunity::new(LeidenConfig {
    gamma: 0.1,           // CPM resolution — higher = more communities
    max_iterations: 10,   // outer loop limit
    min_gain: 1e-9,       // minimum CPM gain to accept a move
});

let communities = leiden.detect(&graph).await?;
// HashMap<NodeId, usize> — 0-indexed community IDs
```

### Example (NQL)

```sql
-- Basic
find n.name, leiden(n) as cluster
from (n)
order by cluster
limit 25

-- With gamma guidance via EXPLAIN
explain find leiden(n) from (n)

-- Compare Louvain vs Leiden (independent caches, single query)
find n.name,
     community(n) as louvain,
     leiden(n)    as leiden_comm
from (n)
order by leiden_comm
limit 20
```

### Gamma Selection Guide

| Graph type | Recommended γ |
|------------|--------------|
| Social / fraud networks | 0.05 – 0.15 |
| Financial transactions | 0.1 – 0.3 |
| Knowledge graphs | 0.05 – 0.1 |
| GNN clustering (dense) | 0.3 – 0.5 |
| Highly fragmented desired | 0.5 – 1.0 |

### Output
- Community ID per node (0-indexed, contiguous integers after renumbering)
- Separate topology-versioned cache from `community()` / Louvain
- Deterministic: same topology always produces the same partition

### Best For
- GNN pipelines where disconnected communities contaminate neighborhood embeddings
- Fraud ring detection requiring structural guarantees
- Any use case where Louvain community quality is suspect
- Research / comparison against Louvain baseline

---

## 🔄 Combining Algorithms

You can combine multiple algorithms in a single query:

```sql
find n.name,
      degree(n) as connections,
      pagerank(n) as importance,
      betweenness(n) as bridge_score,
      clustering(n) as cohesion
from (n:Person)
where connections > 10
order by importance desc
limit 20
```

---

## 🎓 Algorithm Selection Guide

| Your Goal | Best Algorithm | Why |
|-----------|---------------|-----|
| Find influencers | PageRank | Global importance |
| Detect bottlenecks | Betweenness | Path dependency |
| Find tight groups | Clustering | Local density |
| Quick connectivity | Degree | Fast & simple |
| Optimal routing | Shortest Path | Minimal cost |
| Fast community exploration | community_fast | Low-latency approximate |
| Community detection (standard) | community / Louvain | Modularity optimization |
| Community detection (quality) | leiden / Leiden | CPM + well-connected guarantee |
| GNN neighborhood features | leiden | Disconnected communities break embeddings |

---

## ⚡ Performance Tips

### 1. Use GROUP BY for Aggregations
```sql
-- ✅ GOOD: Aggregate by group
find n.city, pagerank(n) as avg_rank
from (n:Person)
group by n.city

-- ❌ BAD: Compute for each node individually
-- (NQL doesn't support per-node projections yet)
```

### 2. Filter Before Computing
```sql
-- ✅ GOOD: Filter first
find betweenness(n) as bc
from (n:Person)
where n.age > 18

-- ❌ BAD: Compute then filter in Python
```

### 3. Use Appropriate Algorithm
```sql
-- ✅ GOOD: Degree for simple connectivity
find degree(n) as connections
from (n:Person)

-- ❌ BAD: Betweenness for simple connectivity
-- (10-100x slower than degree)
```

### 4. Batch Operations
```python
# ✅ GOOD: One query, multiple algorithms
result = graph.execute_nql("""
    find n.type,
          degree(n) as deg,
          pagerank(n) as pr,
          clustering(n) as cc
    from (n)
    group by n.type
""")

# ❌ BAD: Multiple queries
degrees = graph.execute_nql("find degree(n) from (n)")
ranks = graph.execute_nql("find pagerank(n) from (n)")
```

---

## 🧪 Testing

All algorithms include comprehensive test suites:

```bash
# Test individual algorithms
cargo test --lib pagerank
cargo test --lib betweenness
cargo test --lib clustering
cargo test --lib degree
cargo test --lib shortest_path
cargo test --lib community
cargo test leiden   # Leiden-specific tests (7 cases)

# Test all algorithms
cargo test --lib algorithms

# Python integration tests
python examples/test_all_algorithms.py
```

---

## 📚 References

### References
- **PageRank**: Page, L., et al. "The PageRank Citation Ranking" (1999)
- **Betweenness**: Brandes, U. "A Faster Algorithm for Betweenness Centrality" (2001)
- **Clustering**: Watts, D.J., Strogatz, S.H. "Collective dynamics of 'small-world' networks" (1998)
- **Louvain**: Blondel, V.D., et al. "Fast unfolding of communities in large networks" (2008)
- **Leiden**: Traag, V.A., Waltman, L. & van Eck, N.J. "From Louvain to Leiden: guaranteeing well-connected communities" (2019). Scientific Reports, 9, 5233. https://doi.org/10.1038/s41598-019-41695-z

### Implementation Notes
- All algorithms are **async** for non-blocking execution
- **Caching** is used where appropriate
- **Memory-efficient** adjacency list construction
- **Configurable** parameters for all algorithms

---

## 💡 Contributing

Want to add a new algorithm? Check out:
1. `src/algorithms/pagerank.rs` - Simplest example
2. `src/algorithms/betweenness.rs` - Advanced example
3. Follow the pattern: Config struct → Algorithm struct → `compute()` method
4. Add tests in the same file
5. Integrate into NQL via `aggregations.rs`

---

**Questions?** Open an issue on GitHub!  
**Examples?** Check `examples/test_*.py` files!
