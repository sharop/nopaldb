# Feature Tiers & Compilation Guide

**[Version en Espanol abajo](#guia-de-tiers-y-compilacion)**

---

## Overview

NopalDB uses Cargo feature flags to group public Rust capabilities into tiers. This keeps fast builds small while letting users opt into analytics and semantic tooling. Python bindings are built separately with `maturin`.

```
full
  └─ semantic
       └─ core
            └─ default
```

Tiers are additive: `full` includes `semantic` plus `full-isolation`, `semantic` includes `core`, and `core` includes the default Sled storage backend. These tiers are Rust-only; they do not build the Python wrapper.

---

## Tier Reference

### `default` — Minimal Graph Storage

The default build enables the Sled-backed graph database with MVCC, transactions, WAL, indexing, and NQL basics.

```bash
cargo build -p nopaldb
cargo test  -p nopaldb --lib
```

### `core` — Analytics & Algorithms

Everything in `default` plus Arrow/Parquet export, ML data helpers, graph algorithms, hypergraph support, embeddings, and full-text indexing.

| Feature | What it enables |
|---------|-----------------|
| `analytics` | Apache Arrow + Parquet columnar export |
| `ml` | ML integrations and Arrow tensor helpers |
| `algorithms` | PageRank, centrality, clustering, community detection, shortest path |
| `hypergraph` | Hyperedges via `EdgeTarget` |
| `embeddings` / `embeddings-index` | Semantic embeddings and HNSW indexing |
| `fulltext` | Tantivy-backed full-text indexes |
| `full-isolation` | Isolation levels, lock manager and deadlock detection (in the `full` tier) |

```bash
cargo build -p nopaldb --features core
cargo test  -p nopaldb --features core --lib
```

### `semantic` — Ontology & SHACL

Everything in `core` plus OWL-EL reasoning, Turtle import/export, and SHACL validation.

| Feature | What it enables |
|---------|-----------------|
| `reasoner` | EL Reasoner (CR1 transitivity, CR2 conjunction, CR3 existential) |
| `owl-import` | OWL/Turtle import/export helpers |
| `shacl` | SHACL validation constraints |

```bash
cargo build -p nopaldb --features semantic
cargo test  -p nopaldb --features semantic --lib
```

### `full` — Complete Public Feature Set

`full` is the complete tier: everything in `semantic` plus `full-isolation` (transaction isolation levels, per-node lock manager and deadlock detection — see [ISOLATION_LEVELS.md](ISOLATION_LEVELS.md)).

```bash
cargo build -p nopaldb --features full
cargo test  -p nopaldb --features full --lib
```

---

## Common Build Recipes

### Rust library

```bash
cargo build -p nopaldb
cargo build -p nopaldb --features core
cargo build -p nopaldb --features semantic
cargo build -p nopaldb --features full
```

### Python wheel

```bash
pip install maturin
make build-wheel

# Local development install
cd nopaldb
maturin develop --release --features python-full
```

`cargo build -p nopaldb --features full` only builds the Rust library. The Python wrapper must be built with `maturin` because PyO3 needs Python-specific linker configuration.

### Run tests by tier

```bash
cargo test -p nopaldb --lib
cargo test -p nopaldb --features core --lib
cargo test -p nopaldb --features semantic --lib
cargo test -p nopaldb --features full --lib
```

### Clippy per tier

```bash
cargo clippy -p nopaldb -- -D warnings
cargo clippy -p nopaldb --features core -- -D warnings
cargo clippy -p nopaldb --features semantic -- -D warnings
cargo clippy -p nopaldb --features full -- -D warnings
```

---

## Atomic Features Reference

| Feature | Dependencies |
|---------|--------------|
| `storage-sled` | none |
| `analytics` | `arrow`, `parquet` |
| `ml` | `analytics` |
| `algorithms` | none |
| `hypergraph` | none |
| `embeddings` | none |
| `embeddings-index` | `embeddings`, `hnsw_rs` |
| `hnsw-simd` | `embeddings-index`, `hnsw_rs/simdeez_f` |
| `fulltext` | `tantivy` |
| `reasoner` | none |
| `owl-import` | `reasoner` |
| `shacl` | `regex` |
| `python` | `pyo3` |
| `python-reasoner` | `python`, `reasoner` |
| `python-owl` | `python`, `owl-import` |
| `python-full` | `python`, `python-reasoner`, `python-owl`, `analytics` |

---

## Decision Matrix

| I want to... | Use this |
|--------------|----------|
| Embed a graph database in a Rust app | `cargo build -p nopaldb` |
| Export data to Arrow or Parquet | `--features core` |
| Run graph algorithms | `--features core` |
| Use OWL ontologies or Turtle files | `--features semantic` |
| Validate SHACL shapes | `--features semantic` or `--features shacl` |
| Use isolation levels / deadlock detection | `--features full` or `--features full-isolation` |
| Build the Python package | `make build-wheel` or `maturin develop --features python-full` |
| Enable every public capability | `--features full` |

---

## Guia de Tiers y Compilacion

### Resumen

NopalDB organiza sus capacidades publicas con feature flags de Cargo:

```
full
  └─ semantic
       └─ core
            └─ default
```

Los tiers son aditivos: `full` incluye `semantic` mas `full-isolation` (niveles de aislamiento, lock manager y deteccion de deadlocks — ver [ISOLATION_LEVELS.md](ISOLATION_LEVELS.md)), `semantic` incluye `core`, y `core` incluye el backend Sled por defecto. Estos tiers son solo Rust; no construyen el wrapper Python.

### Compilacion rapida por tier

```bash
# Motor minimo
cargo build -p nopaldb

# Core: analytics + ML + algoritmos + hipergrafos + embeddings + full-text
cargo build -p nopaldb --features core

# Semantic: + reasoner OWL-EL + Turtle + SHACL
cargo build -p nopaldb --features semantic

# Full: conjunto publico completo
cargo build -p nopaldb --features full
```

### Tests por tier

```bash
cargo test -p nopaldb --lib
cargo test -p nopaldb --features core --lib
cargo test -p nopaldb --features semantic --lib
cargo test -p nopaldb --features full --lib
```

### Wheel Python

```bash
pip install maturin
make build-wheel

# Instalacion local para desarrollo
cd nopaldb
maturin develop --release --features python-full
```

`cargo build -p nopaldb --features full` solo compila la libreria Rust. El wrapper Python debe compilarse con `maturin`, porque PyO3 necesita configuracion de linker especifica de Python.

### Matriz de decision

| Quiero... | Usar |
|-----------|------|
| Embeber un grafo en mi app Rust | `cargo build -p nopaldb` |
| Exportar a Arrow/Parquet | `--features core` |
| Ejecutar algoritmos de grafos | `--features core` |
| Usar ontologias OWL o Turtle | `--features semantic` |
| Validar shapes SHACL | `--features semantic` o `--features shacl` |
| Usar niveles de aislamiento / deteccion de deadlocks | `--features full` o `--features full-isolation` |
| Construir el paquete Python | `make build-wheel` o `maturin develop --features python-full` |
| Activar todas las capacidades publicas | `--features full` |
