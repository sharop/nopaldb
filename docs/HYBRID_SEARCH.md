# Hybrid search

`search_hybrid` fuses the two retrieval paths NopalDB already has —full-text
(tantivy) and vector (HNSW)— with **Reciprocal Rank Fusion**, plus an optional
label/property filter. It is how you query a second brain that was written with
[`upsert`](UPSERT.md): text relevance and semantic similarity in one ranked list.

## How RRF works

Each path returns candidates in rank order. A document's fused score is:

```
score(d) = Σ_path 1 / (rrf_k + rank_path(d))        # rrf_k default 60
```

RRF is **rank-based**, which is what lets it fuse two branches whose numbers
are not comparable: a BM25 score and a cosine distance live on different
scales, but their rank orders do not. A node that ranks well in *both* paths
outranks one strong in only one. The raw scores are still available for
inspection — see [Explaining a result](#explaining-a-result).

## API

- **Rust:** `Graph::search_hybrid(HybridQuery) -> Vec<HybridHit>`.
- **Python:** `graph.search_hybrid(text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0)` → `list[dict]` of `{node_id, score, text_rank, vector_rank}`.
- **Rust:** `Graph::search_hybrid_explain(HybridQuery) -> HybridExplain` — same search, plus why each hit ranked where it did.
- **Python:** `graph.search_hybrid_explain(...)` with the same arguments → `dict`.
- **MCP:** the `search_hybrid` tool (read-only).
- **NQL:** `where hybrid(n, "text", "ref_name", "model")` — see below.

Provide `text`, `vector`, or both. `vector` requires `model`. The full-text path
needs an index created with `create index on <Label>(<property>) type fulltext`;
if `text_index` is omitted it is auto-discovered (preferring one whose label
matches the filter). The query text is analyzed with **that index's analyzer**
(language stemmer, stop words, accent folding, configured at `create index ...
with (...)`; see the NQL reference), so for a non-English corpus the BM25
branch keeps its recall instead of degrading the fusion to vector-only.

```python
hits = graph.search_hybrid(
    text="graph memory", vector=[...], model="e5-large",
    k=10, label="Chunk", props={"kind": "book"},
)
for h in hits:
    print(h["node_id"], h["score"], h["text_rank"], h["vector_rank"])
```

## Filter

`filter = {label?, props: [equalities]}` (AND). With properties it is applied
as a precomputed allowed-set (label index ∩ property-index lookups) that both
paths intersect. Until 0.6.11 the label part was a scan of every node (#207). v1 supports label + equality; ranges/OR are a follow-up.

**A label-only filter does not scan the label (0.6.10).** Each path checks the
label on its own candidates as it reads them, in rank order, and keeps the
first `k × overfetch` that carry it. The ranks are the ones the allowed-set
would give, because each path is still restricted *before* the fusion, so the
top-k is computed inside the label. Building the set instead meant reading every
node of the label on every query: 132 ms at 100k chunks for 40 candidates.

- **Text path** (`text_path = "global_label_checked"`): a full-text index
  already belongs to one label, so with the index of the filter's label the
  check only confirms.
- **Vector path** (`vector_path = "label_checked"`): it asks the index for
  `k × overfetch` neighbours. When fewer of them carry the label, it asks again
  with 4× as many, up to 4096. If the label is still that rare among the
  neighbours, it falls back to the allowed set with its scan and the exact
  branch below, so the result is never short because of the shortcut.

NQL `hybrid()` always passes the pattern's label as a filter, so it takes this
path too.

**Nothing outside the allowed-set is ever returned**, at any selectivity. That
is a hard guarantee, independent of how either path resolved the query.

## Candidate set

A filter says *which properties* a hit must have. Sometimes the caller already
knows *which nodes* are eligible — the chunks of the document the user
selected, an ACL, `tenant ∩ case ∩ authorized documents` resolved upstream —
and wants the ranking computed inside exactly that set. Since 0.6.15 (#214)
`search_hybrid` takes that set as `candidates`:

```python
hits = graph.search_hybrid(
    text="plazo de entrega", vector=q, model="e5-large", k=8,
    label="Fragment", props={"tenant_id": "B"},
    candidates=[chunk_id for chunk_id in authorized_chunks],
)
```

A candidate set is **not another filter: it is the definition of the universe
the retrieval runs in**. Two invariants hold, and both are tested
(`tests/candidate_set_test.rs`, including a property test over random subsets):

1. **It changes the scorable universe, never the score.** The result is
   `top-k(query, C)`, not `top-k(query, D) ∩ C`. The BM25 score and the cosine
   distance a node gets inside the set are the ones it gets without it: the
   text path scores the set in tantivy with the index-wide IDF
   (`term_set_over_allowed`), the vector path ranks the set's vectors exactly
   (`exact_over_allowed` / `exact_index`).
2. **It is never widened.** `label` and `props` can only keep or shrink the
   caller's set: every hit is a member of `candidates`. Ids that do not exist
   never score. That is what makes the set usable as an ACL.

Why it matters: the shape of #213 reappears on the caller's side whenever a
selection is applied *after* the search. One integrator's case: 65 fragments
in the same tenant and case, the only fragment of the selected document behind
64 closer distractors; asking for 32 inside the tenant filter and keeping the
document afterwards returned nothing, because the fragment was never in the
32. As a candidate set it comes back with a rank in both branches.

How the set is built, and what `search_hybrid_explain` reports as
`candidate_set = {source, caller_size, size}`:

| `candidates` | filter | universe | `source` |
|---|---|---|---|
| none | none | the whole graph | `unrestricted` |
| none | label only | the label, checked on each path's candidates (no set, `size = None`) | `filter` |
| none | with props | property index ∩ …, label confirmed on the survivors | `filter` |
| ids | none | the ids as given, unread: `size = caller_size` counts the distinct ids passed, not nodes confirmed to exist or to carry an embedding, so fewer hits than `size` here is not a filter effect | `ids` |
| ids | any | the ids are the seed of the same pipeline: property lookups intersect into them, the label is confirmed on what survives (`size ≤ caller_size`) | `ids_and_filter` |

`size` is the set both paths ran in, and `caller_size` the distinct ids the
caller passed (duplicates count once). A hit that is missing with
`size < caller_size` was left out when the set was built — by the filter — not
outranked; that is the question `allowed_set_size` alone could not answer.
Only a filter reads the ids: without one, an id that does not exist still
counts in `size` and simply never scores, so to confirm membership of a
caller set without a filter, pass `label` as well.

Cost is proportional to the set, not to the corpus. Measured on the
`retrieval` bench (100k chunks, 64 dims, release, k = 10, redb) with a set of
65 ids, what a tenant ∩ case leaves in one measured deployment; the second
column is 0.6.15, the third 0.6.16 (#224):

| query | 0.6.15 (ms) | 0.6.16 (ms) |
|---|---|---|
| text only, 65 candidates (term set) | 0.16 | 0.16 |
| vector only, 65 candidates (exact over the set) | 3.5 | 0.013 |
| text + vector, 65 candidates | 3.0 | 0.19 |
| text + vector, 65 candidates ∩ `{label, props}` (tenant lookup + label confirmed on the set) | 4.5 | 0.80 |
| text + vector, no set, no filter (reference) | 0.91 | 0.82 |

Until 0.6.15 the vector side dominated for two reasons that had nothing to do
with ranking: the exact path read each of the set's vectors from storage, and
opening the embeddings keyspace for that read was, on redb, a write
transaction per query that also dropped every cached read table (the
hydration that followed paid it again: 17 µs → 5.7 ms). Since 0.6.16 the
exact path takes the vectors from the index itself, which already holds them
in memory, and the embeddings keyspace is cached like the others. What is
left with a filter is the property lookup and the label confirmation on the
set.

The NQL `where`, Leiden communities and `neighborhood` results as candidate
sets, and a cost-based choice between the paths, are the second phase of
#214.

### How the text path applies the filter

The text path is **complete** under a filter: every allowed document that
matches the text is a candidate, whatever the selectivity. `text_path` in
`search_hybrid_explain` says how that was reached:

| Situation | `text_path` | Recall |
|---|---|---|
| no filter | `global`: top `k × overfetch` by BM25 | the top-N *is* the answer |
| label-only filter | `global_label_checked`: global top-N, label checked in rank order, fetch escalated ×4 | complete (the index belongs to the label) |
| property filter, allowed-set ≤ 512 | `term_set_over_allowed`: tantivy scores **only the allowed ids** (a term set over the stored `node_id`, AND-ed with the text); no global query | complete |
| property filter, larger allowed-set, enough allowed documents inside the global top-N (escalating ×4 to 1000) | `global` | complete |
| property filter, larger allowed-set, escalation reaches 1000 still short | `global_then_term_set`: the allowed-set scored in chunks of 512 and merged | complete |

The question the branch answers is always *top-k of the text within the
allowed-set*; the global top-N is only an opportunistic way to compute it,
never a definition of the result. The scores are the same on every path —
the term set contributes 0 to the score, and BM25's IDF is index-wide — so a
document ranks where the global top-N would rank it; the filter only decides
which documents are looked at.
A short text branch under a filter therefore means *no more allowed documents
match the text*, which is what a RAG layer needs before it abstains.

**Cost.** The global top-N costs what BM25 over every matching document
costs (0.25 ms per request at 100k chunks that all match); the term set
costs ~4.3 µs per allowed id regardless of corpus size (8 ids → 0.09 ms,
128 → 0.68 ms, 512 → 2.2 ms, 1000 → 3.8 ms, 4000 → 12 ms; `text_filter`
bench, release). The 512 cutoff is where a term set costs about what the
escalation it replaces (128 → 512 → 1000 global hits, ~2 ms) costs; it is
also why small sets skip the global query altogether: that query's cost
grows with the documents that contain the term (corpus size), the term
set's does not. Queries
whose allowed-set is large *and* whose allowed documents all rank behind
1000 foreign ones pay both — the rare case, and the only one that was wrong
before. End to end at 100k chunks (`retrieval` bench, release): text-only
with a filter that leaves 20 chunks 0.23 ms, with one that leaves 1% (1000
chunks, the chunked fallback) 11.4 ms; the same searches with a vector too
2.4 ms and 17.5 ms; unfiltered 0.60 ms.

The cutoff is by set size only. A *rare* query term makes the global path
cheap (0.02 ms per request, 0.56 ms for the whole escalation at 1% document
frequency) while the term set still costs its 2.3 ms at 512 ids, so sets of
a few hundred with rare terms pay more than they could; choosing by term
frequency as well is the planner of #214 (`CandidateSet`), which also makes
any graph predicate — a community, a neighbourhood, an id list — the
candidate space of both branches.

The allowed-set itself starts from the property index (ids only) and the
label is confirmed on those candidates; until 0.6.12 it started from the
label, reading every node of it (197 ms for a 100k-node label, whatever the
filter left).

> **Changed in 0.6.13 (#213).** Before, the text path fetched a *global*
> top-N by BM25 (escalating to 1000) and filtered afterwards, treating the
> 1000 cap as "no more matches". With a selective filter the allowed
> documents could all sit behind 1000 foreign ones — 1 allowed fragment
> among 1024 that contain the term returned **0 hits** with
> `underfilled = true` — the same design the vector path had dropped in
> 0.5.5. Nothing disallowed ever leaked; the result was incomplete. Queries
> that filled their candidates before are unchanged (same documents, same
> order, same scores); the ones that came back short now come back complete,
> and in a text+vector search a document that only the vector branch found
> now carries its text rank too, so it can move up in the fusion.

### How the vector path applies the filter

Recall — *finding all the allowed neighbours that exist* — depends on which
branch runs. The planner picks one from the allowed-set cardinality and the
index size:

| Situation | Branch | Recall |
|---|---|---|
| index ≤ 1024 vectors | exact scan over every point, then filter | exact |
| allowed-set ≤ 1024 within a larger index | exact cosine over just the allowed vectors | exact |
| larger allowed-set | HNSW walk with the predicate applied natively during traversal, escalating `ef_search` until `k` allowed hits are collected (capped) | approximate |

The middle row is the important one: with a selective filter, scoring the
allowed vectors directly is both exact *and* cheaper than walking a large
graph, so a narrow query does not pay for the index being big.

Only the last row is approximate. If it collects fewer than `k` allowed hits it
stops at the escalation cap rather than walking the whole index — so a result
shorter than `k` means "no more allowed neighbours were found", which is not
always the same as "no more exist".

### Cost

A filtered HNSW walk gets *more* expensive as the filter gets *more*
selective: fewer allowed points means the traversal explores further before its
result heap fills. Measured on a 100k-vector index (dim 32, `k=10`,
`ef_search=30`, release build), recall was 10/10 at every selectivity below:

| allowed-set | share of index | per query |
|---:|---:|---:|
| 105 | 0.1% | 74 ms |
| 988 | 1% | 14 ms |
| 1,109 | 1.1% | 15 ms |
| 9,927 | 10% | 2.2 ms |
| 100,000 | 100% | 0.5 ms |

This is why the exact branch exists, and why its cutoff sits where it does:
below ~1024 allowed nodes, reading those vectors and scoring them directly
costs about the same as the walk — and is exact instead of approximate. Above
it the walk is the cheaper option. Queries whose allowed-set lands just over
the cutoff on a very large index are the costliest case; partitioning the index
by the filtering property is the answer there.

> **Changed in 0.5.5.** Before, the vector path fetched `k × 4` *global*
> neighbours and filtered them afterwards. With a selective filter the allowed
> neighbours were often not among those candidates, so the search silently
> returned fewer results than it should have — measured recall dropped to
> **0** at ~1% selectivity while the exact answer had `k` hits available.
> Output was never wrong (nothing disallowed leaked), just incomplete.

## Explaining a result

The RRF score orders results but does not explain them. It is a sum of
reciprocal *ranks*, so it cannot tell you whether a document rose through
text, through vectors, or both — nor how strongly. `search_hybrid_explain`
runs the same search and keeps the raw numbers the fusion consumes and throws
away.

```python
e = graph.search_hybrid_explain(text="cactus", vector=[...], model="e5-large", k=5)

for h in e["hits"]:
    print(h["node_id"], h["text_rank"], h["text_score"], h["vector_rank"], h["vector_distance"])

print(e["vector_path"], e["text"], e["vector"])
```

Per hit: `text_score` is the raw BM25 score, `vector_distance` the cosine
distance (0 = identical). Each is `None` when the hit did not come through
that branch — "no score" rather than a zero that would read as "scored badly".

Globally it reports the configuration that was actually **used**, including
what you did not choose: the resolved `text_index`, the effective `ef_search`,
`candidates` (= `k × overfetch`), `allowed_set_size` when the filter built
an allowed set (`None` with a label-only filter resolved as `label_checked`)
and `candidate_set = {source, caller_size, size}`: where the universe came
from and its size before and after the filter (see *Candidate set*).

Each branch reports `{requested, returned, underfilled}`. Underfill is the
question a short result cannot answer on its own. For the text branch,
`text_path` (0.6.13) names the path taken and every one of them is complete
under a filter — a short text branch means no more allowed documents match
the text. For the vector branch, read `vector_exact` (0.6.14, #218): `True`
means the branch had recall = 1, so a short result means there genuinely are
no more allowed vectors; `False` means the HNSW walk found no more, which is
not the same. It is resolved from `vector_path` and the index size, so the
caller does not need the table below, which is there to say *why*:

| `vector_path` | What ran | `vector_exact` |
|---|---|---|
| `exact_over_allowed` | small allowed set over a large index: exact cosine over just those vectors, read from the index itself (0.6.16, #224) | `True` |
| `exact_index` | filter over an index of at most 1024 points: the index scans every point and applies the predicate | `True` |
| `hnsw_filtered` | HNSW walk with the predicate applied during traversal, `ef_search` escalated up to 4096 | `False` |
| `unfiltered` / `label_checked` | plain KNN on the index (the label checked on the candidates in rank order) | `True` while the index has at most 1024 points and reads exactly, else `False` |

Until 0.6.13, a filter over a small index was reported as `hnsw_filtered`
although the index had scanned it exactly, and `unfiltered` was documented as
exact at any size. A caller abstaining on a short result would read a
definitive answer as an uncertain one; the flag closes that gap.

`search_hybrid` is the same computation with the trace dropped — it delegates
to the explaining version rather than duplicating it, so asking for the
explanation cannot change the result.

## NQL `hybrid()`

`hybrid(n, "text", "ref_name", "model")` in a WHERE clause filters the pattern to
the top-K hybrid results, best first. The vector is the embedding of the
reference node resolved by its `name` property (the same convention as
`similar_to`), or — since 0.6.9 — a literal written in the query:
`hybrid(n, text = "…", vector = [0.1, -0.2, …], model = "m", k = 10)`, with
`text` and/or `vector`. K is `k = N`, else the query `LIMIT` (default 10). Since
0.6.17 the FIND can project `score(n)` (RRF, at most `2/rrf_k`), `rank(n)`,
`text_score(n)` and `vector_score(n)` (#228; see the NQL reference § score for
the scales and the right threshold for each). The
top-K is computed **inside the FROM pattern's label**: `(n:Chunk)` becomes the hybrid filter, so a better-matching
node of another label cannot push a `Chunk` out of the K (before 0.5.19 the
search ran over every label and the stream dropped the foreign hits afterwards,
which with `limit 1` could leave the query with zero rows).

Every parameter of `HybridQuery` is reachable through named options (after the
four positional arguments, or in the all-named form):

```nql
find n.name, n.body
from (n:Chunk)
where hybrid(n, "graph memory", "current_query", "e5-large",
             rrf_k = 30, ef_search = 128, overfetch = 8, text_index = "Chunk_body")
limit 10
```

| option | type | default | meaning |
|---|---|---|---|
| `rrf_k` | number > 0 | 60 | the RRF constant; lower makes rank 1 weigh more |
| `ef_search` | integer ≥ 1 | 30 | HNSW beam width for the vector branch |
| `overfetch` | integer ≥ 1 | 4 | each branch fetches `K × overfetch` candidates before fusion |
| `text_index` | string | auto | full-text index name (`Label_property`); auto picks the first one matching the label |

An unknown option, a value of the wrong kind, or a wrong number of positional
arguments is a validation error that names the problem. (Until 0.5.19 a
malformed `hybrid(...)` was silently ignored and the predicate passed every
node.) `explain find ...` prints the effective parameters, defaults included
(a literal vector shows as `<literal dim=N>`, never its components):

```
Hybrid: hybrid(n): text="graph memory" ref="current_query" model="e5-large" k=10 rrf_k=30 overfetch=8 ef_search=128 text_index=Chunk_body filter.label=Chunk
```

The search can also seed a pattern of one or two hops — search and expand in
one query, `score(c)` on every expanded row (`EXPLAIN` says `PATTERN PIPELINE
(seed: HYBRID, hops: 2)` and a `Seeds:` line with `k` and the `LIMIT`); see
[GRAPHRAG.md](GRAPHRAG.md) for the placement rules.

## Limits & notes (v1)

- The fusion itself uses rank, not score — that is what makes RRF work across
  two branches whose numbers are not comparable. The raw scores are still
  available for inspection via `search_hybrid_explain`. Choose the right
  property when creating the full-text index (it is per-property).
- `search_hybrid` sees **committed** state; a freshly added embedding is
  inserted into the cached HNSW index by `add_node_embedding` (0.5.19) and is
  visible to the next search.
- Follow-ups: per-path weights; range/OR filters; property filters from the
  NQL `where` (today only the pattern label reaches the hybrid filter; the
  other predicates, including those on the expanded nodes of a one- or
  two-hop pattern, are applied afterwards, so they can leave fewer than K
  rows — the second phase of #214 turns them into a candidate set; from Rust
  and Python, `candidates` already does that today).
