# ADR: Modelo Hibrido REPL + Workbench para NDBStudio

- Estado: Propuesto
- Fecha: 2026-02-17
- Owner: NDBStudio

## Contexto

Hoy NDBStudio permite ejecutar queries, ver resultados y navegar paneles, pero la iteracion larga es costosa:

1. Escribir -> ejecutar -> borrar -> reescribir.
2. Poco soporte para reuso de queries/snippets.
3. Sin contexto de sesion persistente (params, ultimos resultados, trazas).

La meta es evolucionar hacia:

- Sistema declarativo.
- Grafo explicito de ejecucion.
- Ejecucion incremental.
- Cache.
- Trazabilidad completa.

## Decision

Adoptar un modelo hibrido:

1. Workbench (multi-tab editor, resultados multi-vista, explain/profile).
2. REPL persistente (historial ejecutable, contexto de sesion, re-run rapido).
3. Session Engine comun para ambos.

## Arquitectura objetivo

## 1) SessionState

Estado in-memory (persistible):

- session_id
- active_tab_id
- tabs: query_text, params, mode
- timeline: lista de ejecuciones
- snippets/saved_queries
- last_result_ref por tab
- transaction_state (future: explicit tx)

## 2) QueryGraph (DAG)

Cada ejecucion produce un nodo:

- query_hash = hash(normalized_query + bound_params + db_revision)
- parent_refs (si deriva de otra query/snippet)
- result_ref
- plan_ref (opcional)

Uso:

- Re-run incremental.
- Detectar resultados reutilizables.
- Trazar linaje de analisis.

## 3) ResultCache

- Nivel 1 (MVP): in-memory LRU por query_hash.
- Nivel 2: persistencia opcional a disco.
- Invalidation: por db_revision, schema_revision y TTL.

## 4) TraceLog

Por ejecucion:

- timestamp
- query (raw + normalized)
- params
- duration_ms
- row_count/summary
- error_chain
- mode_result (table/json/graph)

## Alcance inicial (MVP)

Sin romper flujo actual:

1. Timeline ejecutable.
2. Saved queries/snippets.
3. Multi-tab editor.
4. Params de sesion.
5. Explain/Profile integrados en misma query.

No incluye en MVP:

1. DAG incremental completo.
2. Cache persistente en disco.
3. Transaction console avanzada.

## Invariantes tecnicas

1. UI nunca se bloquea por ejecucion de query.
2. Toda ejecucion queda trazada.
3. Cache invalida correctamente en writes.
4. Estado restaurable al reiniciar (fase 2).

## Riesgos

1. Complejidad de estado (tabs + timeline + paneles).
2. Falsos hits de cache por hashing incompleto.
3. Degradacion de memoria por resultados grandes.

Mitigaciones:

1. Limites de cache (filas/bytes/TTL).
2. Hash con db_revision y params normalizados.
3. Feature flags por etapa.

## Metricas de exito

1. Tiempo medio de iteracion (query edit-run-rerun) -30%.
2. Reutilizacion de queries desde timeline/snippets >40%.
3. Errores de contexto ("que query corri?") reducidos en soporte.

