# Benchmark QA - community() exact vs community_fast() (Release)

Date: 2026-03-11  
Command:

```bash
cargo run -p nopaldb --release --example benchmark_community_dual
```

## Raw Latency (ms)

| Scenario | community exact (cold) avg/p50/p95 | community exact (cached) avg/p50/p95 | community_fast avg/p50/p95 |
|---|---:|---:|---:|
| small (300 nodes, span=2) | 36.26 / 36.26 / 36.26 | 0.56 / 0.54 / 0.56 | 1.25 / 1.26 / 1.30 |
| medium (1200 nodes, span=4) | 270.97 / 270.97 / 270.97 | 2.09 / 2.05 / 2.17 | 7.88 / 7.87 / 7.98 |
| large (4000 nodes, span=6) | 186.35 / 186.35 / 186.35 | 6.91 / 6.82 / 7.14 | 38.04 / 38.02 / 38.22 |

## Speedup Summary

Speedup formula: `baseline / candidate` (higher is better).

| Scenario | cold exact vs cached exact | cold exact vs fast |
|---|---:|---:|
| small | 64.8x | 29.0x |
| medium | 129.7x | 34.4x |
| large | 27.0x | 4.9x |

## Operational Read

- `community(n)` exact cold is the expensive path.
- Repeated exact runs are dramatically faster due to partition cache reuse.
- In this synthetic benchmark, `community_fast(n)` is slower than cached exact because the exact cache hit is very cheap; `community_fast` remains useful when exact cold would be re-triggered often by topology mutations or when exploring before cache is hot.
- For QA/research workflow:
  - exploration with frequent mutations: test `community_fast(n)`.
  - final/reproducible results: use `community(n)` exact.

## Notes

- Benchmarks were run in release profile on local machine; absolute numbers are hardware-dependent.
- The benchmark uses in-memory graph generation and a deterministic topology pattern.

## Rerun After Release Profile v1 (2026-03-11)

Workspace `Cargo.toml` release profile was normalized as v1 (`opt-level=3`, `lto=true`, `codegen-units=1`, `strip=true`, `panic=\"abort\"`).

| Scenario | community exact (cold) avg/p50/p95 | community exact (cached) avg/p50/p95 | community_fast avg/p50/p95 |
|---|---:|---:|---:|
| small (300 nodes, span=2) | 36.44 / 36.44 / 36.44 | 0.55 / 0.53 / 0.58 | 1.23 / 1.21 / 1.30 |
| medium (1200 nodes, span=4) | 271.86 / 271.86 / 271.86 | 1.96 / 1.92 / 2.02 | 7.57 / 7.57 / 7.74 |
| large (4000 nodes, span=6) | 191.38 / 191.38 / 191.38 | 8.00 / 8.11 / 9.48 | 39.68 / 38.31 / 46.20 |
