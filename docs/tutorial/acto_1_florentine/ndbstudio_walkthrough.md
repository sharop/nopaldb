# Acto 1 — Walkthrough en NDBStudio Web

Esta guía recorre las cuatro queries del Acto 1 dentro de NDBStudio Web, aprovechando la visualización interactiva (Graph view, focus mode, timeline). Asume que ya generaste la DB siguiendo [README.md](README.md).

---

## Levantar NDBStudio Web

Desde la raíz del repo:

```bash
make run-studio-web DB=test_dbs/florentine_families.db
```

O desde `tutorials/`:

```bash
make studio-florentine
```

Abre `http://127.0.0.1:3737` en tu navegador.

---

## Layout que vas a usar

- **Editor** (arriba-izq): pega NQL.
- **Results** (abajo-izq): cicla entre `Table | JSON | Graph | Plan`.
- **Side panel derecho**: `Schema` o `Graph` (toggle con `s` / `x`).
- **Timeline** (abajo): historial de queries.

---

## Paso 1 — Modelo

Pega en el editor el contenido de [`queries/01_modelo.nql`](queries/01_modelo.nql) y ejecuta. Cambia a la vista `Schema` (panel derecho) y verifica que aparece la etiqueta `Family` con propiedades `name`, `wealth_rank`, `faction`.

**Qué buscas:** tabla con 15 filas. Si ves menos, la DB no se generó completa.

---

## Paso 2 — Pattern matching y vista Graph

Pega [`queries/02_pattern_matching.nql`](queries/02_pattern_matching.nql) y ejecuta.

Para visualizar el grafo, **cambia la query** a una variante que retorne nodos en lugar de strings:

```sql
find a, b
from (a:Family) -[:MARRIAGE]-> (b:Family)
where a.name = "Medici"
```

Luego cicla `Results → Graph`. Verás Medici en el centro con sus aliadas como satélites.

**Focus mode**: clickea Medici → "Focus on this node" → el graph view se centra y oculta nodos no conectados a >1 hop. Útil para entender vecindarios.

---

## Paso 3 — Centralidad y resultados como tabla numérica

Pega [`queries/03_centralidad.nql`](queries/03_centralidad.nql) y ejecuta.

En la vista `Table`, ordena clickando los headers. Compara:
- **PageRank** vs **Betweenness**: ambas ponen Medici primero. Confirmación visual del *broker effect*.
- **Clustering coefficient** de Medici: bajo. Sus aliadas no se conocen entre sí.
- **Degree** de Strozzi: alto (cluster denso). Pero su PageRank es menor que Medici → estar en un cluster denso no es lo mismo que ser puente.

> Si quieres ver el grafo coloreado por PageRank, proyecta nodos con la métrica como atributo y usa la vista Graph con coloring por valor numérico.

---

## Paso 4 — Communities

Pega [`queries/04_communities.nql`](queries/04_communities.nql) y ejecuta.

Verás 2 columnas: `louvain` y `leiden`. **Compara visualmente las particiones**:

1. Sort por `louvain`.
2. Anota qué familias quedan agrupadas.
3. Sort por `leiden`.
4. Compara.

Si ambos algoritmos coinciden → la estructura comunitaria es robusta. Si difieren → hay nodos "frontera" que cada algoritmo asigna distinto. En esta red pequeña suelen coincidir excepto en 1-2 nodos.

**Verificación con la facción histórica:** la columna `faccion_historica` (Medici/Albizzi) debería **correlacionar fuertemente** con `louvain` o `leiden`. No idénticamente — los algoritmos descubren la estructura *latente*, no leen el atributo.

---

## Lo que NDBStudio aporta sobre el notebook

- **Graph view interactiva** con focus mode (los notebooks usan networkx + matplotlib estático).
- **Timeline**: ver el orden de queries y re-ejecutarlas en un click.
- **Schema panel**: ver labels, propiedades y conteos sin ejecutar query.
- **Lineage DAG** (cuando uses sketches/commits — no en este acto).

Lo que el notebook aporta sobre NDBStudio:
- Análisis pos-query con pandas (filtrar, joinar con datos externos).
- Reproducibilidad (mismo input → mismo output → versionable).

Por eso usamos los dos.

---

## Verificación

Ya cubierta en [README.md → Verificación cruzada](README.md#verificación-cruzada-gate-del-acto-1). El gate es: **mismo top-3 de PageRank** en los 4 medios.
