# Acto 2 — Walkthrough NDBStudio Web

Esta guía recorre los 6 pasos del Acto 2 dentro de NDBStudio Web. Asume que ya generaste la DB y, **opcionalmente**, que ya corriste el notebook al menos una vez para que los embeddings queden persistidos en la DB (necesarios para los pasos 3 y 6).

## Levantar

```bash
# desde tutorials/
make studio-offshore

# o desde la raíz:
make run-studio-web DB=test_dbs/synthetic_offshore.db
```

Abre `http://127.0.0.1:3737`.

---

## Paso 1 — Schema discovery

Pega [`queries/01_schema_discovery.nql`](queries/01_schema_discovery.nql). En la vista `Schema` (panel derecho) compara contra los conteos: deben coincidir.

## Paso 2 — Índice y plan de query

Pega [`queries/02_indices.nql`](queries/02_indices.nql). Cambia a la vista `Plan` para ver que el query planner usa **IndexScan** sobre el hash index `OffshoreEntity_name_hash`. Compara contra:

```sql
find e.name from (e:OffshoreEntity) where e.industry = "private banking"
```

Esa otra query no tiene índice sobre `industry` — verás `FullScan`. Es la forma visual de entender por qué los índices importan.

## Paso 3 — HNSW similarity

Pega [`queries/03_hnsw.nql`](queries/03_hnsw.nql).

> **Importante:** si la DB no tiene embeddings persistidos, esta query va a fallar o retornar 0 filas. Corre antes el notebook (`02_synthetic_offshore.ipynb`) hasta el final — los embeddings quedan persistidos en la DB y NDBStudio los puede leer.

En la vista `Table` el top-1 debe ser `Atlas Fiduciary Holdings`. En la vista `Graph` los nodos retornados aparecen — usa el side panel `Graph` para ver el subgrafo focalizado.

## Paso 4 — Path query cuantificada

Pega [`queries/04_paths_quantifier.nql`](queries/04_paths_quantifier.nql). En la vista `Table` ordena por `hops` para ver la pirámide en niveles. En la vista `Graph` se renderiza el árbol de control corporativo enraizado en la entidad insignia — es un buen visual del payload del Acto.

## Paso 5 — Path reducer

Pega [`queries/05_path_minivm.nql`](queries/05_path_minivm.nql). El top-3 por `flujo_total` muestra que los caminos más profundos pueden cargar más flujo total que los directos. Útil para identificar exposiciones financieras agregadas que no se ven mirando edges individuales.

## Paso 6 — Path anomaly (E-10)

Pega [`queries/06_path_anomaly.nql`](queries/06_path_anomaly.nql).

> **Importante:** requiere `add_path_reference_embedding(...)` previo en Python (el notebook lo hace). Sin baselines, todos los scores son `1.0` y la query pierde sentido.

---

## Lo que NDBStudio aporta sobre el notebook

- `Plan` view del query planner (FullScan vs IndexScan).
- Visualización del subgrafo de `CONTROLS` con focus desde el flagship.
- Cambio en vivo entre vistas Table/JSON/Graph sin re-ejecutar la query.

## Verificación

Ya cubierta en [README → gate](README.md#verificación-cruzada-gate-del-acto-2). Top-1 de Q3 debe ser `Atlas Fiduciary Holdings`.
