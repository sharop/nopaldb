# Changelog

All notable changes to NopalDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.4.36] - 2026-08-01

### ✨ Highlights

- **Índice de propiedades tipo-correcto (formato v2) con migración
  automática.** El formato anterior stringificaba el valor en la clave
  (`idx:prop:{name}:{value}`), así que `Int(1)`, `Float(1.0)` y
  `String("1")` compartían entrada — buscar cualquiera regresaba los nodos
  de los otros. El formato v2 usa claves tipadas (length-prefix del nombre +
  tag de tipo + valor canónico order-preserving) en un árbol sled propio
  (`prop_idx_v2`). Al abrir una base existente, la migración corre sola:
  borra el índice legado, reconstruye desde los nodos (fuente de verdad,
  chunks de memoria acotada) y escribe el sentinel `meta:prop_idx_format`.
  Idempotente y crash-safe: un crash a media migración simplemente la
  repite en el próximo open; los nodos y aristas jamás se tocan.

- **EXPLAIN honesto.** El plan que reporta `EXPLAIN` ahora sale de la misma
  decisión estática que usa la ejecución (`index_fast_path_decision`), no de
  un camino paralelo: reporta `INDEX SEEK` solo cuando el seek realmente
  ocurrirá, `LABEL SCAN` con la razón concreta (operador no-igualdad, RHS no
  literal, sin índice…), `FULL SCAN` para queries sin label (antes: error) y
  un mensaje claro para writes (antes: Debug dump del AST).

### Added

- `Graph::rebuild_property_index()` — reconstruye el índice de propiedades
  completo desde los nodos. Sirve para reparar un índice desactualizado
  (p. ej. tras escrituras index-blind con `insert_node`) y es la base de la
  migración v1→v2 y de un futuro `REINDEX`.
- `nopaldb/tests/prop_index_v2_test.rs` (contrato del índice tipado) y
  `nql_explain_honesty_test.rs` (matriz plan-reportado == ejecución-real),
  más las suites de migración/crash-safety como unit tests de storage.

### Changed

- **Nota de downgrade**: una base migrada a v2 y abierta con un binario
  ≤0.4.35 no se corrompe (nodos/aristas intactos), pero el índice legado ya
  no existe: los lookups por propiedad dan falsos negativos. Para volver
  atrás hay que reabrir con ≥0.4.36 o reconstruir el índice manualmente.
- `get_node_by_property` (que recibe `&str`) ahora busca `String` estricto:
  ya no «encuentra» nodos `Int`/`Float` vía la colisión del formato viejo.
  Es la restauración del contrato documentado; va aquí por si algún
  consumidor dependía de la colisión.
- `BTreeIndex::range_query` usa `BTreeMap::range` (O(log N + k)) en lugar de
  iterar todo el árbol con filter O(N). Misma semántica; `Between` con
  extremos invertidos regresa vacío (documentado — antes el filter también
  regresaba vacío, pero `range()` habría panicado).

### Fixed

- Colisiones de tipo del índice de propiedades (`Int(1)` == `Float(1.0)` ==
  `String("1")`, `Bool(true)` == `String("true")`, `Null` == `String("null")`),
  inyección de separador (prop `a` + valor `b:c` colisionaba con prop `a:b` +
  valor `c`) y floats no canónicos (`-0.0` y `0.0` eran claves distintas;
  todo NaN colapsa ahora a una clave canónica única).
- Python: `Transaction.add_node`/`add_edge` descartaban el error del motor
  (`let _ = …`) y devolvían el id de un nodo fantasma; ahora propagan la
  excepción y el id devuelto es el confirmado por el motor.
- Storage: los filtros del namespace `node:` usaban blacklists por substring
  (`contains(":v")`…) que funcionaban por accidente; ahora son predicados
  estructurales (prefijo + UUID parseado + sufijo exacto). El caso inverso
  (`get_all_versioned_nodes` aceptando sufijos futuros y reventando el
  export con `SerializationError`) quedó cerrado por la misma vía.

### Documented

- Hazard preexistente de `BTreeIndex`: el `Ord` de `PropertyValue` coerciona
  Int↔Float pero su `Eq` derivado no — claves numéricas heterogéneas se
  fusionan en el bucket del primero insertado. Pinneado en test; el fix real
  (normalización canónica) queda en el roadmap.
- `docs/PROPERTY_INDEXING.md` describe el formato v2 y la migración.

---

## [0.4.35] - 2026-07-27

### ✨ Highlights

- **API de conversión unificada en `PropertyValue`.** El tipo-contrato del
  motor ahora tiene su conversión canónica en el propio enum, y
  `with_property` acepta literales directamente:

  ```rust
  let n = Node::new("Person")
      .with_property("name", "Alice")   // &str → String
      .with_property("age", 30)         // i32  → Int
      .with_property("active", true);   // bool → Bool
  ```

- **Fixed (Python): `True` ya no se convierte en `1`.** Los conversores
  inline de `Transaction.add_node`/`add_edge` probaban `int` antes que
  `bool` (subtipo en Python). La frontera Python↔Rust usa ahora un conversor
  único, que además acepta `list`/`tuple`/`dict` anidados como propiedades.

### Added

- `From<bool/i64/i32/u32/f64/f32/&str/String/Vec<u8>/Vec<PropertyValue>/
  Vec<(String, PropertyValue)>/Option<T>>` para `PropertyValue`; exclusiones
  (`u64`/`usize`, `Vec<T>` genérico) razonadas en el rustdoc del enum.
- `PropertyValue::to_display_string()` + `impl Display` — render humano
  recursivo. Documentado: NO estable; jamás para claves de índice ni
  formatos persistidos.
- Puente `serde_json::Value` ↔ `PropertyValue` en ambas direcciones con
  matches exhaustivos (una variante futura rompe compilación, no serializa
  mal en silencio). Lossiness documentada: `Bytes` roundtripea como
  `List(Int)`; `NaN`→`null`.
- Doc del enum con el bloque «Restricciones de evolución»: agregar una
  variante es semver-major (sin `#[non_exhaustive]`), y el orden de
  declaración es load-bearing para `serde(untagged)` — pinneado en tests.
- Propiedades `list`/`tuple`/`dict` en los bindings Python (antes solo
  escalares y bytes), con guardia de roundtrip tipo-exacto en CI.

### Changed

- `Node::with_property`/`Edge::with_property` se generalizan a
  `impl Into<PropertyValue>`. Compatible con todo código que pasa un
  `PropertyValue` explícito; si un consumidor pasaba `x.into()` desnudo,
  la inferencia ahora es ambigua — usar `x` directamente o
  `PropertyValue::from(x)`.
- ndbstudio muestra floats no finitos como `null` de forma consistente en
  todas las vistas (una tenía drift a `NaN`/`inf`).
- Tutoriales: los helpers `prop_to_*` copiados 4 veces se sustituyen por el
  API del enum (`to_display_string`, `as_number`).

### Fixed

- Python: `bool` → `Int(1)` al escribir vía transacción (dos sitios).
- MCP: `Bytes` se serializaba como `null` por un wildcard `_ =>`; ahora sale
  como arreglo de números y el wildcard desapareció (matches exhaustivos en
  el core).

---

## [0.4.34] - 2026-07-27

### ✨ Highlights

- **`SackItem` ahora enlaza padre-hijo**: el resultado del sack traversal ya
  no es una lista plana irreconstruible. Cada ítem trae `parent`
  (índice del ítem padre en `items` — unívoco incluso cuando el mismo nodo
  aparece varias veces por caminos distintos) y `via_edge` (la arista por la
  que se llegó, que desambigua aristas paralelas y da acceso a sus
  propiedades). Con eso, un árbol anidado — p. ej. el BOM para una UI — se
  reconstruye en un solo paso O(n); el snippet está en el doc del módulo y
  en el ejemplo `bom_costing`, que ahora imprime el árbol indentado.

### Added

- `SackItem::parent: Option<usize>` — índice del ancestro emitido más
  cercano (`None` = hijo directo de un nodo de inicio). Con bloques de un
  salto, `items[parent].depth == depth - 1`; en `emit_leaves()` es siempre
  `None` (reporte plano, documentado).
- `SackItem::via_edge: Option<EdgeId>` — id de la arista seguida para
  llegar al nodo.

### Changed

- Nada rompe: `SackItem` es `#[non_exhaustive]` desde 0.4.33, así que los
  campos nuevos son aditivos para todo el código existente.

---

## [0.4.33] - 2026-07-26

### ✨ Highlights

- **Acumulador por traverser en el traversal fluido** (estilo `sack()` de
  Gremlin). Cada traverser acarrea un valor que se pliega con la arista en
  cada paso — la pieza que faltaba para cálculos como la explosión de
  materiales de un BOM, cadenas multiplicativas de probabilidad o conversión
  de unidades encadenada:

  ```rust
  let r = graph.traverse(raiz)
      .sack(10.0)
      .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
      .emit()
      .await?;   // (nodo, cantidad acumulada) por cada camino
  ```

- **Acceso a la arista durante el recorrido**: `sack_by`/`try_sack_by`
  reciben `&Edge`, y `filter_edge` filtra por propiedades de arista (antes
  solo se podía seleccionar por tipo).
- **Multiplicidad por camino**: sin deduplicación por nodo — el mismo nodo
  alcanzado por dos ramas aparece dos veces, cada una con su acumulador.
- **Truncamiento explícito**: `SackResult::truncated` distingue «terminé» de
  «me detuve por `max_depth`/`max_nodes`», y `TraversalResult.completed` hace
  lo mismo para `bfs`/`dfs`. Los ciclos dentro de `repeat` se manejan con
  `on_cycle(CycleMode::Error | Skip)` (default `Error`, con el camino del
  ciclo en el mensaje).

### Added

- `TraverseBuilder::sack(init)` → `SackBuilder<T>` genérico con
  `out/out_e/in_/in_e/both/filter/filter_edge`, pliegues `sack_by`,
  `try_sack_by`, conveniencias numéricas `sack_mul_by`/`sack_sum_by`
  (con coerción `Int`→`f64`; propiedad faltante o no numérica es error duro),
  `repeat(...)` acotado por `max_depth` (default 32, obligatorio) y
  `max_nodes` (default 10 000), y terminales `emit()` / `emit_leaves()`.
- Tipos nuevos exportados: `SackBuilder`, `SackBlock`, `SackResult`,
  `SackItem`, `CycleMode`, `Truncation`.
- `PropertyValue::as_number()` — valor numérico como `f64` coercionando
  `Int` (a diferencia de `as_f64`).
- Ejemplo `bom_costing` (`cargo run -p nopaldb --example bom_costing`).

### Changed

- **Minor breaking**: `TraversalResult` tiene un campo nuevo
  `completed: bool` (`false` si `bfs`/`dfs` se detuvieron por
  `max_depth`/`max_nodes`). Solo afecta a código que construya o destructure
  el struct literalmente; la lectura de campos no cambia.
- El manejo de ciclos (`on_cycle`) vive en el motor sack; en `bfs`/`dfs` la
  deduplicación por nodo ya garantiza terminación, así que `TraversalConfig`
  no cambia.

---

## [0.4.32] - 2026-07-22

### ⚖️ Licensing

- **Relicensed the `nopaldb` library to MPL-2.0.** The embeddable engine and its
  Python bindings (crates.io + PyPI) are now **Mozilla Public License 2.0** —
  file-level copyleft — so they can be embedded in projects under any license
  (MIT/Apache/proprietary) while changes to NopalDB's own files stay open. This
  unblocks adoption by permissively-licensed projects.
- The applications you *run* — **`nopaldb-mcp`** (MCP server) and **`ndbstudio`**
  (TUI) — remain **AGPL-3.0-only**, each with its own `LICENSE` file. The
  repository's top-level `LICENSE` is MPL-2.0 (the library is the primary
  artifact), so the repository's headline license reflects the library.
- Releases **≤ 0.4.31 remain AGPL-3.0-only** (published versions are immutable);
  this change applies from 0.4.32 onward.

---

## [0.4.31] - 2026-07-06

### ✨ Highlights
- **Cross-transaction WAL group commit**: concurrent commits queued at the single-writer applier share one fsync and apply in FIFO order. 8 concurrent committers: 98.3ms → 29.3ms (3.35×); single-commit latency unchanged.
- **`NqlResult` is iterable**: `for row in graph.execute_nql(...)` works as documented (delegates to the query result; write/index results iterate empty — use `.write`/`.summary`).
- **Adoption guide** ([docs/ADOPTION.md](docs/ADOPTION.md)): the fastest path in for Rust, Python, MCP and Studio users, plus the operational rules.

### Added
- NQL parser fuzzing (cargo-fuzz target, seeded corpus, nightly job — 5.26M executions, zero panics in the seed session).
- Dedicated PyPI project description (`pip install nopaldb` and Python quickstart instead of the crate README).

### Changed
- Python is now installable from PyPI; READMEs updated accordingly.

---

## [0.4.30] - 2026-07-06

### Fixed
- PyPI packaging: the sdist declared `License-File: LICENSE` but shipped the file nested under the crate subdirectory, so PyPI rejected the upload (400). The license (and README, for the project page) are now declared in `pyproject.toml` and land at the sdist root.

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
