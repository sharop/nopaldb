# Tutorial Avanzado de NopalDB

Un recorrido **modular** por las capacidades avanzadas de NopalDB en cuatro actos. Cada acto resuelve un problema diferente y exhibe un subconjunto distinto de features. Cada lección se entrega en cuatro medios paralelos:

1. **Markdown narrativo** (este árbol `docs/tutorial/`)
2. **NDBStudio Web** — walkthrough con queries interactivas y visualización de grafo
3. **Jupyter Notebook** (`tutorials/notebooks/`)
4. **Ejemplo Rust annotado** (`nopaldb/examples/tutorial_acto_*.rs`)

---

## Actos

| Acto | Tiempo | Dataset | Features que protagoniza |
|------|--------|---------|--------------------------|
| [0 — Setup](acto_0_setup/README.md) | ~20 min | (ninguno) | Instalación, smoke test, NDBStudio Web |
| [1 — Florentine Families](acto_1_florentine/README.md) | ~30 min | 16 familias renacentinas | Modelo de datos, NQL pattern matching, `degree`/`pagerank`/`betweenness`/`clustering`, `community()` (Louvain) vs `leiden()`, focus mode en NDBStudio |
| [2 — Synthetic Offshore Network](acto_2_synthetic_offshore/README.md) | ~50 min | Fixture TTL real | Importer Turtle, índices (hash/btree/fulltext), embeddings con `sentence-transformers`, HNSW (`similar_to`, `knn_nodes`), path queries cuantificadas, reducers, mini-VM, E-8/E-9/E-10 (PathSimilarity / PathKNN / PathAnomaly) |
| [3 — Biomedical OWL + SHACL](acto_3_biomedical_owl/README.md) | ~40 min | Ontología biomédica compacta | OWL Turtle, `instanceOf`/`subClassOf`, ELReasoner CR1+CR2+CR3, SHACL validation, combinación reasoner + embeddings |
| [4 — Synthetic Fraud (Final Boss)](acto_4_synthetic_fraud/README.md) | ~40 min | ~5K nodos sintéticos con seed | Todas combinadas: MVCC `AT TIMESTAMP`, jerarquía de clases por reasoner, embeddings de descripciones, `path_anomaly_score` (E-10), community detection sobre el ring, Arrow export |

**Tiempo total estimado:** 3-3.5 horas. Los actos son independientes salvo cuando
el propio README de un acto indique datos generados por un acto anterior.

---

## Mapa de features → acto

| Capability | Aparece en |
|------------|-----------|
| NQL pattern matching | 1, 2, 3, 4 |
| Algoritmos de centralidad (`pagerank`, `betweenness`, `clustering`, `degree`) | 1 |
| Community detection (`community`, `leiden`) | 1, 4 |
| Importer TTL/Turtle | 2, 3 |
| Índices Hash / B-Tree / Full-Text | 2 |
| Embeddings + HNSW (`similar_to`, `knn_nodes`, `embedding_similarity`) | 2, 4 |
| Path queries cuantificadas (`-[:R]->{n,m}`) | 2, 4 |
| Path reducers (`path_sum`, `path_max`, `path_avg`) | 2 |
| Path metadata (`path.depth`, `path.nodes`, `path.edges`) | 2 |
| Mini-VM (`gather`/`return`) | 2 |
| Path Reference Embeddings (E-8 PathSimilarity, E-9 PathKNN, E-10 PathAnomaly) | 2, 4 |
| ELReasoner OWL (`instanceOf`, `subClassOf`) | 3, 4 |
| SHACL validation | 3 |
| MVCC time-travel (`AT TIMESTAMP`) | 4 |
| Arrow export para ML | 4 |

---

## Contrato de los 4 medios (anti-drift)

Para evitar que las queries diverjan entre Markdown / Notebook / Rust / NDBStudio, **cada query NQL vive una sola vez** en `docs/tutorial/<acto>/queries/*.nql`. Los 4 medios la leen del mismo archivo:

- **Markdown narrativo**: bloques con tag `<!-- source: queries/03_centralidad.nql -->` que muestran el contenido del archivo.
- **Notebook Python**:
  ```python
  from shared import load_nql
  nql = load_nql("acto_1", "03_centralidad.nql")
  result = graph.execute_nql(nql)
  ```
- **Rust**:
  ```rust
  let nql = include_str!(
      "../../docs/tutorial/acto_1_florentine/queries/03_centralidad.nql"
  );
  ```
- **NDBStudio Web**: copy/paste manual desde el archivo (paso documentado en cada `ndbstudio_walkthrough.md`).

Si una query debe cambiar, cambia en un único archivo `.nql` y los cuatro medios actualizan automáticamente (excepto NDBStudio que requiere re-pegar).

---

## Cómo ejecutar (tl;dr)

```bash
# Una sola vez:
cd /path/to/nopaldb
make build-wheel                             # genera wheel con python-full
pip install dist/wheels/nopaldb-*.whl        # instala binding
cd tutorials && pip install -r requirements.txt

# Por acto (Florentine como ejemplo):
cd tutorials
python -m jupyter notebook notebooks/01_florentine_families.ipynb
# en otra terminal:
make studio-florentine                        # NDBStudio Web en :3737
# o ejemplo Rust:
cd .. && cargo run --example tutorial_acto_1_florentine
```

Detalles en [Acto 0 — Setup](acto_0_setup/README.md).

---

## Gate de validación cruzada

Cada acto define una **query canónica** que debe producir el mismo resultado en los 4 medios. Por ejemplo, en Acto 1: el top-3 de PageRank debe coincidir entre Notebook, Rust y NDBStudio. Esto se documenta como checklist al final de cada `<acto>/README.md`.

Si un día los resultados divergen → algo se rompió en el camino (cambio de API, drift de versión). El gate es la primera defensa.

---

## Versión validada

Actos 1-4 probados contra **NopalDB v0.4.27**. Si tu versión es distinta, algunas queries pueden requerir ajustes; revisa el changelog del repo.
