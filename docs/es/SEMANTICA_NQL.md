# Semántica de NQL

> English: [docs/NQL_SEMANTICS.md](../NQL_SEMANTICS.md). Sintaxis y lista completa de cláusulas: [referencia NQL](NQL_REFERENCIA.md).

Esta página dice qué **significa** una consulta NQL: qué produce un patrón, qué filas ordena una búsqueda, en qué orden se aplican las cláusulas y qué tiene que preservar toda optimización. Los ejecutores y las optimizaciones pueden cambiar; estos contratos no. Cada uno cita los tests que lo comprueban. Un cambio que rompe uno de esos tests cambia el significado de las consultas: es un cambio incompatible, no una optimización.

## 1. Modelo de datos: las filas son vinculaciones, una por camino

Un patrón `(a:L1)-[r:T]->(b:L2)` produce una **vinculación** por cada coincidencia. Una vinculación es una fila que asigna a cada variable un nodo o una arista. Las filas forman un **multiconjunto** (bag), no un conjunto:

- Dos caminos distintos dan dos filas, aunque terminen en el mismo nodo. `c1 -MENTIONS-> e1 -RELATED-> f` y `c1 -MENTIONS-> e2 -RELATED-> f` son dos filas `(c1, f)`.
- Las aristas paralelas dan una fila cada una.
- `distinct` quita filas **proyectadas** repetidas. Compara las columnas del `find`, no los nodos que hay detrás.
- Una fila tiene exactamente las columnas del `find` (desde 0.10.3, #331): el alias cuando lo hay (`find n.nombre as j` es solo la columna `j`), `var.id`, `var.label` y cada propiedad con `all(var)`, todas las claves con `*`. Hasta 0.10.2 una propiedad con alias conservaba también su clave original, `group by` y los algoritmos por nodo añadían `n.id`, y `all(n)` en una consulta de un solo nodo daba filas vacías. Lo prueba `nql_projection_columns_test`.
- Un *conjunto de nodos*, donde cada nodo aparece una vez a su profundidad mínima, es otra operación: `Graph.neighborhood(...)` / `Graph::neighborhood`. NQL no convierte caminos en vecindad de forma implícita.
- Una semilla sin expansiones válidas no produce filas. Los patrones son *inner joins*; no hay `OPTIONAL`.

Lo comprueban: `nql_two_hops_test::k_bounds_the_seeds_and_limit_the_expanded_rows` (una fila por camino, `distinct` deduplica), `nql_pattern_properties_test::two_hops_with_predicates_match_brute_force` (el multiconjunto de caminos es igual a una enumeración por fuerza bruta).

## 2. Búsqueda: universo, restricciones de semilla y de expansión

`hybrid(v, …)` y `similar_to(v, …)` en el `where` ordenan los nodos de `v` y siembran el patrón desde ellos. El ranking se calcula **dentro de un universo**:

```text
TopK( Filtro(nodos de v, restricciones de semilla), consulta )   -- lo que calcula NQL
Filtro( TopK(nodos de v, consulta), restricciones de semilla )   -- lo que no calcula
```

No son equivalentes. La primera da `k` resultados dentro del filtro siempre que existan suficientes. La segunda puede dar menos, o ninguno.

- **Restricciones de semilla**: definen el universo.
  - la etiqueta de `v`;
  - las igualdades y listas `in` sobre `v` en el `and` raíz del `where` (`v.tenant = "t7"`, `v.kind in ["a", "b"]`, `v.id = "…"`, `v.id in […]`);
  - las propiedades inline del nodo `v`;
  - `in_community(v, key[, via = "T"])`: los miembros de una comunidad materializada, o los nodos con una arista `T` hacia uno de ellos. Es el mismo universo, como conjunto, que el patrón explícito por `IN_COMMUNITY`.

  `score(v)`, `rank(v)` y los scores crudos se calculan en ese universo.
- **Restricciones de expansión**: predicados sobre las otras variables del patrón (`e.kind = "lugar"`). Filtran las filas expandidas y **nunca cambian qué semillas elige la búsqueda**: `k` cuenta semillas. Una semilla cuyas expansiones fallan todas no produce filas, así que una consulta puede devolver menos de `k` semillas.
- Todo lo demás se aplica a las filas de la búsqueda y nunca amplía el universo: rangos, `!=`, `not in`, `null`, funciones, predicados sobre otras variables y condiciones bajo `or`.
- `k = N` acota las semillas. En un patrón, `LIMIT` acota las filas expandidas. Sin `k`, una consulta de un nodo usa su `LIMIT` y un patrón usa 10.
- La búsqueda tiene que ser una condición del `and` raíz. Bajo `or` o una negación da error.

Lo comprueban:
- `nql_where_candidates_test::hybrid_returns_k_rows_inside_the_tenant`, `similar_to_returns_the_exact_top_k_inside_the_tenant`, `ids_are_the_universe_and_intersect`, `residual_predicates_still_filter_and_are_never_widened`, `a_search_must_be_a_root_conjunct` y `a_seeded_pattern_pushes_on_the_seed`;
- `nql_vector_literal_test::explicit_k_wins_over_limit_and_limit_still_caps_rows`;
- `nql_parens_and_literals_test::hybrid_and_a_parenthesised_or`;
- `nql_in_community_test::the_universe_equals_the_explicit_pattern_as_a_set` e `in_community_intersects_with_id_lists`;
- `candidate_set_test` (el universo de `search_hybrid(candidates=…)`).

## 3. Orden de evaluación

Una consulta se evalúa como si siguiera este orden:

1. **Semillas**: la búsqueda (dentro de su universo) o el recorrido del primer nodo del patrón.
2. **Expansión**: los saltos del patrón, con dirección, tipo de arista y propiedades inline de nodos y aristas.
3. **`where`**: la condición entera, sobre cada fila.
4. **Proyección** de las columnas del `find`, con `score(...)` pegado a la fila de su semilla.
5. **`distinct`**.
6. **`order by`**. Sin él, las filas conservan el orden de las semillas: primero la mejor, en una búsqueda.
7. **`limit` / `offset`**, que cuentan filas después de `distinct`.

Un ejecutor puede adelantar pasos cuando eso no puede cambiar el resultado. Por ejemplo, aplica los predicados de una sola variable al expandir (contrato 5b), o deja de leer antes cuando hay `limit` y no hay ni `distinct` ni `order by`.

Qué es exacto y qué es aproximado se informa, no se oculta. `search_hybrid_explain` y las columnas de explicación dicen qué rama vectorial corrió (`exact_over_allowed`, `exact_index`, `hnsw_filtered`, `label_checked`, `unfiltered`) y si fue exacta (`vector_exact`). Ver [HYBRID_SEARCH.md § How the vector path applies the filter](../HYBRID_SEARCH.md#how-the-vector-path-applies-the-filter).

Lo comprueban:
- `nql_pattern_expansion_test::one_hop_distinct_then_limit` y `single_node_distinct_then_limit` (`limit` después de `distinct`);
- `p2_order_by_non_projected_test` (`order by` antes de `limit`);
- `nql_two_hops_test::hybrid_two_hops_equals_search_hybrid_plus_expansion` (el orden de semillas y los scores viajan con las filas).

## 4. Un patrón, un significado

La dirección, el tipo de arista, las propiedades inline y la multiplicidad significan lo mismo en todos los sitios donde aparece un patrón: lectura y escritura, uno o varios saltos, con o sin búsqueda.

- `->` sigue las aristas salientes, `<-` las entrantes y `<->` ambas. `(a)-[:T]->(b)` y `(b)<-[:T]-(a)` describen las mismas aristas y dan las mismas filas.
- `-[:T {w: 1}]->` solo deja las aristas cuyo `w` vale 1. `(b {x: 2})` solo deja los nodos cuyo `x` vale 2.
- `delete` y `update` con patrón cambian exactamente las aristas (o sus extremos) que devuelve `find` con el mismo patrón. Admiten un salto. Un patrón más largo o un cuantificador es un error.

Lo comprueban:
- `nql_pattern_properties_test::direction_is_symmetric` y `delete_with_a_pattern_deletes_what_find_returns`;
- `nql_pattern_expansion_test::one_hop_honours_the_direction`, `two_hops_honour_inline_edge_properties` y `write_patterns_honour_direction_and_inline_properties`.

## 5. Contrato de corrección: lo que toda optimización preserva

a. **Seguridad del filtro.** Toda fila cumple el `where` entero, y ningún resultado de una búsqueda queda fuera de su universo, con cualquier selectividad.
b. **Invariancia del empuje y de la poda.** Adelantar un predicado cambia el coste, nunca las filas ni su multiplicidad. Eso incluye empujarlo al universo de una búsqueda (restricciones de semilla) y aplicarlo al expandir (restricciones de expansión).
c. **Exactitud cuando se promete.** Una rama que se declara exacta devuelve el mismo top-k que la fuerza bruta sobre el universo. Una rama aproximada lo dice.
d. **Simetría de la dirección.** Ver el contrato 4.
e. **Nada en silencio.** Lo que NQL no evalúa es un error con nombre, nunca una condición que pasa o falla sin avisar. Eso incluye la aritmética `+ - * / %`, la negación `!` / `not`, el menos sobre una expresión, una búsqueda bajo `or` y un patrón de escritura de más de un salto. Hasta 0.8.2 esos operadores se descartaban (`x * 2 = 6` se ejecutaba como `x = 6`). Su soporte se sigue en [#269](https://github.com/sharop/nopaldb/issues/269).

   Desde 0.9.4 también: una propiedad, variable o literal suelto como condición de `where`/`having` es un error con nombre (escribe `p.activo = true`, no `p.activo`); un alias de la proyección en `where`/`having` vale por su expresión (`find degree(n) as dg … where dg > 1`), salvo que se llame como una variable del patrón; un extremo anónimo nunca toma un nombre del patrón; una variable repetida tiene que ser el mismo nodo (`(a)-[:F]->(b)-[:F]->(a)` son los ciclos de 2, `(a)-[:F]->(a)` los self-loops); una opción desconocida de `export` es un error que lista las válidas; `sketch` / `commit` por `execute_statement` son un error. Hasta 0.9.3 cada uno daba filas equivocadas, o una respuesta OK, sin error.

   Desde 0.10.3 también: una función que NQL no conoce es un error con nombre en toda cláusula, incluidos `delete`/`update` y `explain` (`id(n)` remite a `n.id`), y los nombres de las funciones conocidas no distinguen mayúsculas (`COUNT(*)` es `count(*)`). Hasta 0.10.2 una función desconocida, o una conocida escrita con otras mayúsculas, se descartaba: su columna desaparecía, `where foo(n)` dejaba pasar todas las filas, `order by foo(n)` no hacía nada. El valor de `update … set var.prop = <valor>` debe ser un literal (#330): hasta 0.10.2 `set n.x = n.nombre` respondía OK y no actualizaba nada.
f. **El viaje en el tiempo lee un grafo pasado.** `find … at timestamp <ts>` (desde 0.10.2, #298) responde lo que la misma consulta sin la cláusula habría respondido sobre el grafo válido en `ts`: cada nodo y arista en la versión válida entonces, nada de lo creado después, lo borrado todavía presente. En el último commit, o después, es el estado actual. Cuando el grafo en `ts` no se conoce entero es `HistoryUnavailable`, nunca una respuesta parcial: `ts` bajo el horizonte del GC, o bajo el suelo de historia de datos escritos sin versión antes de 0.10.1. Las funciones que leen datos sin historia (`similar_to`, `hybrid`, `in_community`, los scores de búsqueda, `has_embedding` y las funciones de embeddings de caminos y patrones) son un error con nombre con `at timestamp`.

Lo comprueban:
- (a) `nql_where_candidates_test::residual_predicates_still_filter_and_are_never_widened`, `candidate_set_test::results_lie_in_the_set_and_fill_k_when_the_set_allows` (test de propiedades), `hybrid_selective_filter_test`;
- (b) `nql_pattern_properties_test::two_hops_with_predicates_match_brute_force` y `partitioning_the_where_adds_up` (las filas sin filtro son las de `P` más las de su complemento, el oráculo de partición de consultas), `nql_pattern_expansion_test::explain_names_the_expansion_filters`;
- (c) `knn_exact_test::exact_matches_brute_force_reference`, `nql_where_candidates_test::similar_to_returns_the_exact_top_k_inside_the_tenant` y el test unitario `graph::hybrid::tests::allowed_set_over_1024_small_for_its_index_ranks_exactly`;
- (d) el contrato 4;
- (e) `nql_parens_and_literals_test::what_is_not_evaluated_yet_is_an_error_not_a_wrong_result`, `nql_where_candidates_test::a_search_must_be_a_root_conjunct`, `nql_unknown_functions_test::an_unknown_function_is_a_named_error_in_every_clause`, `nql_unknown_functions_test::writes_with_an_unknown_function_are_errors_and_change_nothing`, `nql_unknown_functions_test::known_functions_are_matched_without_case`, `nql_update_set_values_test::a_value_that_is_not_a_literal_is_an_error_and_writes_nothing`;
- (f) `mvcc_stamp_model_proptest` (un patrón de nodos y uno de aristas en cada timestamp contra un modelo de fuerza bruta, también tras reabrir), `nql_at_timestamp_test::en_el_ultimo_commit_es_el_presente`, `nql_at_timestamp_test::bajo_el_horizonte_del_gc`, `nql_at_timestamp_test::bajo_el_suelo_de_una_base_adoptada`, `nql_at_timestamp_test::funciones_sin_historia_rechazadas`.

## Comparaciones y valores

Comportamiento actual, fijado por los tests:

- Una comparación sobre una propiedad que el nodo no tiene es **falsa**, también con `!=`. `r.p != 3` no devuelve los nodos sin `p`.
- `= null` no encuentra nada, ni siquiera los nodos sin la propiedad.
- `=` compara valores con su tipo: `3` (entero) no es `3.0` (decimal), y `"3"` no es `3`. `<`, `>`, `<=` y `>=` comparan enteros y decimales por su valor numérico, así que `3 >= 3.0` es verdadero. Alinear `=` con ese orden es parte de [#269](https://github.com/sharop/nopaldb/issues/269).
- Los números negativos son literales: `-5`, `-2.5`, en `where`, `set`, listas y argumentos de función.
- `and` liga más que `or`, y los paréntesis agrupan: `a and (b or c)`.

## Fuera del alcance actual

Todo esto da un error con nombre, nunca un resultado distinto:

- Varios patrones en el `from` junto con una búsqueda.
- Sembrar desde una búsqueda una cadena de más de dos saltos.
- Agregaciones en patrones de varios saltos.
- Patrones `OPTIONAL`.
- Aritmética y negación en expresiones (#269).
- Con `at timestamp`: las búsquedas vectoriales y de texto completo, las comunidades guardadas y los embeddings, que no tienen historia (contrato 5f, [referencia NQL § Viaje en el tiempo](NQL_REFERENCIA.md#viaje-en-el-tiempo-at-timestamp)).
