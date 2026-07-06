# Changelog

All notable changes to NopalDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.4.29] - 2026-07-05

### ✨ Highlights
- **Python isolation levels**: `graph.begin_transaction(isolation="serializable" | "repeatable_read" | "read_committed" | "read_uncommitted")` (requires the `full-isolation` feature).
- **Shared runtime + GIL release**: the Python bindings use one process-wide Tokio runtime and release the GIL during every DB call — measured ~3× read throughput with 4 threads (was serialized).
- **Isolation ↔ GC integration**: version GC runs under the single-writer gate (closing the GC-vs-version-list race), and direct writes are visible to Serializable conflict validation.

### Fixed
- **Lock leak**: failed commits (conflict/deadlock) and rollbacks did not release their locks, blocking subsequent writers on those nodes until the lock timeout; the commit path now cleans up on every error.
- Lock timeouts surface as `ConcurrencyError` instead of an opaque `Custom` error.
- WAL replay of committed edge inserts no longer crashes `Graph::open` when a later non-transactional delete removed the endpoints.

### Added
- Mixed-load SIGKILL crash harness and proptest model-based suite (random op sequences must match a reference model, including after reopen).
- Nightly hardening workflow (100 kill rounds + 200 proptest cases).
- `docs/DURABILITY.md`: the crash-safety contract, including the weaker durability of direct (non-transactional) writes.

### Security
- `arrow`/`parquet` 57 → 59: drops the transitive `thrift` dependency (RUSTSEC memory-allocation advisory).
- `tantivy` 0.25 → 0.26 and `ratatui`/`crossterm` 0.26/0.27 → 0.30/0.29: pull `lru` ≥ 0.16.4 (RUSTSEC `IterMut` soundness advisory); no more `lru` 0.12.x in the tree.

---

## [0.4.28] - 2026-07-04

Consolidated entry for the 0.4.x series.

### ✨ Highlights
- **Isolation Levels** (new opt-in `full-isolation` feature, included in the `full` tier):
  `ReadUncommitted` / `ReadCommitted` (default) / `RepeatableRead` / `Serializable`,
  per-node lock manager with wait-for-graph **deadlock detection**, and MVCC snapshot reads.
  See [docs/ISOLATION_LEVELS.md](docs/ISOLATION_LEVELS.md) and [docs/DEADLOCK_DETECTION.md](docs/DEADLOCK_DETECTION.md).
- **Storage concurrency**: removed the global storage lock; reads no longer block behind writers.
- **NQL**: path queries (quantifiers, metadata, reducers), embedding functions (`similar_to`, KNN,
  path similarity/anomaly), structured `PROFILE`, and write CRUD improvements.
- **Full-text search**: Tantivy-backed index behind the optional `fulltext` feature (in `core`).
- **MCP server**: agentic context tools (project-structure indexing, episodic events,
  PR-context validation), Arrow export over shared memory, input validation and NQL escaping.
- **NDBStudio**: query workbench with graph-hint fallback, session browser, timeline, and a web UI refresh.
- **Python**: abi3 wheels (one wheel for CPython 3.10+), PyPI release workflow, PyO3 0.29.

### Added
- `full-isolation` feature: `IsolationLevel`, `Transaction::with_isolation`, `LockManager`, deadlock detection.
- Synthetic benchmark examples (`examples/benchmarks.rs`, `examples/benchmark_community_dual.rs`) and benchmark report in `docs/benchmarks/`.
- New docs: property indexing internals, executor architecture (ES), Arrow performance notes, REPL workbench ADR/roadmap.

### Changed
- Version alignment across workspace, crate and Python package (0.4.28).
- Dependency updates including security bumps (`lz4_flex`, `time`, `tokio`, `rand`, `bytes`).

### Removed
- Orphan/dead modules (`nopaldb/src/isolation.rs` legacy stub, unused NDBStudio scaffolding).

---

## [0.3.0] - 2026-02-12

### ✨ Highlights
- **Zero Clippy Warnings**: Strict code quality enforcement across the entire codebase.
- **Graph Algorithms Stabilization**: API improvements and test coverage for all 6 algorithms.
- **Improved Type Correctness**: Refactoring of internals to use safer patterns (let-chains, type aliases).

### Changed
- **Algorithm APIs**: Standardized instantiation with `with_defaults()` instead of `default()` for better explicit configuration.
- **Code Quality**: Resolved over 40+ clippy warnings (collapsible ifs, needless borrows, etc.).
- **Cleanup**: Removed unused dependencies and artifacts.

## [0.2.0] - 2026-02-01

### 🎉 Major Release: Graph Algorithms & Advanced Queries

This release introduces **6 graph algorithms**, **schema inspection**, and **aggregation functions** integrated directly into NQL.

### Added

#### Schema Inspection API
- **New Module**: `src/schema/mod.rs` - Schema metadata management
- **Python API**: 8 new methods for schema introspection
  - `get_labels()` - Get all node labels
  - `get_edge_types()` - Get all edge types
  - `get_label_properties(label)` - Get properties for a label
  - `get_label_count(label)` - Count nodes by label
  - `get_edge_type_properties(type)` - Get edge properties
  - `get_edge_type_count(type)` - Count edges by type
  - `get_schema()` - Get complete schema metadata
  - `rebuild_schema()` - Force schema cache rebuild
- **Caching**: Efficient schema caching with dirty flag tracking

#### NQL Aggregation Functions
- **Statistical Aggregations**:
  - `count(n)` - Count nodes/edges
  - `sum(n.property)` - Sum numeric properties
  - `avg(n.property)` - Average of numeric properties
  - `min(n.property)` - Minimum value
  - `max(n.property)` - Maximum value
- **GROUP BY Support**: Full grouping with aggregations
- **Async Execution**: All aggregations run asynchronously

#### Graph Algorithms (6 Total)

1. **PageRank** (`src/algorithms/pagerank.rs`)
   - Classic PageRank algorithm
   - Personalized PageRank support
   - Configurable damping factor and iterations
   - NQL integration: `pagerank(n)`
   - Convergence detection

2. **Betweenness Centrality** (`src/algorithms/betweenness.rs`)
   - Brandes' algorithm implementation
   - O(VE) complexity for unweighted graphs
   - Normalized and unnormalized variants
   - NQL integration: `betweenness(n)`

3. **Clustering Coefficient** (`src/algorithms/clustering.rs`)
   - Local clustering coefficient
   - Global clustering (transitivity)
   - Triangle counting
   - NQL integration: `clustering(n)`

4. **Degree Centrality** (`src/algorithms/degree.rs`)
   - In-degree, out-degree, total degree
   - Normalized variants
   - Degree statistics (min, max, mean, median)
   - NQL integration: `degree(n)`

5. **Shortest Path** (`src/algorithms/shortest_path.rs`)
   - Dijkstra's algorithm (weighted)
   - BFS (unweighted)
   - Single-source shortest paths
   - Average path length calculation
   - Rust API only (NQL integration planned)

6. **Community Detection** (`src/algorithms/community.rs`)
   - Louvain method
   - Modularity optimization
   - Configurable resolution
   - Rust API only (NQL integration planned)

#### Examples
- `examples/schema_inspection.py` - Schema API demonstration
- `examples/synthetic_offshore_schema.py` - Synthetic offshore network analysis
- `examples/test_pagerank.py` - PageRank examples
- `examples/test_betweenness.py` - Betweenness examples
- `examples/test_clustering.py` - Clustering examples
- `examples/test_degree.py` - Degree centrality examples
- `examples/test_all_algorithms.py` - Complete algorithm suite test

#### Documentation
- `docs/python/SCHEMA_INSPECTION.md` - Complete API reference
- Algorithm documentation with examples
- Performance guidelines
- Best practices guide

### Changed

#### NQL Executor
- **Async Transformation**: `execute()` and `project_result()` now async
- **Aggregation Support**: New execution path for aggregations
- **Graph Access**: Aggregations now have access to Graph for algorithms

#### Type System
- **PropertyValue**: Used consistently across aggregations
- **Row Construction**: Improved with helper methods

### Fixed
- GROUP BY now correctly handles `n.label` as node field (not property)
- PropertyValue conversions in aggregations (Int vs Integer)
- Async compilation issues in executor chain
- Memory leaks in schema caching

### Performance
- **Schema Caching**: O(1) for cached schema lookups
- **Batch Operations**: BulkLoader API for efficient imports
- **Algorithm Optimization**: Efficient adjacency list construction
- **Memory**: Reduced allocations in hot paths

### Testing
- 50+ new unit tests
- Integration tests for all algorithms
- Schema API test coverage
- Aggregation test suite
- End-to-end Python tests

### Technical Debt Resolved
- Removed legacy aggregation detection code
- Cleaned up unused imports
- Fixed all compiler warnings
- Improved error messages

---

## [0.1.5] - 2026-01-15

### Added
- MVCC transaction support
- WAL (Write-Ahead Logging)
- Python bindings with PyO3
- Apache Arrow integration
- NQL v0.2 parser and executor
- BulkLoader API for efficient imports
- Synthetic offshore network demo and analysis

### Changed
- Migrated from single-threaded to async/await
- Improved transaction isolation
- Enhanced error handling

### Fixed
- Concurrency bugs in transaction manager
- Memory leaks in WAL
- Edge property serialization

---

## [0.1.0] - 2025-12-01

### Added
- Initial release
- Basic graph operations (nodes, edges)
- Simple query interface
- File-based storage with sled
- Python bindings (basic)

---

## Upcoming in v0.3.0 (Q2 2026)

### Planned Features
- Docker + Jupyter environment
- Performance benchmarks vs Neo4j
- Query optimization
- Sharding support
- PyPI publication
- crates.io publication

### Under Consideration
- GraphQL API
- REST API
- WebAssembly build
- Real-time subscriptions
- Visual query builder

---

## Migration Guides

### Migrating from v0.1.5 to v0.2.0

#### Schema API (New)
```python
# Before: No schema introspection
# Had to query to discover structure

# After: Direct schema access
labels = graph.get_labels()
schema = graph.get_schema()
properties = graph.get_label_properties("Person")
```

#### Aggregations (New)
```python
# Before: Manual aggregation in Python
result = graph.execute_nql("find n from (n:Person)")
count = len(list(result))

# After: NQL aggregations
result = graph.execute_nql("find count(n) from (n:Person)")
count = list(result)[0].get('count')
```

#### Graph Algorithms (New)
```python
# Before: External libraries (NetworkX)
# After: Built-in NQL functions
result = graph.execute_nql("""
    find n.name, pagerank(n) as rank
    from (n:Person)
    order by rank desc
    limit 10
""")
```

---

## Breaking Changes

### v0.2.0
- None! Fully backward compatible with v0.1.5

### Future Breaking Changes (v1.0.0)
- May remove deprecated APIs
- NQL syntax standardization
- Python API cleanup

---

**Version Scheme**: MAJOR.MINOR.PATCH
- **MAJOR**: Breaking changes
- **MINOR**: New features (backward compatible)
- **PATCH**: Bug fixes

---

[0.2.0]: https://github.com/sharop/nopaldb/releases/tag/v0.2.0
[0.1.5]: https://github.com/sharop/nopaldb/releases/tag/v0.1.5
[0.1.0]: https://github.com/sharop/nopaldb/releases/tag/v0.1.0
