
<img width="200" height="200" alt="NopalDB logo" src="https://raw.githubusercontent.com/sharop/nopaldb/main/assets/nopaldb_logo.png" />

# NopalDB™ 🌵

[![Crates.io](https://img.shields.io/crates/v/nopaldb.svg)](https://crates.io/crates/nopaldb)
[![PyPI](https://img.shields.io/pypi/v/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![Python](https://img.shields.io/pypi/pyversions/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![docs.rs](https://docs.rs/nopaldb/badge.svg)](https://docs.rs/nopaldb)
[![CI](https://github.com/sharop/nopaldb/actions/workflows/community-ci.yml/badge.svg)](https://github.com/sharop/nopaldb/actions/workflows/community-ci.yml)
[![Nightly](https://github.com/sharop/nopaldb/actions/workflows/nightly.yml/badge.svg)](https://github.com/sharop/nopaldb/actions/workflows/nightly.yml)
[![Crates.io downloads](https://img.shields.io/crates/d/nopaldb.svg)](https://crates.io/crates/nopaldb)
[![PyPI downloads](https://img.shields.io/pypi/dm/nopaldb.svg)](https://pypi.org/project/nopaldb/)
[![Library: MPL-2.0](https://img.shields.io/badge/library-MPL--2.0-brightgreen.svg)](nopaldb/LICENSE)
[![Apps: AGPL-3.0-only](https://img.shields.io/badge/apps-AGPL--3.0--only-blue.svg)](LICENSE)

A high-performance embedded graph database written in Rust with **MVCC**, **ACID transactions**, **Apache Arrow integration**, and **Python bindings**.

**[English](#english) | [Español](#español)**

---

## English

### Features ✨

#### Core Database
- ✅ **Property Graph Model** - Nodes, edges, and properties
- ✅ **Persistent Storage** - Sled-based backend
- ✅ **ACID Transactions** - Full transaction support
- ✅ **MVCC** - Multi-Version Concurrency Control with snapshot isolation
- ✅ **Isolation Levels** - `ReadCommitted` (default), `RepeatableRead`, `Serializable` with per-node locking and deadlock detection, via the `full-isolation` feature ([guide](docs/ISOLATION_LEVELS.md), [deadlock detection](docs/DEADLOCK_DETECTION.md))
- ✅ **Write-Ahead Logging (WAL)** - Crash recovery and durability
- ✅ **NQL Query Language** - property-graph pattern graph queries
- ✅ **Apache Arrow Export** - Zero-copy ML pipelines
- ✅ **Python Bindings** - PyO3-powered Python API

#### Query Language (NQL)
- ✅ Pattern matching (nodes and relationships)
- ✅ Filtering (WHERE clauses)
- ✅ Multi-hop queries
- ✅ Pagination (LIMIT/OFFSET)
- ✅ ORDER BY / GROUP BY
- ✅ EXPORT to CSV/JSON

#### Python Integration
- ✅ Full API (Graph, Transaction, QueryResult)
- ✅ Arrow export to Pandas/PyTorch/Polars
- ✅ Zero-copy data transfer
- ✅ ML pipeline ready

#### Performance
- ✅ Async/await with Tokio
- ✅ Columnar export (Arrow/Parquet)
- ✅ Efficient indexing
- ✅ Batch operations

#### Advanced Analytics 🧠
- ✅ **Graph Algorithms** - PageRank, Centrality, Clustering, Communities, Shortest Path
- ✅ **Schema Inspection** - Runtime graph schema analysis
- ✅ **Aggregations** - `count`, `sum`, `avg`, `min`, `max` in NQL
- ✅ **Algorithms in NQL** - `pagerank()`, `community()`, `shortestPath()`, `degree()`

#### Semantic Layer 🧬
- ✅ **OWL-EL Reasoner** - CR1 (transitivity) + CR2 (conjunction) + CR3 (existential)
- ✅ **Turtle Import/Export** - RDF/OWL ontology files
- ✅ **NQL Ontology Predicates** - `instanceOf()`, `subClassOf()`
- ✅ **Hypergraph** - Hyperedges via `EdgeTarget`

---

### Quick Start 🚀

#### Rust
```rust
use nopaldb::{Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    // Create database
    let graph = Graph::open("./data.db").await?;
    
    // Begin transaction
    let mut tx = graph.begin_transaction().await?;
    
    // Add node
    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    
    tx.add_node(alice).await?;
    tx.commit().await?;
    
    // Query with NQL
    let result = graph.execute_nql(r#"
        find p.name, p.age
        from (p:Person)
        where p.age > 25
    "#).await?;
    
    for row in result.rows() {
        println!("{:?}", row);
    }
    
    Ok(())
}
```

#### Python
```python
import nopaldb

# Create database
graph = nopaldb.Graph.open("./data.db")

# Begin transaction
tx = graph.begin_transaction()

# Add nodes
alice = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "city": "CDMX"
})

bob = tx.add_node("Person", {
    "name": "Bob",
    "age": 25
})

# Add relationship
tx.add_edge(alice, bob, "KNOWS")
tx.commit()

# Query with NQL
result = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")

for row in result:
    print(f"{row['p.name']}: {row['p.age']} years old")
```

---

### Incremental ingestion — idempotent `upsert` 🔁

For pipelines that re-run over changing sources (RAG indexers, second-brain
ingestion, incremental data pipelines), `upsert` writes the *desired state* of a
node keyed by a business key. Re-running over unchanged data performs **zero writes**.

```python
graph = nopaldb.Graph.in_memory()

# create → update → no-op, all under a stable NodeId
graph.upsert("Chunk", "key", {"key": "note:intro", "path": "a.md"})   # ('created',   <id>)
graph.upsert("Chunk", "key", {"key": "note:intro", "path": "b.md"})   # ('updated',   <id>)  (same id)
graph.upsert("Chunk", "key", {"key": "note:intro", "path": "b.md"})   # ('unchanged', <id>)  (no writes)

# attach an embedding and wikilink-style edges (stub targets are created if absent)
graph.upsert(
    "Note", "key", {"key": "note:intro", "title": "Intro"},
    vector=[...], model="e5-large",
    links=[{"type": "MENTIONS", "target_label": "Note",
            "target_key": "key", "target_key_value": "note:zettelkasten", "stub": True}],
)

# batch form
graph.upsert_many([
    {"label": "Note", "key": "key", "props": {"key": "note:a"}},
    {"label": "Note", "key": "key", "props": {"key": "note:b"}},
])
```

The same operation is available in Rust (`Graph::upsert_node`) and as the
`upsert_node` MCP tool. See [docs/UPSERT.md](docs/UPSERT.md) for semantics,
concurrency guarantees, and current limits.

---

### Hybrid search — full-text + vector (RRF) 🔎

`search_hybrid` fuses full-text (tantivy) and vector (HNSW) retrieval with
Reciprocal Rank Fusion, with an optional label/property filter — how you query a
second brain written with `upsert`.

```python
# needs a fulltext index: create index on Chunk(body) type fulltext
hits = graph.search_hybrid(
    text="graph memory", vector=[...], model="e5-large",
    k=10, label="Chunk", props={"kind": "book"},
)
# hits: [{"node_id", "score", "text_rank", "vector_rank"}, ...] best first
```

Also available in Rust (`Graph::search_hybrid`), as the `search_hybrid` MCP tool,
and inside NQL:

```nql
find n.name from (n:Chunk)
where hybrid(n, "graph memory", "current_query", "e5-large")
limit 10
```

See [docs/HYBRID_SEARCH.md](docs/HYBRID_SEARCH.md) for RRF details, the filter, and limits.

---

### NQL Export (CSV/JSON)

```nql
find p.name, p.age
from (p:Person)
order by p.age
export csv with path="users.csv", header=true
```

```nql
find p.name, p.age
from (p:Person)
limit 100
export json with jsonl=true
```

Notes:
- Recommended syntax: `export` goes at the end of the query.
- Prefix form (`export ...` before `find`) is not supported.
- Export path is optional and must be passed with `path="..."`. If omitted, results are returned inline.
- Arrow/Parquet exports are available via the Graph API (`to_arrow()`, `export_parquet()`).

---

### Installation 📦

#### Rust
```toml
[dependencies]
nopaldb = "0.3"

# With a specific tier
nopaldb = { version = "0.4", features = ["core"] }        # analytics + ML + algorithms + full-text
nopaldb = { version = "0.4", features = ["semantic"] }     # + OWL reasoner + SHACL
nopaldb = { version = "0.4", features = ["full"] }         # complete feature set (includes full-isolation)
```

#### Python
```bash
pip install nopaldb
```

Prebuilt wheels for Linux, macOS and Windows (CPython 3.10+). To build from
source instead: `pip install maturin && maturin develop --release --features python-full`
(from the `nopaldb/` directory).

`cargo build -p nopaldb --features full` builds the Rust library only. The Python wrapper is built with `maturin` because PyO3 needs Python-specific linker configuration.

See **[Feature Tiers Guide](docs/FEATURE_TIERS.md)** for detailed compilation options.

---

### Examples 📚

#### Rust Examples
See `examples/`:
- `basic_usage.rs` - Getting started
- `transaction_demo.rs` - ACID transactions
- `rpg_quest_system.rs` - RPG quest dependencies
- `skill_tree.rs` - Game skill trees
- `character_network.rs` - Social networks

#### Python Examples
See `examples/`:
- `test_nql.py` - NQL queries
- `test_transactions.py` - Transaction API
- `test_arrow_properties.py` - Arrow export + ML

---

### Documentation 📖

#### Query Language
- **[NQL Reference (English)](docs/en/NQL_REFERENCE.md)** - Complete syntax
- **[NQL Tutorial (English)](docs/en/NQL_TUTORIAL.md)** - Learn step-by-step
- **[Referencia NQL (Español)](docs/es/NQL_REFERENCIA.md)** - Sintaxis completa
- **[Tutorial NQL (Español)](docs/es/NQL_TUTORIAL.md)** - Aprende paso a paso

#### API Documentation
- **[Python API Guide](python/README.md)** - Python bindings
- **[Rust API Docs](https://docs.rs/nopaldb)** - Rust documentation (coming soon)

#### Build & Features
- **[Feature Tiers Guide](docs/FEATURE_TIERS.md)** - How to compile by role (researcher, developer, production)
- **[NDBStudio Web Quickstart](docs/ndbstudio/web_quickstart.md)** - Launch the local web workbench for graph and session analysis

#### Architecture
- **[Adoption Guide](docs/ADOPTION.md)** - Fastest path in for Rust, Python, MCP and Studio users
- **[Architecture Overview](docs/ARCHITECTURE.md)** - System design
- **[Durability Guarantees](docs/DURABILITY.md)** - Crash-safety model and what survives SIGKILL
- **[Arrow Integration](docs/arrow/01-OVERVIEW.md)** - Arrow/ML pipeline docs
- **[Graph Algorithms](docs/ALGORITHMS.md)** - Algorithm reference

---

### Apache Arrow Integration 🏹

Export graph data to Arrow for zero-copy ML pipelines:
```python
import pyarrow as pa
import pandas as pd

# Export to Arrow
arrow_bytes = graph.to_arrow(label="Person")
reader = pa.ipc.open_stream(arrow_bytes)
batch = reader.read_next_batch()

# Zero-copy to Pandas
df = batch.to_pandas()

# Use with ML frameworks
X = df[['feature1', 'feature2']].values
model.fit(X, y)
```

Integrates with:
- **Pandas** - DataFrames
- **Polars** - Fast DataFrames
- **DuckDB** - SQL queries
- **PyTorch** - Deep learning
- **Scikit-learn** - ML models

---

### Architecture 🏗️
```
┌─────────────────────────────────────┐
│         Python Bindings             │
│           (PyO3)                    │
├─────────────────────────────────────┤
│         Query Engine                │
│    (NQL Parser + Executor)          │
├─────────────────────────────────────┤
│      Transaction Layer              │
│    (MVCC + ACID + WAL)              │
├─────────────────────────────────────┤
│       Storage Engine                │
│    (Sled + Indices)                 │
└─────────────────────────────────────┘
        ↕                    ↕
   Persistent            Apache Arrow
     Disk               (ML Pipelines)
```

---

### Operational Model 🧩

- **One process per data directory.** The storage engine takes a file lock on the database directory; a second process opening the same path will fail with a "could not acquire lock" error. Close the other process (app, MCP server, or NDBStudio) first.
- **Share within the process by cloning the handle.** `Graph` is `Clone + Send + Sync`; clone it (cheap, `Arc`-backed) to use the same database from multiple threads or tasks.
- For bulk ingestion use `BulkLoader`; for update-heavy datasets enable version GC with `start_auto_gc`.

---

### Project Status 🗺️

NopalDB Community includes the graph storage layer, MVCC transactions, isolation levels with deadlock detection (`full-isolation`), WAL crash recovery, NQL query execution, full-text search, Arrow export, Python bindings, graph algorithms, OWL-EL reasoning, Turtle import/export, SHACL validation, and feature tiers for compiling only the capabilities you need.

The current public Rust tiers are `default`, `core`, `semantic`, and `full`. Python wheels use the separate `python-full` feature through `maturin`. See **[Feature Tiers Guide](docs/FEATURE_TIERS.md)** for build recipes.

---


### Contributing 🤝

Contributions are welcome! Areas where help is needed:

- 🐛 Bug reports and fixes
- 📚 Documentation improvements
- 🌐 Translations (FR, PT, ZH, JA)
- 🧪 Test cases
- ⚡ Performance optimizations
- 🎨 Examples and tutorials

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

### Use Cases 💡

- **AI second brain** - Ingest a markdown vault, query it with hybrid search + the wikilink graph ([20-line example](tutorials/second_brain/README.md))
- **Social Networks** - Friends, followers, recommendations
- **Knowledge Graphs** - Entities, relationships, reasoning
- **Fraud Detection** - Transaction networks (Synthetic Offshore Network demo)
- **Game Development** - Quest systems, skill trees
- **ML Pipelines** - Zero-copy feature extraction
- **Network Analysis** - Infrastructure, dependencies

---

### License 📄

Copyright © 2026 Sergio Haro Pérez (Sharop).

NopalDB is licensed **per component** (effective v0.4.32; releases ≤ 0.4.31 remain AGPL-3.0-only):

| Component | Crate / package | License |
|-----------|-----------------|---------|
| **Library** (embeddable engine + Python bindings) | `nopaldb` (crates.io + PyPI) | **MPL-2.0** ([nopaldb/LICENSE](./nopaldb/LICENSE)) |
| **MCP server** (application) | `nopaldb-mcp` | AGPL-3.0-only ([LICENSE](./LICENSE)) |
| **NDStudio TUI** (application) | `ndbstudio` | AGPL-3.0-only ([LICENSE](./LICENSE)) |

The library is **MPL-2.0** — file-level copyleft — so it can be embedded in projects under any license (including MIT/Apache and proprietary), while improvements to NopalDB's own files stay open. The applications you *run* (`nopaldb-mcp`, `ndbstudio`) remain **AGPL-3.0-only**.

NopalDB™ is a trademark of Sergio Haro Pérez.

The license grants rights to the source code only. No rights are granted to use the NopalDB name, logos, branding, or trademarks.

All other product names, logos, and brands referenced in this project (e.g. Apache Arrow, PyTorch, SNOMED CT, and any other third-party names) are the property of their respective owners and are used here for identification and educational purposes only. Such use does not imply any affiliation with or endorsement by the trademark holders.

---

### Acknowledgments 🙏

Built with:
- 🦀 **Rust** - Systems programming language
- 🐍 **Python** - ML/Data science integration
- 🏹 **Apache Arrow** - Columnar data format
- 🗄️ **Sled** - Embedded database
- ⚡ **Tokio** - Async runtime

---

### Contact 📬

- **GitHub:** [sharop/nopaldb](https://github.com/sharop/nopaldb)
- **Issues:** [Report bugs](https://github.com/sharop/nopaldb/issues)
- **Discussions:** [Community](https://github.com/sharop/nopaldb/discussions)

---

**Built with ❤️ en 🇲🇽**
---

## Español

### Características ✨

#### Base de Datos Principal
- ✅ **Modelo de Grafo de Propiedades** - Nodos, aristas y propiedades
- ✅ **Almacenamiento Persistente** - Backend basado en Sled
- ✅ **Transacciones ACID** - Soporte completo de transacciones
- ✅ **MVCC** - Control de Concurrencia Multi-Versión con aislamiento snapshot
- ✅ **Niveles de Aislamiento** - `ReadCommitted` (default), `RepeatableRead`, `Serializable` con locking por nodo y detección de deadlocks, vía la feature `full-isolation` ([guía](docs/ISOLATION_LEVELS.md), [detección de deadlocks](docs/DEADLOCK_DETECTION.md))
- ✅ **Write-Ahead Logging (WAL)** - Recuperación ante fallos y durabilidad
- ✅ **Lenguaje de Consultas NQL** - Consultas de grafos basadas en patrones de grafos de propiedades
- ✅ **Exportación Apache Arrow** - Pipelines ML zero-copy
- ✅ **Bindings Python** - API Python con PyO3

#### Lenguaje de Consultas (NQL)
- ✅ Coincidencia de patrones (nodos y relaciones)
- ✅ Filtrado (cláusulas WHERE)
- ✅ Consultas multi-salto
- ✅ Paginación (LIMIT/OFFSET)
- ✅ ORDER BY / GROUP BY
- ✅ EXPORT a CSV/JSON

#### Analítica Avanzada 🧠
- ✅ **Algoritmos de Grafos** - PageRank, Centralidad, Clustering, Comunidades, Camino más corto
- ✅ **Inspección de Schema** - Análisis de esquema en tiempo de ejecución
- ✅ **Agregaciones** - `count`, `sum`, `avg`, `min`, `max` en NQL
- ✅ **Algoritmos en NQL** - `pagerank()`, `community()`, `shortestPath()`, `degree()`

#### Capa Semántica 🧬
- ✅ **Reasoner OWL-EL** - CR1 (transitividad) + CR2 (conjunción) + CR3 (existencial)
- ✅ **Import/Export Turtle** - Archivos de ontología RDF/OWL
- ✅ **Predicados de Ontología en NQL** - `instanceOf()`, `subClassOf()`
- ✅ **Hipergrafos** - Hiperaristas via `EdgeTarget`

#### Integración Python
- ✅ API completa (Graph, Transaction, QueryResult)
- ✅ Exportación Arrow a Pandas/PyTorch/Polars
- ✅ Transferencia de datos zero-copy
- ✅ Lista para pipelines ML

---

### Inicio Rápido 🚀

#### Python
```python
import nopaldb

# Crear base de datos
graph = nopaldb.Graph.open("./datos.db")

# Comenzar transacción
tx = graph.begin_transaction()

# Agregar nodos
alice = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "city": "CDMX"
})

bob = tx.add_node("Person", {
    "name": "Bob",
    "age": 25
})

# Agregar relación
tx.add_edge(alice, bob, "CONOCE")
tx.commit()

# Consultar con NQL
resultado = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")

for fila in resultado:
    print(f"{fila['p.name']}: {fila['p.age']} años")
```

---

### Exportación NQL (CSV/JSON)

```nql
find p.name, p.age
from (p:Person)
order by p.age
export csv with path="usuarios.csv", header=true
```

```nql
find p.name, p.age
from (p:Person)
limit 100
export json with jsonl=true
```

Notas:
- Sintaxis recomendada: `export` va al final de la consulta.
- La forma con prefijo (`export ...` antes de `find`) no está soportada.
- La ruta de exportación es opcional y se pasa con `path="..."`. Si se omite, el resultado se devuelve inline.
- Arrow/Parquet se exportan vía la API de Graph (`to_arrow()`, `export_parquet()`).

---

### Instalación 📦

#### Rust
```toml
[dependencies]
nopaldb = "0.3"

# Con un tier específico
nopaldb = { version = "0.4", features = ["core"] }        # analytics + ML + algoritmos + full-text
nopaldb = { version = "0.4", features = ["semantic"] }     # + reasoner OWL + SHACL
nopaldb = { version = "0.4", features = ["full"] }         # conjunto completo (incluye full-isolation)
```

#### Python
```bash
pip install nopaldb
```

Wheels preconstruidas para Linux, macOS y Windows (CPython 3.10+). Para compilar
desde fuente: `pip install maturin && maturin develop --release --features python-full`
(desde el directorio `nopaldb/`).

`cargo build -p nopaldb --features full` solo compila la librería Rust. El wrapper Python se compila con `maturin`, porque PyO3 necesita configuración de linker específica de Python.

Ver **[Guía de Feature Tiers](docs/FEATURE_TIERS.md)** para opciones de compilación detalladas.

---

### Documentación 📖

#### Lenguaje de Consultas
- **[Referencia NQL (Español)](docs/es/NQL_REFERENCIA.md)** - Sintaxis completa
- **[Tutorial NQL (Español)](docs/es/NQL_TUTORIAL.md)** - Aprende paso a paso
- **[NQL Reference (English)](docs/en/NQL_REFERENCE.md)** - Complete syntax
- **[NQL Tutorial (English)](docs/en/NQL_TUTORIAL.md)** - Learn step-by-step

#### Documentación API
- **[Índice de Documentación (Español)](docs/es/README.md)** - Guía central de docs y runbooks
- **[Guía de Adopción](docs/ADOPTION.md)** - La ruta rápida para Rust, Python, MCP y Studio
- **[Guía API Python](python/README.md)** - Bindings Python

#### Versionado (SemVer)
- **Patch (`X.Y.Z`)**: incremento automático por push a `main` mediante PR de bot (`.github/workflows/auto-bump-patch.yml`).
- **Minor (`X.Y.0`)**: incremento manual cuando se agrupan features para release.
- **Major (`X.0.0`)**: incremento manual para breaking changes.
- Fuente de verdad de versión: `Cargo.toml`, `nopaldb/Cargo.toml`, `nopaldb/pyproject.toml`.

---

### Modelo Operacional 🧩

- **Un proceso por directorio de datos.** El motor de almacenamiento toma un file lock sobre el directorio; un segundo proceso que abra la misma ruta fallará con "could not acquire lock". Cierra primero el otro proceso (app, servidor MCP o NDBStudio).
- **Comparte dentro del proceso clonando el handle.** `Graph` es `Clone + Send + Sync`; clónalo (barato, respaldado por `Arc`) para usar la misma base desde varios hilos o tasks.
- Para cargas masivas usa `BulkLoader`; con datasets de muchas actualizaciones habilita el GC de versiones con `start_auto_gc`.

---

### Casos de Uso 💡

- **Redes Sociales** - Amigos, seguidores, recomendaciones
- **Grafos de Conocimiento** - Entidades, relaciones, razonamiento
- **Detección de Fraude** - Redes de transacciones (demo Synthetic Offshore Network)
- **Desarrollo de Videojuegos** - Sistemas de misiones, árboles de habilidades
- **Pipelines ML** - Extracción de features zero-copy
- **Análisis de Redes** - Infraestructura, dependencias

---

### Licencia 📄

Copyright © 2026 Sergio Haro Pérez (Sharop).

NopalDB tiene licencia **por componente** (a partir de v0.4.32; las versiones ≤ 0.4.31 permanecen AGPL-3.0-only):

| Componente | Crate / paquete | Licencia |
|------------|-----------------|----------|
| **Librería** (motor embebible + bindings Python) | `nopaldb` (crates.io + PyPI) | **MPL-2.0** ([nopaldb/LICENSE](./nopaldb/LICENSE)) |
| **Servidor MCP** (aplicación) | `nopaldb-mcp` | AGPL-3.0-only ([LICENSE](./LICENSE)) |
| **TUI NDStudio** (aplicación) | `ndbstudio` | AGPL-3.0-only ([LICENSE](./LICENSE)) |

La librería es **MPL-2.0** —copyleft por archivo— así que puede embeberse en proyectos con cualquier licencia (incluidas MIT/Apache y propietarias), mientras que las mejoras a los archivos propios de NopalDB siguen abiertas. Las aplicaciones que *ejecutas* (`nopaldb-mcp`, `ndbstudio`) siguen siendo **AGPL-3.0-only**.

NopalDB™ es una marca de Sergio Haro Pérez.

La licencia concede derechos únicamente sobre el código fuente. No concede derechos para usar el nombre NopalDB, logotipos, identidad visual, branding o marcas.

Todos los demás nombres de productos, logotipos y marcas mencionados en este proyecto (p. ej. Apache Arrow, PyTorch, SNOMED CT y cualquier otro nombre de terceros) son propiedad de sus respectivos dueños y se usan aquí únicamente con fines de identificación y educativos. Dicho uso no implica afiliación ni respaldo por parte de los titulares de las marcas.

---

**Construido con ❤️ en 🇲🇽**
