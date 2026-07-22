# Idempotent upsert

`upsert_node` writes the desired state of a node identified by a business key
`(label, key_property, value)` — create if absent, update if changed, no-op if
identical. Re-running the same upsert over unchanged data costs **zero writes**,
which is what incremental pipelines (RAG indexers, incremental indexers,
second-brain ingestion) need.

## API

- **Rust:** `Graph::upsert_node(UpsertRequest) -> (UpsertOutcome, NodeId)` and
  `Graph::upsert_batch(Vec<UpsertRequest>)`.
- **Python:** `graph.upsert(label, key, props, vector=None, model=None, links=None)`
  returns `(outcome, node_id)`; `graph.upsert_many([...])`. `outcome` is one of
  `"created" | "updated" | "unchanged"`.
- **MCP:** the `upsert_node` tool (requires the server *not* be in `--readonly`).

`props` is the complete desired property map and must contain the key property.

### Links (wikilink pattern)

Each link is reconciled as an outgoing edge. The target is resolved by its own
business key; if it does not exist and `stub` is set, a stub node
`{target_key: target_key_value}` is created — the Obsidian pattern of linking to
a note that may not exist yet. A later upsert of that target "fills" the stub
(same NodeId). Re-running an upsert never duplicates an existing edge.

## Semantics

| Existing match | Result |
|---|---|
| 0 | **Created** — new node with a fresh NodeId. |
| 1, identical props + links + embedding | **Unchanged** — transaction is rolled back; nothing is written. |
| 1, something differs | **Updated** — node overwritten under its existing NodeId; property index reconciled; missing links added; embedding refreshed. |
| >1 | Error `AmbiguousUpsertKey` — deduplicate the key first. |

Updates preserve the NodeId. Changing the embedding vector refreshes the HNSW
index (rebuilt on demand).

## Concurrency & limits (v1)

- **Same-key races** are serialized by a process-global per-key lock, so two
  concurrent creates of the same key converge to one node. This is best-effort,
  **not** a transactional unique constraint: separate processes, or a crash
  between the lookup and the commit, could still create duplicates. A real
  unique index is the structural fix (tracked follow-up).
- **Links are additive** — v1 adds missing edges but does not delete edges that
  are no longer in `links` (tracked follow-up).
- **Batch** currently loops `upsert_node`; a batched fast path (one tx per batch,
  HNSW batch build) is a tracked follow-up.

## See also

For the delete counterpart and the full-sync reconcile pattern, see
[docs/INCREMENTAL_INGESTION.md](INCREMENTAL_INGESTION.md).
