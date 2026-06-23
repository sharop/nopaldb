# Acto 1 — Florentine Families

**Tiempo estimado:** 30 min
**Dataset:** 15 familias renacentinas + 20 lazos de matrimonio (red clásica de Padgett & Ansell, 1993).
**DB:** `test_dbs/florentine_families.db`

## Por qué este dataset (y por qué solo Acto 1)

La red de matrimonios florentinos es el ejemplo canónico para enseñar **centralidad y detección de comunidades** en grafos. Es lo bastante pequeña para razonar a mano (15 familias) pero tiene una estructura no-trivial: la familia Medici domina por *posición estructural*, no por riqueza, y el dataset lo demuestra contundentemente.

Sus límites también son útiles: 15 nodos no alcanza para HNSW, embeddings significativos ni reasoner OWL. Por eso este acto se enfoca en lo que sí luce con poco dato — **algoritmos de centralidad y community detection** — y los actos siguientes escalan a datasets mayores.

## Qué vas a hacer

1. Generar la DB.
2. Validar el modelo (15 familias, 20 lazos × 2 direcciones).
3. Hacer pattern matching directo (vecinas de Medici).
4. Calcular 4 métricas de centralidad simultáneamente.
5. Comparar Louvain (`community()`) vs Leiden (`leiden()`) sobre el mismo grafo.

## Setup

```bash
# Desde la raíz del repo
python nopaldb/examples/florentine_families_dataset.py \
  --db test_dbs/florentine_families.db --reset
```

Esto crea ~15 nodos `:Family` con `name`, `wealth_rank`, `faction`, y ~40 edges `:MARRIAGE` (cada lazo se almacena en ambas direcciones para que el pattern matching dirigido funcione natural).

---

## Paso 1: validar el modelo

<!-- source: queries/01_modelo.nql -->
```sql
find f.name, f.wealth_rank, f.faction
from (f:Family)
order by f.name
```

**Qué hace:** lista las 15 familias con su rango de riqueza histórico y su facción política (Medici / Albizzi).

**Qué obtienes:** una tabla ordenada alfabéticamente. Si esto retorna 15 filas, el dataset cargó bien. Si menos → revisa el output del generador.

**Por qué primero:** validar el modelo antes de algoritmos elimina una clase entera de bugs ("¿el algoritmo está mal o el dataset?").

---

## Paso 2: pattern matching

<!-- source: queries/02_pattern_matching.nql -->
```sql
find a.name as origen, b.name as aliada, b.faction as faccion_aliada
from (a:Family) -[:MARRIAGE]-> (b:Family)
where a.name = "Medici"
order by b.name
```

**Qué hace:** encuentra todas las familias conectadas directamente a la Medici por matrimonio.

**Qué obtienes:** ~6 filas (Acciaiuoli, Albizzi, Barbadori, Ridolfi, Salviati, Tornabuoni). Nota que **una de las aliadas pertenece a la facción Albizzi** (la rival): los matrimonios cruzaban facciones — el grafo no es solo "su facción".

**Por qué importa:** el pattern matching `(a) -[:R]-> (b)` es la primitiva de NQL. Todo lo demás (paths, agregaciones, joins) compone sobre esto. Si entiendes esta query, entiendes 80% de NQL.

---

## Paso 3: centralidad

<!-- source: queries/03_centralidad.nql -->
```sql
find f.name,
     pagerank(f) as pr,
     betweenness(f) as btw,
     clustering(f) as cc,
     degree(f) as deg
from (f:Family)
order by pr desc
limit 10
```

**Qué hace:** calcula cuatro métricas de importancia estructural simultáneamente, una columna por algoritmo.

**Qué obtienes:** una tabla con Medici en la cima de PageRank y Betweenness. Su clustering coefficient es **bajo** comparado con su grado — eso significa que sus aliadas no están conectadas entre sí: Medici es un *broker*, no un núcleo de un cluster denso.

**El insight central:** Medici no era la familia más rica (rank 1 en wealth, pero Strozzi era #2 con red más densa entre los Albizzi). Lo que la hace dominante es su *posición*: conecta facciones que no están conectadas entre sí. PageRank y Betweenness lo capturan. Wealth_rank no.

> Este es el resultado clásico de Padgett & Ansell (1993): el poder de los Medici emerge de la topología.

**Por qué cuatro métricas a la vez:** una sola métrica engaña. Degree alto sin clustering bajo = miembro de un núcleo denso (no es lo mismo que ser broker). PageRank sin betweenness = importancia por asociación, no por puente. Verlas juntas es la manera correcta.

---

## Paso 4: community detection — Louvain vs Leiden

<!-- source: queries/04_communities.nql -->
```sql
find f.name,
     f.faction as faccion_historica,
     community(f) as louvain,
     leiden(f) as leiden
from (f:Family)
order by louvain, leiden, f.name
```

**Qué hace:** asigna cada familia a una comunidad detectada por dos algoritmos distintos. **Sin** mirar el atributo `faction` — solo la topología.

**Qué obtienes (resultado real, v0.4.21):**

```
Louvain — 5 comunidades:
  Guadagni, Lamberteschi                                   <- corredor Albizzi-Medici
  Barbadori, Bischeri, Castellani, Peruzzi, Strozzi        <- bloque Strozzi-sur
  Albizzi, Ginori                                          <- Albizzi norte (rama Ginori)
  Acciaiuoli, Medici, Ridolfi, Tornabuoni                  <- bloque Medici
  Pazzi, Salviati                                          <- par periférico

Leiden (gamma=0.1, CPM) — 5 comunidades, distinta de Louvain en Barbadori:
  Pazzi, Salviati                                          <- par periférico
  Guadagni, Lamberteschi                                   <- corredor Albizzi-Medici
  Acciaiuoli, Barbadori, Medici, Ridolfi, Tornabuoni       <- bloque Medici (con Barbadori)
  Albizzi, Ginori                                          <- Albizzi norte (rama Ginori)
  Bischeri, Castellani, Peruzzi, Strozzi                   <- bloque Strozzi-sur
```

**Lo que muestra este resultado:**

- **Barbadori: el broker que diferencia a Louvain de Leiden.** Barbadori tiene exactamente 1 arista hacia Medici y 1 hacia Castellani — es genuinamente equidistante. Louvain lo coloca con el bloque Strozzi; Leiden lo coloca con el bloque Medici. Leiden tiene razón históricamente: `faction=Medici` en el dataset. Esta diferencia no es ruido — es Leiden encontrando una partición de mayor calidad CPM que refleja mejor la estructura de alianzas reales.
- **Albizzi se divide en dos clusters:** Albizzi+Ginori (norte, hoja en el grafo) queda separado de Guadagni+Lamberteschi (corredor con más conexiones transfacción). El grafo detecta una partición que las facciones históricas no documentan explícitamente.
- **Pazzi+Salviati, par aislado:** Pazzi conecta solo a Salviati; Salviati conecta a Pazzi y Medici. Con CPM o modularidad, el par queda en comunidad propia.
- **¿Por qué el fix de v0.4.20 importó?** La versión anterior de Louvain colapsaba el grafo en 1 sola comunidad porque la fórmula de ganancia era incompleta (no restaba el costo de salir de la comunidad actual). La fórmula neta correcta (Blondel et al. 2008) produce esta partición coherente.

**Lección:** ambos algoritmos dan 5 comunidades, pero **no la misma partición**. La diferencia está en Barbadori, un broker equidistante donde la función objetivo CPM de Leiden supera a la modularidad de Louvain para encontrar la asignación históricamente correcta. Esto es el "wow moment" del acto: los datos topológicos detectan la facción real sin mirar el atributo `faction`.

**Louvain (`community()`):** maximiza modularidad clásica (Blondel et al. 2008).
**Leiden (`leiden()`):** garantiza comunidades internamente "well-connected" y usa CPM con gamma=0.1 (Traag et al. 2019). Produce particiones de mayor calidad en nodos broker.

---

## Verificación cruzada (gate del Acto 1)

Los **4 medios** deben producir el mismo top-3 de PageRank. Estos son los nombres exactos esperados (ordenados por PageRank descendente):

```
1. Medici      pr ≈ 0.146
2. Guadagni    pr ≈ 0.098
3. Strozzi     pr ≈ 0.088
```

> **Nota:** Guadagni queda #2, no Albizzi. Es contraintuitivo si solo miras la facción histórica — Guadagni es Albizzi pero su posición es más central que la de la familia Albizzi misma. La centralidad mide topología, no nombres.

| Medio | Comando |
|-------|---------|
| Markdown | leer `queries/03_centralidad.nql` |
| Notebook | `tutorials/notebooks/01_florentine_families.ipynb`, celda "centralidad" |
| Rust | `cargo run --example tutorial_acto_1_florentine` |
| NDBStudio Web | `make studio-florentine` desde `tutorials/`, luego pegar `queries/03_centralidad.nql` |

Si los 4 coinciden en top-3 → Acto 1 OK.

---

## Walkthrough en NDBStudio

Ver [ndbstudio_walkthrough.md](ndbstudio_walkthrough.md) para el recorrido visual paso-a-paso (focus mode, timeline, results graph view).

## Siguiente

[Acto 2 — Synthetic Offshore Network](../acto_2_synthetic_offshore/README.md): embeddings reales, HNSW, path queries con anomalía.
