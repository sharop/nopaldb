# NopalDB Documentation · Documentación

Single index for every document in `docs/`, in English with the Spanish pages marked (ES).

## Start here · Empieza aquí

- [Feature Tiers & Compilation Guide](FEATURE_TIERS.md) — how to build by role (EN/ES)
- [Adoption Guide](ADOPTION.md) — fastest path in for Rust and Python; operational model
- [Migrating to 0.6](MIGRATION_0.6.md) · [ES](es/MIGRACION_0.6.md) — redb is the default engine; sled databases still open; how to migrate
- [Durability Guarantees](DURABILITY.md) — what survives a crash, per write type and engine
- [Backup & read-only](BACKUP_AND_READ_ONLY.md)
- [Operating](OPERATIONS.md) — what `get_stats()` / `nopaldb stats` say for each symptom (slow open, slow ingestion, growing WAL, empty full-text results) and the progress event

## Query language (NQL)

- [NQL Reference (EN)](en/NQL_REFERENCE.md) · [Referencia NQL (ES)](es/NQL_REFERENCIA.md)
- [NQL Tutorial (EN)](en/NQL_TUTORIAL.md) · [Tutorial NQL (ES)](es/NQL_TUTORIAL.md)
- [Hands-on CRUD (ES)](NQL_WRITE_CRUD_HANDS_ON.md)
- [Guided tutorial, four acts (ES)](tutorial/README.md) — run in CI on every push

## Python

- [Python docs index](python/README.md) · [Quick Start](python/QUICKSTART.md) · [API Reference](python/API_REFERENCE.md) · [Configuration](python/CONFIGURATION.md)
- [NQL Guide](python/NQL_GUIDE.md) · [Edge patterns](python/EDGE_PATTERNS.md) · [Schema inspection](python/SCHEMA_INSPECTION.md) · [Arrow export](python/ARROW_EXPORT.md) · [Examples](python/EXAMPLES.md)
- [Referencia API Python (resumen en español)](es/API_PYTHON.md) · [Configuración (ES)](es/CONFIGURACION.md)

## Writing data

- [Idempotent upsert](UPSERT.md) · [Incremental ingestion](INCREMENTAL_INGESTION.md)

## Search

- [Embeddings & vector search](EMBEDDINGS.md) · [HNSW in NopalDB](HNSW_ALGORITHM.md)
- [Hybrid search (full-text + vector, RRF)](HYBRID_SEARCH.md) · [GraphRAG retrieval cycle](GRAPHRAG.md) — search, hydrate, expand in one call each
- [Indexing system](INDEXING_DOCS.MD) · [Property index (layout v2, ES)](PROPERTY_INDEXING.md) · [Query planner](QUERY_PLANNER_DOCS.MD)

## Semantic tier

- [SHACL validation](SHACL.md) · [ES](es/SHACL.md) — the Turtle bridge and the reasoner are described in [ARCHITECTURE.md](ARCHITECTURE.md) § Tier semántico

## Engineering

- [Architecture](ARCHITECTURE.md) — layers, MVCC, WAL decision, planner, feature tiers
- [Isolation levels](ISOLATION_LEVELS.md) · [Deadlock detection](DEADLOCK_DETECTION.md)
- [Graph algorithms](ALGORITHMS.md) · [ML examples (GNN, fraud, recommendation)](ML_EXAMPLES.md)
- [Guía de desarrollo (ES)](es/GUIA_DESARROLLO.md) · [Arquitectura del ejecutor Volcano (ES)](es/ARQUITECTURA_EJECUTOR.md)

## Arrow & ML

- [Overview](arrow/01-OVERVIEW.md) · [Quickstart](arrow/02-QUICKSTART.md) · [Technical](arrow/03-TECHNICAL.md) · [Examples](arrow/04-EXAMPLES.md) · [ML integration](arrow/05-ML-INTEGRATION.md) · [Performance](arrow/06-PERFORMANCE.md)

## Project

- [Roadmap](../ROADMAP.md) — direction by theme and what is out of scope
- [Changelog](../CHANGELOG.md)
- [Examples](../nopaldb/examples/) — Rust and Python programs, run by CI where marked
- [Contributing](../CONTRIBUTING.md)

## Ecosystem

The applications born in this repo live in their own repositories (AGPL-3.0, with full history):

- [NDBStudio (TUI / web workbench)](https://github.com/Anxious-Mind-Group/ndbstudio)
- [MCP server](https://github.com/Anxious-Mind-Group/nopaldb-mcp)
