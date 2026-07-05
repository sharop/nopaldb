# Roadmap: REPL + Workbench para NDBStudio

Estado actual (Feb 2026):

- Fase 0: completada
- Fase 1: completada
- Fase 2: completada
- Fase 3: completada
- Fase 4: en progreso (QueryGraph + lineage + re-run dependents)

## Fase 0: Preparacion (1-2 dias)

1. Definir modelos base en `app/session`:
   - `SessionState`
   - `TimelineEntry`
   - `SavedQuery`
   - `QueryTab`
2. Agregar feature flag: `ndbstudio.session_v2`.
3. Acordar formatos de persistencia (`~/.ndstudio/session.json`).

Entregable:

- Modelos compilando + tests unitarios iniciales.

## Fase 1: MVP usabilidad (4-6 dias)

1. Timeline visible en panel inferior (toggle).
2. Re-run sobre entradas timeline.
3. Guardar query a snippets/favoritos.
4. Multi-tab editor:
   - crear/cerrar/cambiar tab
   - cada tab conserva query y ultimo resultado
5. Parametros por sesion:
   - parser simple `$param`
   - panel/command para bind values

Entregable:

- Flujo "REPL persistente" funcional sin DAG.

## Fase 2: Explain/Profile + trazabilidad fuerte (3-4 dias)

1. Acciones `Run / Explain / Profile` por tab.
2. Guardar plan/perfil en timeline.
3. Mejoras de errores:
   - variable no definida
   - hints contextuales.

Entregable:

- Workbench base con telemetria local por ejecucion.

## Fase 3: Cache in-memory (3-5 dias)

1. Implementar `query_hash`:
   - normalized query
   - params serializados
   - db_revision/schema_revision
2. Cache LRU por resultado.
3. Invalidacion tras writes.
4. Indicador UI: `cache hit` / `cache miss`.

Progreso actual:

- `query_hash` activo con modo+revisiones+params.
- Cache LRU en memoria activa.
- Invalidador fino por write `data` vs `schema` (sin clear total por defecto).
- Badge `cache hit/miss` visible en Results y Timeline/Session Browser.
- Hit-rate por sesion/tab y ventana reciente (`:cache recent ...`).

Entregable:

- Re-runs rapidos en consultas repetidas.

## Fase 4: QueryGraph (DAG) incremental (5-8 dias)

1. Construir `QueryGraph` con dependencias.
2. Re-ejecucion selectiva por cambios.
3. Vista simplificada de linaje en timeline.

Entregable:

- Primer motor incremental operativo.

Progreso actual:

- QueryGraph persistente por run (nodes/edges + reason).
- `:timeline lineage <n>` y `:timeline dag <n>`.
- `:timeline rerun dependents <n>` con prioridad por impacto semántico.
- Dependencias semánticas por labels, edge types y properties.
- `:timeline impact <n>` y `:timeline rerun impacted <n> --threshold`.

## Fase 5: Persistencia y colaboracion local (3-5 dias)

1. Persistir session state, tabs y timeline.
2. Recuperacion de sesion al abrir NDBStudio.
3. Exportar session report (`json/md`).

Entregable:

- Workbench de analisis reproducible entre sesiones.

## Backlog tecnico (post-v1)

1. Snapshot diff entre ejecuciones.
2. Cache en disco con cuotas.
3. Transacciones interactivas (`begin`, `commit`, `rollback`) desde REPL.
4. Plantillas de analisis por dominio (fraude, social network, supply chain).

## Dependencias clave

1. Normalizador de queries NQL (para hash estable).
2. Exponer `db_revision` y `schema_revision` desde engine.
3. Contratos de resultado unificados entre modos (`table/json/graph`).

## Criterios Go/No-Go por fase

1. Sin regresion de UX actual (scroll, panels, commands).
2. `cargo test -p ndbstudio` estable.
3. Tiempos de render UI sin bloqueos percibidos.
