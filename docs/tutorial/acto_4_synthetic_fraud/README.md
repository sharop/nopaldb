# Acto 4 — Synthetic Fraud (Final Boss)

**Tiempo estimado:** 40 min
**Dataset:** ~570 nodos sintéticos generados deterministically (seed=42).
**DB:** `test_dbs/synthetic_fraud.db`

## Por qué este acto

Los actos 1-3 cubrieron features aisladas. El Acto 4 las **combina** sobre un escenario realista: detección de un anillo de lavado de dinero plantado dentro de un grafo más grande de actividad financiera normal.

**El reto:** dadas 312 cuentas con ~400 transferencias, encontrar las 5 que forman un ciclo cerrado. Sin etiquetas, sin marcar el ring como tal — solo via señales topológicas y comportamentales.

**Las features que se combinan:**
- Topología (`group by n.label`, `count(*)`).
- Densidad inbound (el indicador más simple y robusto).
- Path queries con cuantificadores (cadenas de control corporativo).
- Embeddings sobre descripciones textuales (Person.description).
- HNSW similarity search (`similar_to`).
- Arrow export — para que un pipeline de ML (sklearn, XGBoost, GNN) consuma el resultado columnar.

## Topología

```
Person  ─OWNS────────►  Account  ─TRANSFERS{amount, timestamp}─►  Account
   │
   └─DIRECTS─►  LegalEntity / ShellCompany  ─CONTROLS{amount}─►  ...
```

**Conteos esperados (seed=42):**
- 200 Person, 312 Account, 50 LegalEntity, 10 ShellCompany.
- 312 OWNS, ~99 DIRECTS, ~20 CONTROLS, 400 TRANSFERS.

**El ring plantado:**
- 5 Persons (los primeros del generador) → 5 Accounts (los primeros de cada uno).
- Entre los 5 Accounts: 30 transfers secuenciales (6 por par adyacente) + ~40 cross-transfers densos.
- **Cada cuenta del ring tiene exactamente 14 inbound transfers** (4 del predecesor + ~10 cross). Las cuentas no-ring tienen ≤4.

Esa diferencia es el **gate central del acto**.

## Setup

```bash
cd /path/to/nopaldb
python3 tutorials/shared/synthetic_fraud_dataset.py \
  --db test_dbs/synthetic_fraud.db --reset
```

---

## Paso 1 — Topología

<!-- source: queries/01_topology.nql -->
```sql
find n.label as etiqueta, count(*) as total
from (n)
group by n.label
order by total desc
```

**Qué obtienes:**
```
Account         312
Person          200
LegalEntity      50
ShellCompany     10
```

---

## Paso 2 — Detección por densidad inbound

<!-- source: queries/02_top_inbound.nql -->
```sql
find b.id, count(*) as inbound
from (a:Account) -[:TRANSFERS]-> (b:Account)
group by b.id
```

**Qué obtienes:** una fila por cuenta, ordenadas por insertion order. **El cliente** (notebook o ejemplo Rust) ordena descendiente y toma top-5.

> **Nota sobre v0.4.19:** `ORDER BY ... DESC LIMIT N` sobre agregaciones agrupadas no respeta el orden — los resultados vienen en orden arbitrario. Por eso los 4 medios ordenan client-side.

**El gate:** las 5 cuentas con `inbound >= 14` son las del ring. Todas las demás tienen `inbound < 14`.

---

## Paso 3 — Path queries cuantificadas

<!-- source: queries/05_path_chains.nql -->
```sql
find a.id as origen, b.id as destino, path.depth as hops, path_sum("amount") as flujo
from (a:Account) -[:TRANSFERS]-> {2,3} (b:Account)
order by flujo desc
limit 10
```

**Qué hace:** explora cadenas de 2-3 hops y agrega el monto total. Los caminos del ring tienen `flujo` mucho más alto que la media porque cada hop carga 900k+.

---

## Paso 4 — Embeddings + HNSW (notebook)

El notebook genera embeddings con `all-MiniLM-L6-v2` sobre `Person.description`, los adjunta vía `add_node_embedding` y consulta `similar_to`. La señal es débil porque las descripciones son genéricas, pero el pipeline funciona — cualquier texto rico (descripciones de transacciones, comentarios) lo haría más útil.

---

## Paso 5 — Arrow export

```python
arrow_bytes = graph.to_arrow()
table = pa.ipc.open_stream(io.BytesIO(arrow_bytes)).read_all()
df = table.to_pandas()
```

`to_arrow()` devuelve un IPC stream que pyarrow deserializa. Una vez en pandas, alimenta cualquier pipeline (sklearn, GNN con PyG, modelos de scoring).

---

## Verificación cruzada (gates del Acto 4)

**Rust** (`tutorial_acto_4_fraud_finalboss`):
- Top-5 cuentas inbound tienen min=14 transfers → ✅
- Topología: 200 Person, 312 Account, 50 LegalEntity, 10 ShellCompany → ✅

**Python notebook** (`04_synthetic_fraud.ipynb`):
- Exactamente 5 cuentas tienen `inbound >= 14`, todas con valor exacto 14 → ✅
- Topología consistente → ✅
- Arrow export deserializa correctamente → ✅

**NDBStudio Web:**
- `queries/02_top_inbound.nql` retorna 312 filas; ordenando client-side, top-5 = 14 inbound cada uno → ✅

## Walkthrough en NDBStudio

Ver [`ndbstudio_walkthrough.md`](ndbstudio_walkthrough.md).

## Cierre del tutorial

Has recorrido los 4 actos:
1. **Florentine** — algoritmos clásicos.
2. **Synthetic Offshore Network** — embeddings + HNSW + path queries.
3. **Biomedical OWL** — reasoner.
4. **Synthetic Fraud** — combinación final con detección estructural y export para ML.

Cada acto vivió en 4 medios paralelos sincronizados via NQL canónico. El patrón replicable es:
- queries `.nql` como fuente única,
- generador determinista en Python,
- ejemplo Rust para integración embebida,
- notebook para análisis interactivo,
- NDBStudio Web para visualización.
