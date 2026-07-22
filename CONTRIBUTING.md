# Contributing to NopalDB 🌵

Thank you for your interest in contributing to NopalDB! We welcome contributions from everyone.

## Getting Started

1.  **Read the Developer Guide**: If you are new to the codebase, check out our **[Developer Guide](docs/es/GUIA_DESARROLLO.md)** (in Spanish) for a detailed walkthrough.
2.  **Fork the repository** on GitHub.
3.  **Clone your fork** locally:
    ```bash
    git clone https://github.com/<your-user>/nopaldb.git
    cd nopaldb
    ```
4.  **Create a branch** for your feature or fix:
    ```bash
    git checkout -b feature/amazing-feature
    ```

## Development Environment

-   **Rust**: You need the latest stable Rust toolchain.
    ```bash
    rustup update stable
    ```
-   **Dependencies**: generic build tools (gcc, make, etc.) for some dependencies.

## Feature Tiers

NopalDB organizes features into tiers. Choose the one that matches your work:

```bash
cargo build -p nopaldb                        # default (Sled backend only)
cargo build -p nopaldb --features core        # + analytics, ML, algorithms, hypergraph
cargo build -p nopaldb --features semantic    # + OWL-EL reasoner, Turtle import/export
cargo build -p nopaldb --features full        # complete public feature set
```

Tiers are additive: `full ⊃ semantic ⊃ core`.

Python bindings are orthogonal — add `python` to any tier: `--features core,python`.

See **[Feature Tiers Guide](docs/FEATURE_TIERS.md)** for the complete reference, build recipes, and decision matrix.

## Running Tests

```bash
# Quick check (default features only)
cargo test -p nopaldb --lib

# Test the tier you're working on
cargo test -p nopaldb --features core --lib
cargo test -p nopaldb --features semantic --lib

# Full test suite (all features)
cargo test -p nopaldb --features full --lib

# Integration tests (require specific features)
cargo test --test owl_import_integration_test --features owl-import
cargo test --test p1_isolation_levels_test --features full-isolation

# Full QA package
make package-qa
```

## Code Style & Linting

We enforce strict code quality standards. Before submitting a PR, please run:

1.  **Format your code**:
    ```bash
    cargo fmt
    ```
2.  **Run Clippy** (ensure 0 warnings):
    ```bash
    cargo clippy -p nopaldb --features core -- -D warnings
    ```

## Commit Guidelines

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

-   `feat: add graph traversal algorithm`
-   `fix: resolve panic in edge deletion`
-   `docs: update README with new example`
-   `refactor: simplify query parser`
-   `test: add integration test for transactions`

## Pull Request Process

1.  Push your branch to your fork.
2.  Open a Pull Request against the `main` branch.
3.  Ensure all CI checks pass.
4.  Wait for review!

## Architecture Overview

```
nopaldb/src/
├── graph/          Core graph CRUD, adjacency maps, traversal
├── query/nql/      NQL parser (Pest) + executor (Volcano model)
├── transaction/    Write-buffering, commit/rollback, isolation dispatch
├── mvcc/           Multi-version node tracking, version chains
├── wal/            Write-Ahead Logging for crash recovery
├── storage/        Sled-backed KV storage
├── index/          Hash, B-Tree, Full-Text (Tantivy), Taxonomy
├── algorithms/     PageRank, centrality, clustering, community, shortest path
├── arrow_export/   Zero-copy Arrow IPC for ML pipelines
├── ml/             ML integrations (Arrow tensors, PyG)
├── reasoner/       OWL-EL reasoner (CR1+CR2+CR3)
├── rdf_owl/        Turtle importer/exporter
├── schema/         Runtime schema inference
└── python/         PyO3 bindings
```

## Security

Never commit secrets — `.env` files, API keys, private keys (`*.pem`, `*.key`), or
credentials of any kind. These patterns are already in `.gitignore`, but please double-check
your diffs before pushing. Consider running a scanner such as
[`gitleaks`](https://github.com/gitleaks/gitleaks) or
[`detect-secrets`](https://github.com/Yelp/detect-secrets) locally. If you believe you have
found a security vulnerability, please report it privately rather than opening a public issue.

## License

NopalDB is licensed per component. By contributing, you agree that your
contribution is licensed under the license of the component it touches:

- the **`nopaldb` library** (crate + Python bindings) → **MPL-2.0**;
- the **`nopaldb-mcp`** and **`ndbstudio`** applications → **AGPL-3.0-only**.
