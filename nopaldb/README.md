# NopalDB

[![Crates.io](https://img.shields.io/crates/v/nopaldb.svg)](https://crates.io/crates/nopaldb)
[![Docs.rs](https://docs.rs/nopaldb/badge.svg)](https://docs.rs/nopaldb)
[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)

High-performance embedded **graph database** with ACID transactions, MVCC
time-travel, a built-in query language (NQL), and Apache Arrow analytics.

## Features

- **ACID + MVCC** — snapshot isolation and time-travel queries over a versioned store.
- **NQL** — a Cypher-like query language for pattern matching and traversal.
- **Embedded** — runs in-process, no separate server. Sled-backed storage by default.
- **Analytics** — zero-copy export to Apache Arrow / Parquet (`analytics` feature).
- **Optional tiers** — vector embeddings + HNSW, full-text search, graph algorithms,
  ML tensors, OWL reasoning and SHACL validation, all behind feature flags.

## Install

```bash
cargo add nopaldb
```

Requires Rust ≥ 1.85 (edition 2024).

## Quickstart

```rust
use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Open (or create) an embedded graph at a path
    let graph = Graph::open("my_graph.db").await?;

    // Add nodes with properties
    let alice = graph
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = graph
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;

    // Connect them with an edge
    graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;

    Ok(())
}
```

## Feature flags

| Feature | Enables |
|---|---|
| `storage-sled` *(default)* | Sled-backed embedded storage |
| `analytics` | Apache Arrow / Parquet export |
| `ml` | ML tensors (implies `analytics`) |
| `algorithms` | Graph algorithms (community detection, centrality, …) |
| `embeddings`, `embeddings-index` | Vector embeddings + HNSW ANN index |
| `fulltext` | Full-text search (Tantivy) |
| `reasoner`, `owl-import`, `shacl` | OWL EL reasoning, RDF/OWL import, SHACL validation |
| `python` | PyO3 bindings (used to build the Python package) |

Convenience meta-features: `core`, `semantic`, `full`. See
[`Cargo.toml`](Cargo.toml) for the full list.

## Python

NopalDB also ships as a Python package built with [maturin]. See the
[repository][repo] for build instructions.

## License

Licensed under the **GNU Affero General Public License v3.0 only**
([AGPL-3.0-only](LICENSE)).

[maturin]: https://github.com/PyO3/maturin
[repo]: https://github.com/sharop/nopaldb