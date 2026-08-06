# Changelog

All notable changes to NopalDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.5.4] - unreleased

### ✨ Highlights

- **Integridad del redo del WAL.** Dos bugs encontrados por el nuevo oráculo de invariantes (nightly) quedaron cerrados: un commit que FALLA ya no se materializa tras reabrir (prevalidación de endpoints antes del WAL + registro de aborto como respaldo — semántica explícita: commit con `Err` = abortado definitivo, también tras un crash), y el redo ya no puede perder datos commiteados en secuencias que mezclan escrituras directas y transaccionales (marca de progreso por timestamp de commit; las guardas existentes se conservan como defensa). Los deletes de aristas de una transacción ahora viajan al WAL como el resto del write-set.

### Added

- Oráculo de invariantes estructurales con proptest en nightly: secuencias aleatorias de operaciones + verificación de biyección aristas↔adyacencia, RAM==disco tras reopen, punteros de versión válidos.
- Búsqueda vectorial exacta bajo umbral (determinista) y `ef_search` expuesto en Rust y Python (M1-6).

### Fixed

- El primer commit transaccional sobre un nodo creado con `add_node` directo fallaba con `NodeNotFound` (bug desde 0.4.27); el time-travel de esos nodos también quedó reparado.
- `add_nodes_batch` reseteaba la adyacencia en RAM de ids existentes.
- Familia de tests flaky de HNSW eliminada (20/20 × 3 tests).

## [0.5.3] - 2026-08-05

### ✨ Highlights

- **Layout de almacenamiento v2.** La adyacencia pasa de blobs
  `Vec<EdgeId>` por nodo a una clave binaria tipada por arista
  (`O|src|etype|tgt|edge` + espejo `I|…`): insertar o borrar una arista es
  O(1) — antes reescribía la lista completa del nodo, O(grado), y borrar
  un supernodo era O(grado²). Los datos viven en keyspaces separados con
  claves binarias order-preserving (entities, history, adjacency, indexes,
  catalog): se acabó la sopa de namespaces string del tree default. Los
  tipos de arista se internan (u32 en la clave) y el índice temporal se
  des-blobea: una clave `t|ts|nodo|versión` por entrada en vez del RMW de
  un `Vec` bajo el commit lock.
- **Migración automática al primer open.** Una base ≤0.5.2 abre con este
  binario y queda 100% en layout v2 sin intervención, vía una máquina de
  estados reanudable: copia (el legado queda intacto) → verificación por
  identidad (digest multiconjunto por namespace, no solo conteos) →
  activación → limpieza idempotente. Cada transición se hace durable antes
  de avanzar, así que un crash en cualquier fase se reanuda solo en el
  siguiente open. Si la verificación falla, el open devuelve error fuerte:
  el sentinel jamás se escribe y el legado sigue recuperable.

### ⚠️ DOWNGRADE

- **Una base migrada a layout v2 NO es legible por binarios ≤0.5.2** — y
  peor: un binario viejo podría ESCRIBIR datos en el formato antiguo,
  creando un universo paralelo dentro de la misma base. No hay vuelta
  automática; para volver a ≤0.5.2, usa `Storage::copy_database` hacia una
  base nueva con el binario nuevo, o restaura un backup previo a la
  migración. La marca de diagnóstico `meta:layout_migrated_to` queda
  escrita en el formato viejo para que soporte pueda distinguir el caso.

### Added

- Invariante de atomicidad cruzada: una arista y sus claves de adyacencia
  se confirman juntas o ninguna (`apply_multi`, batch atómico
  cross-keyspace en el contrato KV interno, con suite de conformidad para
  ambos motores). Sin ella, un crash podía dejar `edges` sin adyacencia o
  una dirección sin su espejo.
- Claves `idx:out/in:` huérfanas: la migración reconstruye la adyacencia
  desde `edges` (fuente de verdad), así que las huérfanas de nodos
  borrados que v1 acumulaba mueren al migrar y son imposibles hacia
  adelante.

### Changed

- **Rendimiento medido del layout v2** (misma máquina, vs 0.5.2): bulk load
  **−38%**, lecturas iguales o mejores, commits concurrentes iguales. El
  commit transaccional pequeño en **sled** paga ~+14% (la invariante de
  atomicidad cruzada usa su transacción multi-tree, que serializa con un
  lock global); en **redb** el mismo commit corre en ~6 ms — 2× más rápido
  que el baseline anterior de sled — porque su transacción única da la
  invariante sin costo extra. Trade-off aceptado a sabiendas: el backend
  experimental decidido absorbe la invariante gratis.

- La migración de layout corre ANTES del replay del WAL: el redo necesita
  reconciliar transacciones commiteadas del WAL sobre el estado ya
  migrado — replayar sobre keyspaces v2 vacíos re-crearía cadenas MVCC
  truncadas al sufijo del WAL. El porqué completo está documentado en el
  call site.
- `flush_indices` es ahora un no-op: la adyacencia se persiste por
  operación (dirección edges → disco → RAM) y los blobs en RAM dejan de
  tener autoridad sobre el disco.
- **Split del ecosistema:** `ndbstudio` y `nopaldb-mcp` viven ahora en
  github.com/Anxious-Mind-Group (con su historial completo). Este repositorio
  queda enfocado en el engine + wrapper Python y es MPL-2.0 puro. Sin cambios
  de código por este split.

### Fixed

- `add_edges_batch`/`BulkLoader` ahora persiste aristas + adyacencia
  directamente en el mismo batch atómico (antes dependía de un
  `flush_indices` posterior que alguien tenía que acordarse de llamar).
- Borrar un nodo ya no deja claves de adyacencia huérfanas: la purga de un
  supernodo va por chunks idempotentes (incluido el espejo del otro
  extremo) y la entidad se borra al final — un crash intermedio deja un
  nodo válido con menos aristas, no basura.

---

## [0.5.2] - unreleased

### ✨ Highlights

- **Migración entre motores, verificada.** `Storage::copy_database(src, …,
  dst, …)` copia una base completa entre engines (p. ej. sled → redb) byte a
  byte, keyspace por keyspace — nodos, aristas, historia MVCC completa
  (el time-travel sobrevive intacto), índices, relojes y embeddings. La
  verificación es doble: conteo de pares + checksum por keyspace,
  recalculados con un re-scan del destino; si no cuadra, la migración
  devuelve error. El destino debe estar vacío: nunca se mezclan bases.
  Ejemplo listo: `cargo run --example migrate_engine --features
  storage-redb -- <src> sled <dst> redb`.

### Added

- `MigrationReport` (pares y bytes por keyspace + veredicto de
  verificación), re-exportado en la raíz del crate.
- Bench `gc_removals` (GC de versiones MVCC end-to-end, escala por env) y
  `graph_ops` parametrizable por motor con `NOPALDB_BENCH_ENGINE=sled|redb`
  — la base de la comparación formal entre engines.
- CI: round-trip de migración sled→redb→sled verificado por checksum en
  cada build.

---

## [0.5.1] - 2026-08-03

### ✨ Highlights

- **Backend redb experimental** (`--features storage-redb`). Primer motor
  alternativo sobre el contrato KV de 0.5.0: pasa la misma suite de
  conformidad y el mismo crash harness (SIGKILL) que sled, con la suite
  completa del crate en verde usando redb como único backend. La base vive
  como un archivo `nopal.redb` dentro del directorio; la durabilidad
  replica el contrato de sled (escrituras diferidas + checkpoint durable
  periódico y al cerrar), con el WAL propio como garantía por commit.
- **Sentinel de engine**: abrir un directorio que contiene una base de otro
  motor falla con un error explícito en vez de crear una base vacía al
  lado — los locks del OS no protegen ese cruce.

### ⚠️ MSRV

- `rust-version` sube de 1.87 a **1.89** (lo exige redb 4.1). Es la
  negociación consciente para la que existe el pin explícito del workspace.

### Added

- `StorageEngine::Redb` (aditivo; el enum es `non_exhaustive` desde 0.5.0)
  y `engine="redb"` en los bindings Python. Sin la feature compilada, pedir
  redb da un error claro, no un panic.
- Jobs de CI para el backend: conformidad dual-engine, suite completa con
  redb como único backend, y crash harness nightly de 100 rondas SIGKILL.

---

## [0.5.0] - 2026-08-02

### ✨ Highlights

- **El storage quedó desacoplado del motor.** `Storage` (MVCC, adyacencia,
  índices) ahora opera sobre un contrato KV interno con una suite de
  conformidad que pinnea sus invariantes (orden, límites de prefijo,
  atomicidad de batch, RMW atómico). sled es la implementación por defecto,
  detrás de una feature real — un motor alternativo se agrega implementando
  dos traits y pasando la suite, sin tocar el resto del crate. Es la base
  para la evaluación de motores de siguiente generación.
- **Ventana única de breaking changes.** Todo lo incompatible de este ciclo
  va junto aquí (detalle abajo); 0.5.x en adelante vuelve a ser aditivo.

### ⚠️ Breaking

- `NopalError::StorageError` ya no expone `sled::Error`: envuelve un
  `StorageError { kind, message }` neutral al motor. Quien matcheaba la
  variante esperando el tipo de sled debe usar `kind()` (con
  `StorageErrorKind`: `Io`/`Corruption`/`Conflict`/`InvalidData`/
  `Unsupported`/`Internal`) y `message()`.
- `#[non_exhaustive]` en `NopalError`, `StorageEngine`, `StorageProfile` y
  `StorageErrorKind`: los `match` externos necesitan brazo comodín. Se paga
  una sola vez — agregar motores, perfiles o categorías de error ya no
  romperá a nadie.
- El trait `StorageBackend` fue **eliminado** (metadata y hooks que jamás
  tuvieron un caller). Los tipos `StorageEngine`/`StorageOptions`/
  `StorageProfile`/`StorageTuning` permanecen sin cambios.
- `rust-version = "1.87"` explícito en el workspace (MSRV real verificado
  por clippy; el código ya usaba API de 1.87).
- sled pasa a dependencia **opcional** detrás de `storage-sled` (activa por
  default). Compilar sin ningún backend produce un `compile_error!` con
  mensaje accionable.

### Added

- Contrato KV interno (`KvEngine`/`KvKeyspace`) + suite de conformidad
  parametrizada por motor: roundtrips 0 B–1 MiB, claves con `0x00`/`0xFF`,
  orden big-endian numérico, bordes de prefijo, batch todo-o-nada,
  RMW concurrente (el patrón de los relojes lógicos), independencia de
  keyspaces y persistencia tras reopen.
- `storage/keys.rs`: toda clave compuesta del almacén se construye y
  clasifica en un solo módulo — la lección del índice v1 (colisiones por
  stringificación dispersa) aplicada al namespace completo.
- `StorageError` y `StorageErrorKind` re-exportados en la raíz del crate.

### Fixed

- `StorageProfile::Server` no podía **abrir** una base: pedía compresión a
  un sled sin la feature `compression` (`Storage error: Unsupported: the
  'compression' feature must be enabled`) — roto desde que existen los
  perfiles. Esa feature además es inactivable en este workspace (su zstd
  0.9 colisiona por `links` con el zstd de parquet), así que el perfil ya
  no la pide y el backend la ignora con un warning: abrir jamás falla por
  tuning. El backend sled no comprime; la compresión volverá como capacidad
  por-keyspace de motores que la den sin conflicto.

### Changed

- GC de versiones MVCC: el borrado pasa de N `remove` individuales a un
  solo batch atómico — menos tiempo bloqueando escritores por ciclo de GC.
- La caché del perfil `Default` queda explícita en 1 GiB (el mismo valor
  que sled aplicaba implícitamente; un perfil no debe heredar en silencio
  el default de un motor).

---

## [0.4.36] - 2026-08-01

### ✨ Highlights

- **Índice de propiedades tipo-correcto (formato v2) con migración
  automática.** El formato anterior stringificaba el valor en la clave
  (`idx:prop:{name}:{value}`), así que `Int(1)`, `Float(1.0)` y
  `String("1")` compartían entrada — buscar cualquiera regresaba los nodos
  de los otros. El formato v2 usa claves tipadas (length-prefix del nombre +
  tag de tipo + valor canónico order-preserving) en un árbol sled propio
  (`prop_idx_v2`). Al abrir una base existente, la migración corre sola:
  borra el índice legado, reconstruye desde los nodos (fuente de verdad,
  chunks de memoria acotada) y escribe el sentinel `meta:prop_idx_format`.
  Idempotente y crash-safe: un crash a media migración simplemente la
  repite en el próximo open; los nodos y aristas jamás se tocan.

- **EXPLAIN honesto.** El plan que reporta `EXPLAIN` ahora sale de la misma
  decisión estática que usa la ejecución (`index_fast_path_decision`), no de
  un camino paralelo: reporta `INDEX SEEK` solo cuando el seek realmente
  ocurrirá, `LABEL SCAN` con la razón concreta (operador no-igualdad, RHS no
  literal, sin índice…), `FULL SCAN` para queries sin label (antes: error) y
  un mensaje claro para writes (antes: Debug dump del AST).

### Added

- `Graph::rebuild_property_index()` — reconstruye el índice de propiedades
  completo desde los nodos. Sirve para reparar un índice desactualizado
  (p. ej. tras escrituras index-blind con `insert_node`) y es la base de la
  migración v1→v2 y de un futuro `REINDEX`.
- `nopaldb/tests/prop_index_v2_test.rs` (contrato del índice tipado) y
  `nql_explain_honesty_test.rs` (matriz plan-reportado == ejecución-real),
  más las suites de migración/crash-safety como unit tests de storage.

### Changed

- **Nota de downgrade**: una base migrada a v2 y abierta con un binario
  ≤0.4.35 no se corrompe (nodos/aristas intactos), pero el índice legado ya
  no existe: los lookups por propiedad dan falsos negativos. Para volver
  atrás hay que reabrir con ≥0.4.36 o reconstruir el índice manualmente.
- `get_node_by_property` (que recibe `&str`) ahora busca `String` estricto:
  ya no «encuentra» nodos `Int`/`Float` vía la colisión del formato viejo.
  Es la restauración del contrato documentado; va aquí por si algún
  consumidor dependía de la colisión.
- `BTreeIndex::range_query` usa `BTreeMap::range` (O(log N + k)) en lugar de
  iterar todo el árbol con filter O(N). Misma semántica; `Between` con
  extremos invertidos regresa vacío (documentado — antes el filter también
  regresaba vacío, pero `range()` habría panicado).

### Fixed

- Colisiones de tipo del índice de propiedades (`Int(1)` == `Float(1.0)` ==
  `String("1")`, `Bool(true)` == `String("true")`, `Null` == `String("null")`),
  inyección de separador (prop `a` + valor `b:c` colisionaba con prop `a:b` +
  valor `c`) y floats no canónicos (`-0.0` y `0.0` eran claves distintas;
  todo NaN colapsa ahora a una clave canónica única).
- Python: `Transaction.add_node`/`add_edge` descartaban el error del motor
  (`let _ = …`) y devolvían el id de un nodo fantasma; ahora propagan la
  excepción y el id devuelto es el confirmado por el motor.
- Storage: los filtros del namespace `node:` usaban blacklists por substring
  (`contains(":v")`…) que funcionaban por accidente; ahora son predicados
  estructurales (prefijo + UUID parseado + sufijo exacto). El caso inverso
  (`get_all_versioned_nodes` aceptando sufijos futuros y reventando el
  export con `SerializationError`) quedó cerrado por la misma vía.

### Documented

- Hazard preexistente de `BTreeIndex`: el `Ord` de `PropertyValue` coerciona
  Int↔Float pero su `Eq` derivado no — claves numéricas heterogéneas se
  fusionan en el bucket del primero insertado. Pinneado en test; el fix real
  (normalización canónica) queda en el roadmap.
- `docs/PROPERTY_INDEXING.md` describe el formato v2 y la migración.

---

## [0.4.35] - 2026-07-27

### ✨ Highlights

- **API de conversión unificada en `PropertyValue`.** El tipo-contrato del
  motor ahora tiene su conversión canónica en el propio enum, y
  `with_property` acepta literales directamente:

  ```rust
  let n = Node::new("Person")
      .with_property("name", "Alice")   // &str → String
      .with_property("age", 30)         // i32  → Int
      .with_property("active", true);   // bool → Bool
  ```

- **Fixed (Python): `True` ya no se convierte en `1`.** Los conversores
  inline de `Transaction.add_node`/`add_edge` probaban `int` antes que
  `bool` (subtipo en Python). La frontera Python↔Rust usa ahora un conversor
  único, que además acepta `list`/`tuple`/`dict` anidados como propiedades.

### Added

- `From<bool/i64/i32/u32/f64/f32/&str/String/Vec<u8>/Vec<PropertyValue>/
  Vec<(String, PropertyValue)>/Option<T>>` para `PropertyValue`; exclusiones
  (`u64`/`usize`, `Vec<T>` genérico) razonadas en el rustdoc del enum.
- `PropertyValue::to_display_string()` + `impl Display` — render humano
  recursivo. Documentado: NO estable; jamás para claves de índice ni
  formatos persistidos.
- Puente `serde_json::Value` ↔ `PropertyValue` en ambas direcciones con
  matches exhaustivos (una variante futura rompe compilación, no serializa
  mal en silencio). Lossiness documentada: `Bytes` roundtripea como
  `List(Int)`; `NaN`→`null`.
- Doc del enum con el bloque «Restricciones de evolución»: agregar una
  variante es semver-major (sin `#[non_exhaustive]`), y el orden de
  declaración es load-bearing para `serde(untagged)` — pinneado en tests.
- Propiedades `list`/`tuple`/`dict` en los bindings Python (antes solo
  escalares y bytes), con guardia de roundtrip tipo-exacto en CI.

### Changed

- `Node::with_property`/`Edge::with_property` se generalizan a
  `impl Into<PropertyValue>`. Compatible con todo código que pasa un
  `PropertyValue` explícito; si un consumidor pasaba `x.into()` desnudo,
  la inferencia ahora es ambigua — usar `x` directamente o
  `PropertyValue::from(x)`.
- ndbstudio muestra floats no finitos como `null` de forma consistente en
  todas las vistas (una tenía drift a `NaN`/`inf`).
- Tutoriales: los helpers `prop_to_*` copiados 4 veces se sustituyen por el
  API del enum (`to_display_string`, `as_number`).

### Fixed

- Python: `bool` → `Int(1)` al escribir vía transacción (dos sitios).
- MCP: `Bytes` se serializaba como `null` por un wildcard `_ =>`; ahora sale
  como arreglo de números y el wildcard desapareció (matches exhaustivos en
  el core).

---

## [0.4.34] - 2026-07-27

### ✨ Highlights

- **`SackItem` ahora enlaza padre-hijo**: el resultado del sack traversal ya
  no es una lista plana irreconstruible. Cada ítem trae `parent`
  (índice del ítem padre en `items` — unívoco incluso cuando el mismo nodo
  aparece varias veces por caminos distintos) y `via_edge` (la arista por la
  que se llegó, que desambigua aristas paralelas y da acceso a sus
  propiedades). Con eso, un árbol anidado — p. ej. el BOM para una UI — se
  reconstruye en un solo paso O(n); el snippet está en el doc del módulo y
  en el ejemplo `bom_costing`, que ahora imprime el árbol indentado.

### Added

- `SackItem::parent: Option<usize>` — índice del ancestro emitido más
  cercano (`None` = hijo directo de un nodo de inicio). Con bloques de un
  salto, `items[parent].depth == depth - 1`; en `emit_leaves()` es siempre
  `None` (reporte plano, documentado).
- `SackItem::via_edge: Option<EdgeId>` — id de la arista seguida para
  llegar al nodo.

### Changed

- Nada rompe: `SackItem` es `#[non_exhaustive]` desde 0.4.33, así que los
  campos nuevos son aditivos para todo el código existente.

---

## [0.4.33] - 2026-07-26

### ✨ Highlights

- **Acumulador por traverser en el traversal fluido** (estilo `sack()` de
  Gremlin). Cada traverser acarrea un valor que se pliega con la arista en
  cada paso — la pieza que faltaba para cálculos como la explosión de
  materiales de un BOM, cadenas multiplicativas de probabilidad o conversión
  de unidades encadenada:

  ```rust
  let r = graph.traverse(raiz)
      .sack(10.0)
      .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
      .emit()
      .await?;   // (nodo, cantidad acumulada) por cada camino
  ```

- **Acceso a la arista durante el recorrido**: `sack_by`/`try_sack_by`
  reciben `&Edge`, y `filter_edge` filtra por propiedades de arista (antes
  solo se podía seleccionar por tipo).
- **Multiplicidad por camino**: sin deduplicación por nodo — el mismo nodo
  alcanzado por dos ramas aparece dos veces, cada una con su acumulador.
- **Truncamiento explícito**: `SackResult::truncated` distingue «terminé» de
  «me detuve por `max_depth`/`max_nodes`», y `TraversalResult.completed` hace
  lo mismo para `bfs`/`dfs`. Los ciclos dentro de `repeat` se manejan con
  `on_cycle(CycleMode::Error | Skip)` (default `Error`, con el camino del
  ciclo en el mensaje).

### Added

- `TraverseBuilder::sack(init)` → `SackBuilder<T>` genérico con
  `out/out_e/in_/in_e/both/filter/filter_edge`, pliegues `sack_by`,
  `try_sack_by`, conveniencias numéricas `sack_mul_by`/`sack_sum_by`
  (con coerción `Int`→`f64`; propiedad faltante o no numérica es error duro),
  `repeat(...)` acotado por `max_depth` (default 32, obligatorio) y
  `max_nodes` (default 10 000), y terminales `emit()` / `emit_leaves()`.
- Tipos nuevos exportados: `SackBuilder`, `SackBlock`, `SackResult`,
  `SackItem`, `CycleMode`, `Truncation`.
- `PropertyValue::as_number()` — valor numérico como `f64` coercionando
  `Int` (a diferencia de `as_f64`).
- Ejemplo `bom_costing` (`cargo run -p nopaldb --example bom_costing`).

### Changed

- **Minor breaking**: `TraversalResult` tiene un campo nuevo
  `completed: bool` (`false` si `bfs`/`dfs` se detuvieron por
  `max_depth`/`max_nodes`). Solo afecta a código que construya o destructure
  el struct literalmente; la lectura de campos no cambia.
- El manejo de ciclos (`on_cycle`) vive en el motor sack; en `bfs`/`dfs` la
  deduplicación por nodo ya garantiza terminación, así que `TraversalConfig`
  no cambia.

---

## [0.4.32] - 2026-07-22

### ⚖️ Licensing

- **Relicensed the `nopaldb` library to MPL-2.0.** The embeddable engine and its
  Python bindings (crates.io + PyPI) are now **Mozilla Public License 2.0** —
  file-level copyleft — so they can be embedded in projects under any license
  (MIT/Apache/proprietary) while changes to NopalDB's own files stay open. This
  unblocks adoption by permissively-licensed projects.
- The applications you *run* — **`nopaldb-mcp`** (MCP server) and **`ndbstudio`**
  (TUI) — remain **AGPL-3.0-only**, each with its own `LICENSE` file. The
  repository's top-level `LICENSE` is MPL-2.0 (the library is the primary
  artifact), so the repository's headline license reflects the library.
- Releases **≤ 0.4.31 remain AGPL-3.0-only** (published versions are immutable);
  this change applies from 0.4.32 onward.

---

## [0.4.31] - 2026-07-06

### ✨ Highlights
- **Cross-transaction WAL group commit**: concurrent commits queued at the single-writer applier share one fsync and apply in FIFO order. 8 concurrent committers: 98.3ms → 29.3ms (3.35×); single-commit latency unchanged.
- **`NqlResult` is iterable**: `for row in graph.execute_nql(...)` works as documented (delegates to the query result; write/index results iterate empty — use `.write`/`.summary`).
- **Adoption guide** ([docs/ADOPTION.md](docs/ADOPTION.md)): the fastest path in for Rust, Python, MCP and Studio users, plus the operational rules.

### Added
- NQL parser fuzzing (cargo-fuzz target, seeded corpus, nightly job — 5.26M executions, zero panics in the seed session).
- Dedicated PyPI project description (`pip install nopaldb` and Python quickstart instead of the crate README).

### Changed
- Python is now installable from PyPI; READMEs updated accordingly.

---

## [0.4.30] - 2026-07-06

### Fixed
- PyPI packaging: the sdist declared `License-File: LICENSE` but shipped the file nested under the crate subdirectory, so PyPI rejected the upload (400). The license (and README, for the project page) are now declared in `pyproject.toml` and land at the sdist root.

---

## [0.4.29] - 2026-07-05

### ✨ Highlights
- **Python isolation levels**: `graph.begin_transaction(isolation="serializable" | "repeatable_read" | "read_committed" | "read_uncommitted")` (requires the `full-isolation` feature).
- **Shared runtime + GIL release**: the Python bindings use one process-wide Tokio runtime and release the GIL during every DB call — measured ~3× read throughput with 4 threads (was serialized).
- **Isolation ↔ GC integration**: version GC runs under the single-writer gate (closing the GC-vs-version-list race), and direct writes are visible to Serializable conflict validation.

### Fixed
- **Lock leak**: failed commits (conflict/deadlock) and rollbacks did not release their locks, blocking subsequent writers on those nodes until the lock timeout; the commit path now cleans up on every error.
- Lock timeouts surface as `ConcurrencyError` instead of an opaque `Custom` error.
- WAL replay of committed edge inserts no longer crashes `Graph::open` when a later non-transactional delete removed the endpoints.

### Added
- Mixed-load SIGKILL crash harness and proptest model-based suite (random op sequences must match a reference model, including after reopen).
- Nightly hardening workflow (100 kill rounds + 200 proptest cases).
- `docs/DURABILITY.md`: the crash-safety contract, including the weaker durability of direct (non-transactional) writes.

### Security
- `arrow`/`parquet` 57 → 59: drops the transitive `thrift` dependency (RUSTSEC memory-allocation advisory).
- `tantivy` 0.25 → 0.26 and `ratatui`/`crossterm` 0.26/0.27 → 0.30/0.29: pull `lru` ≥ 0.16.4 (RUSTSEC `IterMut` soundness advisory); no more `lru` 0.12.x in the tree.

---

## [0.4.28] - 2026-07-04

Consolidated entry for the 0.4.x series.

### ✨ Highlights
- **Isolation Levels** (new opt-in `full-isolation` feature, included in the `full` tier):
  `ReadUncommitted` / `ReadCommitted` (default) / `RepeatableRead` / `Serializable`,
  per-node lock manager with wait-for-graph **deadlock detection**, and MVCC snapshot reads.
  See [docs/ISOLATION_LEVELS.md](docs/ISOLATION_LEVELS.md) and [docs/DEADLOCK_DETECTION.md](docs/DEADLOCK_DETECTION.md).
- **Storage concurrency**: removed the global storage lock; reads no longer block behind writers.
- **NQL**: path queries (quantifiers, metadata, reducers), embedding functions (`similar_to`, KNN,
  path similarity/anomaly), structured `PROFILE`, and write CRUD improvements.
- **Full-text search**: Tantivy-backed index behind the optional `fulltext` feature (in `core`).
- **MCP server**: agentic context tools (project-structure indexing, episodic events,
  PR-context validation), Arrow export over shared memory, input validation and NQL escaping.
- **NDBStudio**: query workbench with graph-hint fallback, session browser, timeline, and a web UI refresh.
- **Python**: abi3 wheels (one wheel for CPython 3.10+), PyPI release workflow, PyO3 0.29.

### Added
- `full-isolation` feature: `IsolationLevel`, `Transaction::with_isolation`, `LockManager`, deadlock detection.
- Synthetic benchmark examples (`examples/benchmarks.rs`, `examples/benchmark_community_dual.rs`) and benchmark report in `docs/benchmarks/`.
- New docs: property indexing internals, executor architecture (ES), Arrow performance notes, REPL workbench ADR/roadmap.

### Changed
- Version alignment across workspace, crate and Python package (0.4.28).
- Dependency updates including security bumps (`lz4_flex`, `time`, `tokio`, `rand`, `bytes`).

### Removed
- Orphan/dead modules (`nopaldb/src/isolation.rs` legacy stub, unused NDBStudio scaffolding).

---

## [0.3.0] - 2026-02-12

### ✨ Highlights
- **Zero Clippy Warnings**: Strict code quality enforcement across the entire codebase.
- **Graph Algorithms Stabilization**: API improvements and test coverage for all 6 algorithms.
- **Improved Type Correctness**: Refactoring of internals to use safer patterns (let-chains, type aliases).

### Changed
- **Algorithm APIs**: Standardized instantiation with `with_defaults()` instead of `default()` for better explicit configuration.
- **Code Quality**: Resolved over 40+ clippy warnings (collapsible ifs, needless borrows, etc.).
- **Cleanup**: Removed unused dependencies and artifacts.

## [0.2.0] - 2026-02-01

### 🎉 Major Release: Graph Algorithms & Advanced Queries

This release introduces **6 graph algorithms**, **schema inspection**, and **aggregation functions** integrated directly into NQL.

### Added

#### Schema Inspection API
- **New Module**: `src/schema/mod.rs` - Schema metadata management
- **Python API**: 8 new methods for schema introspection
  - `get_labels()` - Get all node labels
  - `get_edge_types()` - Get all edge types
  - `get_label_properties(label)` - Get properties for a label
  - `get_label_count(label)` - Count nodes by label
  - `get_edge_type_properties(type)` - Get edge properties
  - `get_edge_type_count(type)` - Count edges by type
  - `get_schema()` - Get complete schema metadata
  - `rebuild_schema()` - Force schema cache rebuild
- **Caching**: Efficient schema caching with dirty flag tracking

#### NQL Aggregation Functions
- **Statistical Aggregations**:
  - `count(n)` - Count nodes/edges
  - `sum(n.property)` - Sum numeric properties
  - `avg(n.property)` - Average of numeric properties
  - `min(n.property)` - Minimum value
  - `max(n.property)` - Maximum value
- **GROUP BY Support**: Full grouping with aggregations
- **Async Execution**: All aggregations run asynchronously

#### Graph Algorithms (6 Total)

1. **PageRank** (`src/algorithms/pagerank.rs`)
   - Classic PageRank algorithm
   - Personalized PageRank support
   - Configurable damping factor and iterations
   - NQL integration: `pagerank(n)`
   - Convergence detection

2. **Betweenness Centrality** (`src/algorithms/betweenness.rs`)
   - Brandes' algorithm implementation
   - O(VE) complexity for unweighted graphs
   - Normalized and unnormalized variants
   - NQL integration: `betweenness(n)`

3. **Clustering Coefficient** (`src/algorithms/clustering.rs`)
   - Local clustering coefficient
   - Global clustering (transitivity)
   - Triangle counting
   - NQL integration: `clustering(n)`

4. **Degree Centrality** (`src/algorithms/degree.rs`)
   - In-degree, out-degree, total degree
   - Normalized variants
   - Degree statistics (min, max, mean, median)
   - NQL integration: `degree(n)`

5. **Shortest Path** (`src/algorithms/shortest_path.rs`)
   - Dijkstra's algorithm (weighted)
   - BFS (unweighted)
   - Single-source shortest paths
   - Average path length calculation
   - Rust API only (NQL integration planned)

6. **Community Detection** (`src/algorithms/community.rs`)
   - Louvain method
   - Modularity optimization
   - Configurable resolution
   - Rust API only (NQL integration planned)

#### Examples
- `examples/schema_inspection.py` - Schema API demonstration
- `examples/synthetic_offshore_schema.py` - Synthetic offshore network analysis
- `examples/test_pagerank.py` - PageRank examples
- `examples/test_betweenness.py` - Betweenness examples
- `examples/test_clustering.py` - Clustering examples
- `examples/test_degree.py` - Degree centrality examples
- `examples/test_all_algorithms.py` - Complete algorithm suite test

#### Documentation
- `docs/python/SCHEMA_INSPECTION.md` - Complete API reference
- Algorithm documentation with examples
- Performance guidelines
- Best practices guide

### Changed

#### NQL Executor
- **Async Transformation**: `execute()` and `project_result()` now async
- **Aggregation Support**: New execution path for aggregations
- **Graph Access**: Aggregations now have access to Graph for algorithms

#### Type System
- **PropertyValue**: Used consistently across aggregations
- **Row Construction**: Improved with helper methods

### Fixed
- GROUP BY now correctly handles `n.label` as node field (not property)
- PropertyValue conversions in aggregations (Int vs Integer)
- Async compilation issues in executor chain
- Memory leaks in schema caching

### Performance
- **Schema Caching**: O(1) for cached schema lookups
- **Batch Operations**: BulkLoader API for efficient imports
- **Algorithm Optimization**: Efficient adjacency list construction
- **Memory**: Reduced allocations in hot paths

### Testing
- 50+ new unit tests
- Integration tests for all algorithms
- Schema API test coverage
- Aggregation test suite
- End-to-end Python tests

### Technical Debt Resolved
- Removed legacy aggregation detection code
- Cleaned up unused imports
- Fixed all compiler warnings
- Improved error messages

---

## [0.1.5] - 2026-01-15

### Added
- MVCC transaction support
- WAL (Write-Ahead Logging)
- Python bindings with PyO3
- Apache Arrow integration
- NQL v0.2 parser and executor
- BulkLoader API for efficient imports
- Synthetic offshore network demo and analysis

### Changed
- Migrated from single-threaded to async/await
- Improved transaction isolation
- Enhanced error handling

### Fixed
- Concurrency bugs in transaction manager
- Memory leaks in WAL
- Edge property serialization

---

## [0.1.0] - 2025-12-01

### Added
- Initial release
- Basic graph operations (nodes, edges)
- Simple query interface
- File-based storage with sled
- Python bindings (basic)

---

## Upcoming in v0.3.0 (Q2 2026)

### Planned Features
- Docker + Jupyter environment
- Performance benchmarks vs Neo4j
- Query optimization
- Sharding support
- PyPI publication
- crates.io publication

### Under Consideration
- GraphQL API
- REST API
- WebAssembly build
- Real-time subscriptions
- Visual query builder

---

## Migration Guides

### Migrating from v0.1.5 to v0.2.0

#### Schema API (New)
```python
# Before: No schema introspection
# Had to query to discover structure

# After: Direct schema access
labels = graph.get_labels()
schema = graph.get_schema()
properties = graph.get_label_properties("Person")
```

#### Aggregations (New)
```python
# Before: Manual aggregation in Python
result = graph.execute_nql("find n from (n:Person)")
count = len(list(result))

# After: NQL aggregations
result = graph.execute_nql("find count(n) from (n:Person)")
count = list(result)[0].get('count')
```

#### Graph Algorithms (New)
```python
# Before: External libraries (NetworkX)
# After: Built-in NQL functions
result = graph.execute_nql("""
    find n.name, pagerank(n) as rank
    from (n:Person)
    order by rank desc
    limit 10
""")
```

---

## Breaking Changes

### v0.2.0
- None! Fully backward compatible with v0.1.5

### Future Breaking Changes (v1.0.0)
- May remove deprecated APIs
- NQL syntax standardization
- Python API cleanup

---

**Version Scheme**: MAJOR.MINOR.PATCH
- **MAJOR**: Breaking changes
- **MINOR**: New features (backward compatible)
- **PATCH**: Bug fixes

---

[0.2.0]: https://github.com/sharop/nopaldb/releases/tag/v0.2.0
[0.1.5]: https://github.com/sharop/nopaldb/releases/tag/v0.1.5
[0.1.0]: https://github.com/sharop/nopaldb/releases/tag/v0.1.0
