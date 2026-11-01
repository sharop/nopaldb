# NQL semantics

> Español: [docs/es/SEMANTICA_NQL.md](es/SEMANTICA_NQL.md). Syntax and the full list of clauses: [NQL reference](en/NQL_REFERENCE.md).

This page states what an NQL query **means**: what a pattern produces, which rows a search ranks, in which order the clauses apply, and what every optimization must preserve. Executors and optimizations are free to change; these contracts are not. Each one cites the tests that check it. A change that breaks a test here changes the meaning of queries, so it is a breaking change, not an optimization.

## 1. Data model: rows are bindings, one per path

A pattern `(a:L1)-[r:T]->(b:L2)` produces one **binding** per match. A binding is a row that maps each variable to one node or edge. Rows form a **multiset** (a bag), not a set:

- Two different paths give two rows, even if they end on the same node. `c1 -MENTIONS-> e1 -RELATED-> f` and `c1 -MENTIONS-> e2 -RELATED-> f` are two rows `(c1, f)`.
- Parallel edges give one row each.
- `distinct` removes duplicate **projected** rows. It compares the columns of `find`, not the nodes behind them.
- A row has exactly the columns of `find` (since 0.10.3, #331): the alias when there is one (`find n.name as j` is the column `j` alone), `var.id`, `var.label` and each property for `all(var)`, every key for `*`. Until 0.10.2 an aliased property kept its original key too, `group by` and per-node algorithms added `n.id`, and `all(n)` on a single-node query gave empty rows. Tested by `nql_projection_columns_test`.
- The schema (names, order, columns of `*`) comes from `find`, not from the path that runs the query (#335): one column per projection in its order, the alias or the text of the expression as name (`count(*)`, `degree(n)`), `*` expanded to `var.id`, `var.label` and the properties sorted; a repeated name, a column outside `group by` and an aggregate in `order by` that is not projected are errors. Until 0.10.3 `count(*)` was the column `count`, `group by` reordered the columns, and a repeated alias or a non-grouped column was lost without an error. Tested by `nql_result_schema_test`, which compares the columns with and without an index and `at timestamp`.
- Arithmetic (`+ - * / %`, unary minus) and negation (`not`, `!`) mean the same in every clause and every path (#269): one definition of the operators (`executor::value_ops`) for conditions, `having`, computed columns, `order by` and `set`. Integers stay integers (`7 / 2` is `3`); an overflow or a division by zero is a named error; numbers compare by value (`3 = 3.0`); `not` is two-valued. A computed column (`r.p * 2 as d`) is evaluated once from its leaves, after the path gives them. Until 0.10.4 they were a parse error, because most evaluators read `!(…)` as true and arithmetic as no value. Tested by `nql_arithmetic_test`, also with and without an index and `at timestamp`.
- Several patterns in `from` are joined by their shared variables; patterns that share none give the cross product, by every path (#345). Until 0.10.4 a `from` of single-node patterns took the single-node path, which reads only the first pattern: `from (a:X), (b:Y)` gave the rows of `a` with `b` empty, and `count(*)` counted `a`. Tested by `nql_disconnected_patterns_test`, also with an index and `at timestamp`.
- Aggregations over several patterns and over a multi-hop pattern aggregate the rows of the join that pass the `where` (#362), with the semantics of the one-hop aggregation, which are SQL's: `count(*)` counts rows, `count(x)` the values present, `sum` and `avg` the numbers (`avg` of none is null), `min` and `max` order as `order by` does and are null with no values. The groups come in the order they first appear; `limit` counts groups, so the join does not stop early, and `Graph::max_join_rows` applies. Until 0.10.9 both were named errors. Tested by `nql_pattern_aggregations_test`, also with an index, a reordered join and `at timestamp`.
- Aggregation has one semantics by every path, the node path included (#364): the same question over the same rows gives the same values and the same groups in the same order whatever the shape of `from`. Without `order by` the groups come in the order they first appear, and a key is its value (a number stays a number; a missing key is a `null` group). Until 0.10.10 the node path counted every node in `count(x)`, divided `avg(x)` by every node, failed `min`/`max` with no values, took only a property as argument and ordered the groups by the text of the key (`10` before `9`), giving the keys as text; the one-hop path left the order of the groups undefined. Tested by `nql_aggregation_semantics_test`, which asks each question by the three paths.
- The join of several patterns applies each part of the `where` as soon as its variables are bound, never before (#349): a part that reads a variable of a later pattern waits for it, so a missing variable is never read as `false` early. The rows are the same as filtering the whole product at the end; the rows that pass count toward `Graph::max_join_rows`, and past it the query is a named error. A join by hash (a shared variable, or an equality with one side in each pattern) gives the same rows in the same order as the nested loop: the key only groups, every pair of a group still passes the `where`, numbers are grouped by value (`3 = 3.0`, `0 = -0.0`), and a `NaN`, which `=` gives equal to every integer, sends the join back to the nested loop. When the patterns are joined in another order (one not related to the ones before it waits), the rows are rebuilt and sorted back into the sequence of the nested loop over the order of `from`, paths included. Tested by `nql_join_limits_test`.
- Only `count`, `sum`, `avg`, `min`, `max` and `group by` merge rows. A per-node algorithm (`degree(n)`, `pagerank(n)`, `community(n)`…) gives one row per node (since 0.10.3, #336; until 0.10.2 `find degree(n)` gave one row for the whole graph, and with a property it grouped by it). Tested by `nql_algorithm_rows_test`.
- A *set of nodes*, where each node appears once at its minimum depth, is a different operation: `Graph.neighborhood(...)` / `Graph::neighborhood`. NQL does not turn paths into a neighbourhood implicitly.
- A seed with no valid expansion produces no row. Patterns are inner joins; there is no `OPTIONAL`.

Tested by: `nql_two_hops_test::k_bounds_the_seeds_and_limit_the_expanded_rows` (one row per path, `distinct` dedups), `nql_pattern_properties_test::two_hops_with_predicates_match_brute_force` (the bag of paths equals a brute-force enumeration).

## 2. Search: the universe, seed constraints and expansion constraints

`hybrid(v, …)` and `similar_to(v, …)` in the `where` rank the nodes of `v` and seed the pattern from them. The ranking is computed **inside a universe**:

```text
TopK( Filter(nodes of v, seed constraints), query )        -- what NQL computes
Filter( TopK(nodes of v, query), seed constraints )        -- what it does not compute
```

These are not equivalent. The first gives `k` results inside the filter whenever enough of them exist. The second can give fewer, or none.

- **Seed constraints** define the universe:
  - the label of `v`;
  - the equalities and `in` lists on `v` in the top-level `and` of the `where` (`v.tenant = "t7"`, `v.kind in ["a", "b"]`, `v.id = "…"`, `v.id in […]`);
  - inline properties of `v`'s node pattern;
  - `in_community(v, key[, via = "T"])`: the members of a materialized community, or the nodes with a `T` edge to one of them. It is the same universe, as a set, as the explicit pattern through `IN_COMMUNITY`.

  `score(v)`, `rank(v)` and the raw scores are computed in that universe.
- **Expansion constraints** are predicates on the other variables of the pattern (`e.kind = "lugar"`). They filter the expanded rows and **never change which seeds the search picks**: `k` counts seeds. A seed whose expansions all fail produces no row, so a query can return fewer than `k` seeds.
- Everything else is applied to the rows of the search and never widens the universe: ranges, `!=`, `not in`, `null`, functions, predicates on other variables, and conditions under `or`.
- `k = N` bounds the seeds. In a pattern, `LIMIT` bounds the expanded rows. Without `k`, a single-node query uses its `LIMIT` and a pattern uses 10.
- The search must be a condition of the top-level `and`. Under `or` or a negation it is an error.

Tested by:
- `nql_where_candidates_test::hybrid_returns_k_rows_inside_the_tenant`, `similar_to_returns_the_exact_top_k_inside_the_tenant`, `ids_are_the_universe_and_intersect`, `residual_predicates_still_filter_and_are_never_widened`, `a_search_must_be_a_root_conjunct` and `a_seeded_pattern_pushes_on_the_seed`;
- `nql_vector_literal_test::explicit_k_wins_over_limit_and_limit_still_caps_rows`;
- `nql_parens_and_literals_test::hybrid_and_a_parenthesised_or`;
- `nql_in_community_test::the_universe_equals_the_explicit_pattern_as_a_set` and `in_community_intersects_with_id_lists`;
- `candidate_set_test` (the universe for `search_hybrid(candidates=…)`).

## 3. Order of evaluation

A query is evaluated as if in this order:

1. **Seeds**: the search (inside its universe) or the scan of the first node pattern.
2. **Expansion**: the hops of the pattern, with direction, edge type, and the inline properties of nodes and edges.
3. **`where`**: the whole condition, on every row.
4. **Projection** of the `find` columns, with `score(...)` attached to the row of its seed.
5. **`distinct`**.
6. **`order by`**. Without it, rows keep the seed order: best first for a search.
7. **`limit` / `offset`**, which count rows after `distinct`.

An executor may do steps earlier when that cannot change the result. For example, it applies single-variable predicates while expanding (contract 5b), or stops reading early when there is a `limit` and neither `distinct` nor `order by`.

What is exact and what is approximate is reported, not hidden. `search_hybrid_explain` and the explain columns say which vector branch ran (`exact_over_allowed`, `exact_index`, `hnsw_filtered`, `label_checked`, `unfiltered`) and whether it was exact (`vector_exact`). See [HYBRID_SEARCH.md § How the vector path applies the filter](HYBRID_SEARCH.md#how-the-vector-path-applies-the-filter).

Tested by:
- `nql_pattern_expansion_test::one_hop_distinct_then_limit` and `single_node_distinct_then_limit` (`limit` after `distinct`);
- `p2_order_by_non_projected_test` (`order by` before `limit`);
- `nql_two_hops_test::hybrid_two_hops_equals_search_hybrid_plus_expansion` (seed order and scores travel with the rows).

## 4. One pattern, one meaning

Direction, edge type, inline properties and multiplicity mean the same thing in every place a pattern appears: reads and writes, one hop or several, with or without a search.

- `->` follows outgoing edges, `<-` incoming ones, and `<->` both. `(a)-[:T]->(b)` and `(b)<-[:T]-(a)` describe the same edges and give the same rows.
- `-[:T {w: 1}]->` keeps only the edges whose `w` equals 1. `(b {x: 2})` keeps only the nodes whose `x` equals 2.
- `delete` and `update` with a pattern change exactly the edges (or their endpoints) that `find` with the same pattern returns. They support one hop. A longer pattern or a quantifier is an error.

Tested by:
- `nql_pattern_properties_test::direction_is_symmetric` and `delete_with_a_pattern_deletes_what_find_returns`;
- `nql_pattern_expansion_test::one_hop_honours_the_direction`, `two_hops_honour_inline_edge_properties` and `write_patterns_honour_direction_and_inline_properties`.

## 5. Correctness contract: what every optimization preserves

a. **Filter safety.** Every row satisfies the whole `where`, and no search result lies outside its universe, at any selectivity.
b. **Pushdown and pruning invariance.** Moving a predicate earlier changes the cost, never the rows or their multiplicity. That covers pushing it into a search universe (seed constraints) and applying it while expanding (expansion constraints).
c. **Exactness when promised.** A branch that reports itself exact returns the same top-k as brute force over the universe. An approximate branch says so.
d. **Direction symmetry.** See contract 4.
e. **No silent fallbacks.** What NQL does not evaluate is a named error, never a condition that silently passes or fails. That includes a search under `or` or `not`, an expression an evaluator cannot compute, and a write pattern of more than one hop. Arithmetic `+ - * / %` and negation `!` / `not` were in this list until 0.10.4 (until 0.8.2 they were dropped: `x * 2 = 6` ran as `x = 6`); since [#269](https://github.com/sharop/nopaldb/issues/269) they are evaluated.

   Since 0.9.4 also: a bare property, variable or literal as a `where`/`having` condition is a named error (write `p.active = true`, not `p.active`); a projection alias in `where`/`having` stands for its expression (`find degree(n) as dg … where dg > 1`), unless it has the name of a pattern variable; an anonymous endpoint never takes a name of the pattern; a repeated variable must be the same node (`(a)-[:F]->(b)-[:F]->(a)` are the 2-cycles, `(a)-[:F]->(a)` the self-loops); an unknown `export` option is an error that lists the valid ones; `sketch` / `commit` through `execute_statement` are an error. Until 0.9.3 each of these gave wrong rows, or an OK answer, without an error.

   Since 0.10.3 also: a function NQL does not know is a named error in every clause, including `delete`/`update` and `explain` (`id(n)` points to `n.id`), and known function names are matched without case (`COUNT(*)` is `count(*)`). Until 0.10.2 an unknown function, or a known one in other case, was dropped: its column vanished, `where foo(n)` let every row through, `order by foo(n)` did nothing. The value of `update … set var.prop = <value>` must be a literal (#330): until 0.10.2 `set n.x = n.name` answered OK and updated nothing.
f. **Time travel reads one past graph.** `find … at timestamp <ts>` (since 0.10.2, #298) answers as the same query without the clause would have answered on the graph valid at `ts`: every node and edge in the version valid then, nothing created later, deleted ones still there. At the latest commit, or later, it is the current state. When the graph at `ts` is not fully known it is `HistoryUnavailable`, never a partial answer: `ts` below the GC horizon, or below the history floor of data written without a version before 0.10.1. Functions that read data with no history (`similar_to`, `hybrid`, `in_community`, the search scores, `has_embedding` and the path and pattern embedding functions) are a named error with `at timestamp`.

Tested by:
- (a) `nql_where_candidates_test::residual_predicates_still_filter_and_are_never_widened`, `candidate_set_test::results_lie_in_the_set_and_fill_k_when_the_set_allows` (property test), `hybrid_selective_filter_test`;
- (b) `nql_pattern_properties_test::two_hops_with_predicates_match_brute_force` and `partitioning_the_where_adds_up` (the rows without a filter are the rows of `P` plus the rows of its complement, the query-partitioning oracle), `nql_pattern_expansion_test::explain_names_the_expansion_filters`;
- (c) `knn_exact_test::exact_matches_brute_force_reference`, `nql_where_candidates_test::similar_to_returns_the_exact_top_k_inside_the_tenant`, and the unit test `graph::hybrid::tests::allowed_set_over_1024_small_for_its_index_ranks_exactly`;
- (d) contract 4;
- (e) `nql_arithmetic_test::overflow_and_division_by_zero_are_named_errors`, `nql_where_candidates_test::a_search_must_be_a_root_conjunct`, `nql_unknown_functions_test::an_unknown_function_is_a_named_error_in_every_clause`, `nql_unknown_functions_test::writes_with_an_unknown_function_are_errors_and_change_nothing`, `nql_unknown_functions_test::known_functions_are_matched_without_case`, `nql_update_set_values_test::a_value_it_cannot_evaluate_is_an_error_and_writes_nothing`;
- (f) `mvcc_stamp_model_proptest` (a node pattern and an edge pattern at every timestamp against a brute-force model, also after reopening), `nql_at_timestamp_test::en_el_ultimo_commit_es_el_presente`, `nql_at_timestamp_test::bajo_el_horizonte_del_gc`, `nql_at_timestamp_test::bajo_el_suelo_de_una_base_adoptada`, `nql_at_timestamp_test::funciones_sin_historia_rechazadas`.

## Comparisons and values

Current behaviour, which the tests pin down:

- A comparison on a property the node does not have is **false**, for `!=` too. `r.p != 3` does not return nodes without `p`.
- `= null` matches nothing, not even nodes without the property.
- `=` compares typed values: `3` (integer) is not `3.0` (float), and `"3"` is not `3`. `<`, `>`, `<=` and `>=` compare integers and floats numerically, so `3 >= 3.0` is true. Aligning `=` with the ordering is part of [#269](https://github.com/sharop/nopaldb/issues/269).
- Negative numbers are literals: `-5`, `-2.5`, in `where`, `set`, lists and function arguments.
- `and` binds tighter than `or`, and parentheses group: `a and (b or c)`.

## Out of scope today

These are named errors, never a different result:

- Several patterns in the `from` together with a search.
- Seeding a chain longer than two hops from a search.
- `OPTIONAL` patterns.
- `distinct` with a computed column (`r.p * 2`) or with `order by` an expression (#269).
- With `at timestamp`: vector and full-text searches, saved communities and embeddings, which have no history (contract 5f, [NQL reference § Time travel](en/NQL_REFERENCE.md#time-travel-at-timestamp)).
