# Durability Guarantees

What NopalDB guarantees when the process dies at an arbitrary point (e.g. `SIGKILL`, power loss at the OS-buffer level notwithstanding), and what it does not. Continuously exercised by the SIGKILL crash harnesses (`nopaldb/tests/crash_commit_test.rs`, `crash_mixed_load_test.rs`) run on every CI build and nightly with 100 kill rounds.

## Committed transactions

A `Transaction::commit()` that **returned `Ok`** is durable:

1. The whole write-set (Begin + operations + Commit) is written to the WAL as **one batch with one fsync** before any storage mutation. If the fsync did not complete, `commit()` never returned.
2. Storage application happens after WAL durability. If the process dies mid-application, the next `Graph::open()` **replays** the committed operations from the WAL (redo), rebuilding MVCC version chains with their original commit timestamps.
3. The per-node version write-set (previous-version invalidation, new version, current pointer, version lists, current record) applies as a **single atomic storage batch** — a crash cannot leave a node without a current version.

A commit whose `Ok` was never observed may or may not be durable (standard "committed but unacknowledged" semantics): if its Commit record reached the WAL, it will be replayed; otherwise it is discarded.

## Torn WAL tails

A crash mid-append can leave a torn record at the WAL tail (incomplete length prefix, truncated payload, or undecodable bytes). On open, the log is scanned tolerantly and **truncated to the last valid record**. A torn record was never acknowledged, so discarding it is safe. Databases never become unopenable due to a torn tail.

## Derived structures

Adjacency lists and property indexes are persisted incrementally by the single-writer apply funnel, and are **not** journaled in the WAL. After a crash recovery (WAL replay ran), adjacency is **rebuilt from the edges** — the source of truth — instead of trusting possibly-stale snapshots. Structural invariants (no orphaned edges, bidirectional adjacency consistency, no duplicates, index↔data consistency) are asserted by the crash harnesses on every reopen.

## Logical clocks

MVCC timestamps and transaction ids are persisted as monotonic bounds and restored on open (falling back to the WAL maxima and a version scan for older databases). Time-travel ordering (`history()`, `as_of()`) is preserved across restarts and crashes.

## Direct (non-transactional) writes — weaker guarantees

`graph.add_node()`, `add_edge()`, `delete_node()`, `delete_edge()` **do not write to the WAL**. Their durability relies on the storage engine's periodic flush (`flush_every_ms`, profile-dependent: 500–3000 ms). This means:

- A direct write acknowledged less than one flush interval before a crash **may be lost**.
- WAL replay is tolerant of this: committed operations whose targets were later removed by direct writes are skipped rather than resurrected.

If you need per-operation durability, use transactions. Use direct writes for bulk/throwaway ingestion or data you can regenerate.

## Scope

- Durability is at the `fsync` level; disks or VMs that lie about `fsync` weaken every layer equally.
- One process per data directory (see the Operational Model in the README); the on-disk state is only defined when the owning process is the single writer.
