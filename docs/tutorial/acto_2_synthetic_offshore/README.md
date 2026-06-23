# Acto 2 — Synthetic Offshore Network (sintético)

**Tiempo estimado:** 50 min
**Dataset:** ~140 nodos ficticios generados deterministamente (seed=42).
**DB:** `test_dbs/synthetic_offshore.db`

## Contexto: ¿por qué una red offshore sintética?

Este acto usa una red ficticia para practicar análisis de estructuras corporativas complejas sin depender de nombres, marcas ni datasets externos. El patrón que queremos estudiar es:

```
Persona real → controla → Empresa offshore → controla → Filial en paraíso fiscal
```

El dataset reproduce esa estructura con ~140 nodos: empresas offshore, directivos, jurisdicciones ficticias o genéricas, intermediarios y empresas pantalla. Es lo suficientemente grande para que HNSW y path queries tengan resultados interesantes, pero pequeño para razonar a mano sobre cada resultado.

## Topología generada

```
Officer  ─OWNS{shares}─►  OffshoreEntity  ─REGISTERED_IN─►  Jurisdiction
                               │           └─USES_INTERMEDIARY─►  Intermediary
                               │
                           CONTROLS{amount}
                               ▼
                           OffshoreEntity / ShellCompany (subsidiaria)
```

- **4** Jurisdictions (BVI, Harbor Cay, Seychelles, Luxembourg) con nivel de riesgo.
- **5** Intermediaries (Atlas Fiduciary Group, Alpha Services…) con `description` textual para embeddings.
- **~80** Entities (`OffshoreEntity` + `ShellCompany`) cada una con `description` rica.
- **~50** Officers con `country`. Cada entity tiene 1-3 owners cuyas `shares` suman 100%.
- **Pirámide CONTROLS** plantada deterministamente desde `Atlas Fiduciary Holdings` hacia 3 hijos y 6 nietos — garantiza que las path queries siempre retornen resultados reales.

## Setup

```bash
python tutorials/shared/synthetic_offshore_dataset.py \
  --db test_dbs/synthetic_offshore.db --reset
```

---

## Paso 1 — Schema discovery: ¿qué hay en este dataset?

**¿Qué es el "schema"?**
En cualquier dataset desconocido, la primera pregunta es: *¿qué tipos de entidades existen y cuántas hay de cada tipo?* Esto se llama "schema discovery". En una investigación de fraude real no recibes un manual — recibes datos en crudo y tienes que entender la forma antes de poder hacer preguntas útiles.

<!-- source: queries/01_schema_discovery.nql -->
```sql
find n.label as etiqueta, count(*) as total
from (n)
group by n.label
order by total desc
```

**Qué obtienes (seed=42):**
```
OffshoreEntity   52   ← empresas offshore (el activo principal a investigar)
Officer          50   ← personas que declaran ser dueños
ShellCompany     29   ← empresas pantalla (a menudo sin actividad real)
Intermediary      5   ← bufetes o agentes que montan las estructuras
Jurisdiction      4   ← países donde están registradas las empresas
```

**Regla de oro:** valida el schema antes de cualquier algoritmo. Si el dataset tiene 140 nodos pero `OffshoreEntity` solo devuelve 10, algo falló en la generación — no el algoritmo.

---

## Paso 2 — Índice hash: búsqueda instantánea por nombre

**¿Qué es un índice?**
Imagina buscar "Atlas Fiduciary Holdings" escaneando las 52 entidades offshore una por una — eso es una búsqueda lineal O(N). Un *Hash index* funciona como el índice de un diccionario: convierte el nombre en una dirección matemática y salta directo a ella en tiempo O(1), sin importar si tienes 52 entidades o 52 millones.

El generador crea automáticamente un Hash index sobre `OffshoreEntity(name)`. La siguiente query lo usa sin que tengas que hacer nada especial — el planificador de NQL detecta que el `WHERE` coincide con el índice y elige el lookup O(1).

<!-- source: queries/02_indices.nql -->
```sql
find e.name, e.industry, e.incorporated, e.shell, e.description
from (e:OffshoreEntity)
where e.name = "Atlas Fiduciary Holdings"
```

**Qué obtienes:** una sola fila con la entidad insignia plantada:
- Name: `Atlas Fiduciary Holdings`
- Industry: `private banking`
- Incorporated: `1986-04-17`
- Shell: `False` (el flagship no es una pantalla, es la holding raíz)

**Por qué importa la velocidad:** sobre 80 entidades la diferencia es imperceptible. Sobre un dataset real de 1M de entidades offshore, la diferencia entre con/sin índice es de 3-4 órdenes de magnitud.

---

## Paso 3 — Búsqueda semántica con HNSW

**¿Qué es un embedding?**
Un embedding es una representación numérica del *significado* de un texto. El modelo `all-MiniLM-L6-v2` lee la descripción de cada empresa y la convierte en un vector de 384 números (como coordenadas en un espacio de 384 dimensiones). Dos textos con significado parecido producen vectores cercanos en ese espacio — aunque no compartan palabras exactas.

**¿Qué es HNSW?**
*(Hierarchical Navigable Small World)* Es un índice especializado para encontrar los vectores más cercanos sin revisar todos. Construye una red jerárquica de "vecinos" con atajos entre niveles, similar a como funciona un mapa de carreteras con autopistas y calles locales. Búsqueda lineal: O(N) comparaciones. HNSW: O(log N) saltos. Sobre 1M de vectores, el speedup es de 1000×.

**Precondición:** el notebook genera embeddings de `e.description` con el modelo `all-MiniLM-L6-v2` (384d), los persiste en la DB con `add_node_embedding`, cierra y reabre el grafo (para que el HNSW se reconstruya desde Sled), y luego ejecuta esta query.

<!-- source: queries/03_hnsw.nql -->
```sql
find e.name, e.industry, e.shell
from (e:OffshoreEntity)
where similar_to(e, "Atlas Fiduciary Holdings", "minilm")
limit 5
```

**`similar_to(e, "<nombre_ref>", "<modelo>")`** resuelve el embedding de la entidad referenciada y hace una búsqueda HNSW por los `k=LIMIT` vecinos más cercanos por similitud coseno.

**Qué obtienes (top-5 con seed=42):**
```
1. Atlas Fiduciary Holdings    private banking   shell=False   ← self-match, coseno ≈ 1.0
2. Zephyr Holdings #079        commodities       shell=False
3. Sentinel Holdings #000      private banking   shell=False   ← misma industria, sale arriba
4. Spectrum Capital #040       art trading       shell=False
5. Cascade Holdings #078       art trading       shell=False
```

**Cómo interpretar:**
- El top-1 es la entidad misma — es una verificación del pipeline. Si no es top-1, algo falló.
- `Sentinel Holdings #000` sale en top-3 porque comparte industria (`private banking`) — el modelo capta señales de dominio aunque sea de propósito general.
- Para fraud detection en producción combinarías HNSW con filtros estructurales: "dame entidades similares a esta que además estén en jurisdicciones de alto riesgo" (Paso 4 muestra esa dimensión estructural).

### Caché local de embeddings: archivos `.npz` en `tutorials/precomputed/`

**¿Qué son los archivos `.npz`?**
`.npz` es el formato de archivo binario comprimido de NumPy (NumPy Zip). Cada archivo contiene uno o más arrays numpy guardados en un solo zip. Los que usa este tutorial tienen siempre dos arrays:

| Array | Forma | Tipo | Contenido |
|-------|-------|------|-----------|
| `texts` | `(N,)` | `object` | Los textos originales que se embebieron |
| `vectors` | `(N, 384)` | `float32` | Los vectores producidos por `all-MiniLM-L6-v2` |

**¿Dónde y cuándo se generan?**
La primera vez que corres el notebook (o con `force_recompute=True`), el helper en `tutorials/shared/embeddings.py` descarga el modelo, computa los vectores y los guarda localmente en `tutorials/precomputed/<cache_name>.npz`. En corridas posteriores, si los textos coinciden, carga el `.npz` directamente.

```python
# Lógica en tutorials/shared/embeddings.py
cache_path = PRECOMPUTED_DIR / f"{cache_name}.npz"
if cache_path.is_file():
    # Carga y valida que los textos coincidan (orden exacto o por conjunto)
    return cached_vectors
# Si no existe o los textos cambiaron → computa con SentenceTransformer y guarda
```

**¿Por qué no están versionados en git?**
Son artefactos derivados del modelo y del dataset generado localmente. El repo público conserva el código fuente que los produce, pero no versiona esos binarios.

**¿Qué pasa si los textos del dataset cambian?**
El helper compara los textos cacheados con los actuales — primero en orden exacto, luego como conjunto desordenado. Si no hay match en ninguno de los dos, descarta el caché y recomputa desde cero. Nunca usará vectores que no correspondan a los textos actuales.

---

## Paso 4 — Path queries con cuantificadores: seguir el rastro del dinero

**¿Qué es un "path" en un grafo?**
Un path es una secuencia de nodos conectados. `A → B → C` significa: "A controla a B, que a su vez controla a C". En fraude corporativo, los esquemas más sofisticados usan múltiples niveles de holding para ocultar al propietario real.

**¿Qué significa `-[:CONTROLS]->{1,3}`?**
El cuantificador `{1,3}` le dice al motor: "sígueme los edges `CONTROLS` de longitud mínima 1 y máxima 3 hops". Sin cuantificadores necesitarías 3 queries separadas (uno por nivel) y unir los resultados manualmente. Con `{1,3}` lo haces en una sola pasada.

*Simple-path semantics*: NopalDB garantiza que no revisita el mismo nodo dos veces en un path — evita bucles infinitos en grafos cíclicos.

<!-- source: queries/04_paths_quantifier.nql -->
```sql
find a.name as origen, b.name as destino, path.depth as hops
from (a:OffshoreEntity) -[:CONTROLS]->{1,3} (b)
where a.name = "Atlas Fiduciary Holdings"
order by hops, b.name
limit 20
```

**Qué obtienes (pirámide plantada, seed=42):**
```
hops=1 (hijos directos):    Crescent Resources #001, Sentinel Holdings #000, Tempest Advisors #002
hops=2 (nietos):            Aurora Group #008, Beacon International #003, Cobalt Resources #006,
                            Northern Investments #005, Onyx Investments #007, Sapphire Resources #004
hops=3 (bisnietos):         Eastpoint Resources #042, Onyx Holdings #047, Pinnacle International #038
Total: 12 paths
```

La pirámide es determinista — siempre verás exactamente 3 + 6 + 3 = 12 paths con seed=42.

---

## Paso 5 — Reducer `path_sum`: ¿cuánto dinero fluye por cada cadena?

**¿Qué hace `path_sum("amount")`?**
Suma el valor de la propiedad `amount` en cada arista a lo largo del camino. Si la arista A→B tiene `amount=5M` y B→C tiene `amount=2M`, entonces el path A→B→C tiene `path_sum = 7M`.

**¿Por qué importa la suma y no solo la longitud?**
En investigaciones financieras, las rutas más cortas no son necesariamente las más importantes. Lo que buscas son las rutas que mueven más dinero. Un path de 3 hops que transfiere 11.5M en total es más relevante que un path de 1 hop que transfiere 500K.

<!-- source: queries/05_path_minivm.nql -->
```sql
find a.name as origen,
     b.name as destino,
     path.depth as hops,
     path_sum("amount") as flujo_total
from (a:OffshoreEntity) -[:CONTROLS]->{1,3} (b)
where a.name = "Atlas Fiduciary Holdings"
order by flujo_total desc
limit 10
```

**Qué obtienes (top-3, seed=42):**
```
Pinnacle International #038   3 hops   11,480,044 ← máxima exposición total
Onyx Holdings #047            3 hops    7,664,621
Beacon International #003     2 hops    7,000,000 ← 2 hops pero transferencia directa grande
```

**Lectura:** las rutas más largas no son las más caras — depende de los `amount` individuales. Estos serían los paths a priorizar en una investigación real: `Pinnacle International #038` recibe dinero acumulado de tres transferencias encadenadas sumando ~11.5M.

`path_sum` / `path_min` / `path_max` / `path_avg` son los reductores disponibles. Con estos primitivos puedes construir análisis de flujo sin necesidad de exportar a otra herramienta.

---

## Paso 6 — PathAnomaly (E-10): detectar rutas inusuales

**¿Qué es detección de anomalías sin supervisión?**
En vez de definir reglas explícitas ("si hay más de 3 hops y el monto supera X, es fraude"), defines qué es "normal" con ejemplos de referencia y mides cuánto se aleja cada path de esa línea base. Es especialmente útil cuando no tienes ejemplos etiquetados de fraude.

**¿Cómo funciona `path_anomaly_score(node_model, edge_model)`?**
1. Registras uno o varios *paths de referencia* que representan comportamiento normal.
2. Cada referencia se convierte en un vector (promedio de los embeddings de sus nodos y aristas).
3. Para cada nuevo path, el motor calcula `1 − coseno(vector_path, centroide_referencias)`.
4. Score próximo a 1.0 → el path es muy diferente al baseline → señal de alerta.
5. Score próximo a 0.0 → el path se parece al baseline → comportamiento normal.

**Precondición:** registrar al menos un path de referencia con `add_path_reference_embedding(nombre, node_model, edge_model, vector)` que defina el "patrón normal".

<!-- source: queries/06_path_anomaly.nql -->
```sql
find a.name as origen, b.name as destino,
     path.depth as hops,
     path_anomaly_score("minilm", "edge_minilm") as anomaly
from (a:OffshoreEntity) -[:CONTROLS]->{1,3} (b)
order by anomaly desc
limit 10
```

**Qué obtienes:** el notebook registra primero un baseline (transferencia típica desde el flagship) y muestra qué paths se desvían más. En este dataset sintético las anomalías son moderadas; sobre datos reales el score se vuelve accionable.

**Por qué completa el ciclo:**
- Paso 3 → embeddings de nodos (significado textual)
- Pasos 4-5 → estructura de paths (cadenas de control + flujos)
- Paso 6 → embeddings de paths enteros comparados contra baseline = detección no supervisada de fraude

---

## Verificación cruzada (gate del Acto 2)

Los tres medios principales verifican invariantes distintos pero complementarios:

| Medio | Gate | Valor esperado |
|-------|------|----------------|
| **Notebook** | `similar_to` top-1 (HNSW self-match) | `"Atlas Fiduciary Holdings"` |
| **Rust** | `path_sum` top-1 destino | `"Pinnacle International #038"` (~11.48M, 3 hops) |
| **NDBStudio Web** | pegar `queries/03_hnsw.nql` | Requiere que el notebook haya persistido embeddings en la DB |

> **Nota sobre NDBStudio Web:** la query `similar_to` funciona en NDBStudio solo si los embeddings ya están persistidos en la DB (el notebook los persiste). El generador no los crea automáticamente. Si ejecutas `make studio-offshore` sin haber corrido el notebook, la query devolverá vacío.

### Cómo correr el gate de Rust

```bash
# Generar DB (si no existe)
python tutorials/shared/synthetic_offshore_dataset.py --db test_dbs/synthetic_offshore.db --reset

# Correr ejemplo Rust (pasos estructurales: schema, índice, paths, path_sum)
cargo run --example tutorial_acto_2_synthetic_offshore_paths -- test_dbs/synthetic_offshore.db
```

El ejemplo Rust no cubre embeddings (eso requiere Python + sentence-transformers). Su gate verifica la parte estructural: el path de mayor flujo desde el flagship.

---

## Walkthrough en NDBStudio

Ver [`ndbstudio_walkthrough.md`](ndbstudio_walkthrough.md) para el recorrido visual paso-a-paso.

## Siguiente

[Acto 3 — Biomedical OWL + reasoner](../acto_3_biomedical_owl/README.md): razonamiento explícito sobre ontologías OWL + validación de schema con SHACL.
