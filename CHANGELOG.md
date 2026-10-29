# Changelog

All notable changes to NopalDB will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.10.2] - unreleased

NQL `at timestamp` lee el pasado (#298, #326): fase 3 del plan MVCC. Sin
cambios incompatibles de API ni de formato en disco.

### Added
- **NQL `find … at timestamp <ts>` lee el grafo de entonces** (#298). La
  consulta corre sobre el grafo válido en `ts`, materializado desde la
  historia MVCC: patrones, `where`, agregaciones, algoritmos, `instanceOf`,
  `explain`/`profile` y `export`. Es también la primera lectura en el pasado
  desde Python. Bajo el horizonte del GC o bajo el suelo de historia de una
  base adoptada es `HistoryUnavailable`; `similar_to`, `hybrid`,
  `in_community`, los scores de búsqueda y las funciones de embeddings son un
  error con nombre (leen datos sin historia). Coste: O(historia entera) por
  consulta, y `explain` lo dice. Hasta 0.10.1 la cláusula era un error con
  nombre.

### Changed
- Un `Executor` usado directamente con una consulta `at timestamp` da error
  (la cláusula la aplica `Graph::execute_nql`/`execute_statement`).

## [0.10.1] - 2026-10-29

Toda escritura deja historia MVCC (#319, #320). `add_node` directo y la
importación Turtle/OWL versionan; el BulkLoader guarda una marca de
nacimiento sin copiar el registro; las aristas encadenan versiones. El primer
open de una base anterior adopta, una vez y de forma reanudable, lo escrito
sin versión con un suelo de historia por entidad. Sin cambios incompatibles
de API; formato en disco compatible (0.10.0 y 0.9.4 abren la base, pero no
deben escribir en ella: ver §6 de
[docs/UPGRADING_0.10.md](docs/UPGRADING_0.10.md)).

### Fixed
- **Toda escritura deja historia MVCC** (#319). `add_node` directo versiona
  con el mismo timestamp que va al WAL (antes la misma escritura tenía
  historia solo si había un crash); la importación Turtle/OWL y
  `storage_insert_*` (ahora por WAL, versión e índices) también.
  `get_class_nodes_at` pierde el fallback al estado actual.
- **El BulkLoader da historia a lo que carga** (#319): un id nuevo recibe una
  marca de nacimiento —su versión es el registro mismo, válido desde el lote,
  sin copia— y un id existente continúa su cadena. Avanza y persiste el reloj
  y `last_commit_timestamp`. La primera escritura que toca una entidad con
  marca escribe antes su v1 desde el registro.
- **Las aristas encadenan versiones** (#319): reescribir una arista cierra la
  versión vigente y escribe v+1; recrearla tras borrarla sigue desde su
  versión más alta (antes se pisaba la v1).
- **Reproducir el WAL entero sin la marca de progreso ya no borra** un nodo o
  arista recreado después del borrado registrado (#319).
- Una transacción RepeatableRead o Serializable lee los nodos del BulkLoader.

### Added
- **Adopción de historia al abrir una base anterior**, una sola vez y
  reanudable (#319): lo escrito sin versión recibe un piso por entidad (antes
  del piso, `HistoryUnavailable`; desde el piso, el registro actual), sin
  copia; las cadenas desincronizadas reciben v+1 y las abiertas de entidades
  ya borradas se cierran.
- `Graph::history_floor`, `Graph::edge_history_floor`,
  `VersionedEdge::new_version`, `storage::META_HISTORY_ADOPTED` y
  `storage::META_HISTORY_ADOPTION_TS`.

### Changed
- Las primitivas sin historia de `Storage` (`insert_node`,
  `insert_nodes_batch`, `insert_edge*`) retiran la marca de lo que pisan sin
  cadena: esa entidad pasa a `HistoryUnavailable`.
- Costo: BulkLoader de nodos +6 % de tiempo y disco, de aristas +23 % de
  tiempo; el primer arranque de una base anterior tarda unos 0.26 s por 100k
  nodos + 100k aristas.
- `ELReasoner::from_graph_at(ts)` por debajo del piso devuelve
  `HistoryUnavailable` en vez de usar la taxonomía actual.

### Docs
- Guía de actualización a 0.10 (EN/ES): adopción de historia en el primer
  open, marca de nacimiento y suelo, cadena de aristas, cómo volver a 0.10.0
  o 0.9.4 y una sección "de 0.10.0 a 0.10.1". Al día también la referencia
  NQL (time travel), DURABILITY, ARCHITECTURE (versiones implícitas) y
  BACKUP_AND_READ_ONLY (el primer open adopta).

## [0.10.0] - 2026-10-28

Historia MVCC correcta y GC seguro (#313, #314, #315). Borrar un nodo cierra
su historia y la de sus aristas; el redo del WAL conserva los timestamps del
commit; el GC por antigüedad ya no borra toda la historia cerrada y guarda su
horizonte; `get_node_at`/`as_of` no devuelven el estado actual cuando no hay
versión; el `update` de NQL pasa por el WAL y no se pierde en un crash.

**Versión menor por dos cambios incompatibles en Rust:** `GCConfig` gana el
campo público `min_age_ms`, y `get_node_at`/`as_of` devuelven `NodeNotFound`
o `HistoryUnavailable` donde antes daban el nodo actual. Guía:
[docs/UPGRADING_0.10.md](docs/UPGRADING_0.10.md). El formato en disco y el
del WAL no cambian.

### Fixed
- **Borrar un nodo cierra su historia MVCC** (#309). Su versión vigente y la
  de cada arista borrada en cascada reciben `valid_to` = timestamp del
  borrado, en el mismo lote atómico que el borrado, por todos los caminos
  (`delete_node`, transacción, NQL `delete`, `delete_node_by_key` / Python
  `delete` y el redo del WAL). Antes la historia quedaba abierta y
  `as_of`/`get_node_at` posteriores seguían mostrando el nodo y sus aristas.
  El pasado sigue legible, y volver a crear un nodo con el mismo id continúa
  su cadena (v2…) sin pisar la anterior. Los nodos borrados antes de esta
  versión conservan su historia abierta.
- **El redo del WAL usa el timestamp registrado del commit** (#309) para
  altas y bajas de aristas y borrados de nodos. Antes tomaba timestamps
  nuevos y, tras un crash, la historia reconstruida tenía otras fechas. El
  formato del WAL no cambia: un WAL pendiente de 0.9.4 se reproduce bien.
- **Borrar una arista cierra su versión en el mismo lote atómico** que el
  registro y la adyacencia; antes eran dos escrituras.
- **GC: `GCConfig::older_than_ms/hours/days` y `Graph::gc_default` borraban
  toda la historia cerrada** (#310). Comparaban milisegundos Unix con los
  timestamps lógicos de las versiones. Ahora la edad se traduce al reloj
  lógico con marcas reloj-de-pared → lógico que la base guarda (a lo sumo una
  por segundo); la traducción es conservadora: solo se borra lo que la base
  puede probar que se cerró hace al menos esa edad. **En una base escrita por
  ≤ 0.9.4 el GC por edad no borra nada hasta que pase esa edad desde la
  primera escritura con esta versión.**
- **`Graph::get_node_at` y `as_of(ts)` ya no devuelven el estado actual**
  cuando no hay versión en `ts` (#311): un nodo que no existía entonces da
  `NodeNotFound`; uno sin historia (escrito con `add_node` directo) o un `ts`
  bajo el horizonte del GC da el error nuevo `NopalError::HistoryUnavailable`.
  `GraphSnapshot::get_nodes` solo omite los `NodeNotFound`.
  `get_class_nodes_at` (reasoner) ya no incluye clases que aún no existían.
- **NQL `update … set` escribe por la transacción de la sentencia** (#312),
  como `add` y `delete`: va al WAL, deja versión MVCC y mantiene también los
  índices hash/btree/full-text de usuario, y es atómico por sentencia. Hasta
  0.9.4 escribía directo en el motor: un crash antes del siguiente checkpoint
  perdía un `update` ya confirmado, un índice hash seguía encontrando el nodo
  por el valor viejo y un fallo a media sentencia dejaba la mitad escrita.
- **El redo del WAL ya no se salta la reescritura de propiedades de una arista
  existente** (#312): NQL `update (a)-[r]->(b) set r.x = …` o `tx.add_edge` con
  un id existente se perdían en un crash. Reescribir una arista todavía
  reemplaza su versión en lugar de encadenar otra.

### Added
- **Guía de actualización a 0.10** desde 0.9.4
  ([docs/UPGRADING_0.10.md](docs/UPGRADING_0.10.md),
  [docs/es/ACTUALIZAR_0.10.md](docs/es/ACTUALIZAR_0.10.md)): los cambios
  incompatibles de `GCConfig` y de `get_node_at`/`as_of`, el arranque del GC
  por antigüedad, la historia que se cierra al borrar y el `update`
  transaccional. Ejemplos de instalación en `"0.10"`; referencia NQL (time
  travel), `DURABILITY`, `OPERATIONS`, `ARCHITECTURE` y el cheat sheet al día.
- `Graph::last_commit_timestamp()` (Rust y Python, #311): el timestamp
  lógico de la última escritura aplicada, el que se pasa a
  `as_of`/`get_node_at`. No lo mueven `begin_transaction`, los rollbacks ni
  los commits vacíos.
- `Graph::gc_horizon()` (Rust y Python; también en `stats().gc.horizon` /
  `get_stats()["gc"]["horizon"]`, #310): hasta dónde recolectó el GC,
  persistido.
- `GCConfig::before_timestamp(ts)` para cortes lógicos explícitos.

### Changed
- **Incompatible: `GCConfig` tiene un campo nuevo, `min_age_ms`** (#310). Un
  literal que nombre todos los campos sin `..Default::default()` deja de
  compilar; también `GcAutoSummary` y `GcSection` ganan campos.
  `older_than_*` deja `cutoff_timestamp = u64::MAX` y pone `min_age_ms`.
- **`get_node_at` / `as_of` devuelven errores** donde antes devolvían el nodo
  actual (#311).

## [0.9.4] - 2026-10-27

Arreglos de resultados silenciosos encontrados al revisar la documentación
contra el código: consultas NQL que devolvían filas incorrectas o ignoraban
parte de la consulta sin error (#293, #294, #298, #299), un `export` que
respondía éxito sin escribir (#295), candados de Serializable que no se
soltaban (#296), `path_anomaly_score` constante (#297) y listas que volvían
como bytes (#300). El crate vuelve a compilar con su versión mínima de Rust
(1.89). Guía de actualización y documentación revisadas.

**Para volver atrás:** una base escrita por 0.9.4 que contenga una lista de
enteros 0–255, una lista vacía, una lista de pares `[texto, valor]` o un
objeto vacío no la abre 0.9.3 o anterior (#300).

### Fixed
- **NQL: un patrón ligaba el nodo equivocado a una variable** (#293). Ambas
  daban filas incorrectas sin error.
  - En un extremo anónimo, la variable se adivinaba de la proyección y podía
    tomar el nombre que el patrón da al otro extremo:
    `find u.name from ()-[:F]->(u)` devolvía los **orígenes**, y el "más
    seguido" del quickstart de Python contaba mal. Ahora un extremo anónimo
    nunca toma un nombre del patrón.
  - Una variable repetida se re-ligaba en vez de exigir el mismo nodo:
    `(a)-[:F]->(b)-[:F]->(a)` devolvía caminos que no cierran. Ahora la
    segunda `a` tiene que ser la primera (también una arista repetida), y
    `(a)-[:F]->(a)` en un salto es un self-loop.
- **NQL: condiciones y alias que filtraban mal sin avisar** (#294).
  - Una propiedad, variable o literal sueltos en `where`/`having`
    (`where p.active`) se aceptaban: el `find` dejaba pasar todos los nodos y
    `delete`/`update` no tocaban ninguno. Ahora es un error con nombre que
    pide la comparación (`p.active = true`).
  - Un alias de la proyección en `where`/`having` se leía como propiedad
    inexistente. `find degree(n) as dg … where dg > 1` daba 0 filas y
    `avg(r.w) as m … having m > 0.5`, ninguna. Ahora el alias es su
    expresión; un alias que coincide con una variable del patrón no se
    sustituye.
  - Con un algoritmo proyectado (`degree(n) as dg`), `where degree(n) > 1`
    en una consulta de un nodo se ignoraba: el camino de agregaciones no
    aplicaba el filtro de algoritmos. Ahora lo aplica.
  - `having avg(r.w) > 0.5` pedía `r.w` en el GROUP BY; los argumentos de
    una agregación ya no se validan contra él.
- **NQL: `group by … having` en una consulta de un nodo devolvía todos los
  grupos** (#299). El camino de patrones aplicaba el `having`; el de un solo
  nodo (`find p.town, count(*) … from (p:T) group by p.town having count(*) > 1`)
  no lo evaluaba. Ahora lo aplica por grupo, con agregaciones, algoritmos,
  propiedades de la clave y `and`/`or`.
- **NQL: `at timestamp <n>` se aceptaba y se ignoraba** (#298): la consulta
  leía el estado actual sin aviso. Ahora es un error con nombre que remite a
  la API de time travel de Rust (`Graph::as_of`, `Graph::get_node_at`,
  `Graph::history`), hasta que NQL lea de verdad en un instante.
- **El crate no compilaba con Rust anterior a 1.96**, aunque declara 1.89
  como versión mínima: `easter_eggs::motivational_message` devolvía un
  `&&str` de un arreglo local que solo 1.96 promovía a `'static`. Corregido,
  y la CI gana el job `msrv`, que compila con el `rust-version` del
  `Cargo.toml` (hoy 1.89) para que no vuelva a pasar.
- **NQL: `where false` dejaba pasar todas las filas**: los evaluadores de
  condiciones trataban cualquier expresión que no fuera una comparación como
  verdadera. Ahora `true`/`false` valen lo que dicen (#294).
- **Una lista de enteros pequeños, una tupla o una lista vacía volvían como
  `bytes`** (#300). `Bytes` se serializa como una secuencia de enteros, igual
  que una lista, y `PropertyValue` sin etiqueta probaba `Bytes` primero: en
  el storage (MessagePack) y en el WAL (JSON), `[1, 2]` volvía como
  `b'\x01\x02'`, `[]` como `b''`, una lista de pares `[texto, valor]` como
  objeto y el objeto vacío como `bytes`. Ahora esas tres formas se escriben
  envueltas (`{"$list": …}`, `{"$object": []}`) y vuelven como se
  escribieron; todo lo demás se escribe con los mismos bytes que antes y lo
  ya guardado se lee igual. **Lo que se guardó antes de 0.9.4 con esas formas
  sigue leyéndose como `bytes`** (en disco no se distingue de unos bytes):
  hay que reescribirlo. Una base escrita por 0.9.4 con alguno de esos valores
  no la lee 0.9.3 o anterior.
- **NQL `export` respondía éxito por cosas que no hacía** (#295).
  - `export parquet "ruta"` contestaba `Exported N rows as Parquet` sin
    escribir el archivo, y `export arrow` devolvía un resumen de texto. Ahora
    `export parquet "ruta"` escribe el Parquet (mismo escritor que
    `Graph::export_parquet`; tipo de cada columna inferido de sus valores) y
    `export arrow with path="…"` escribe un archivo Arrow IPC. `export arrow`
    sin `path` es error, y sin la feature `analytics` los dos son un error con
    nombre.
  - Las opciones desconocidas se ignoraban: `export csv with delimiter=";"`
    escribía comas. Ahora fallan con la lista de opciones válidas (la del
    separador es `separator`). Una opción de texto con otro tipo (`path=3`)
    también es error.
  - `sketch` / `commit` en `execute_statement`/`execute_nql` contestaban un
    mensaje OK sin hacer nada. Ahora son error; desde Rust se usa
    `SketchManager`.
  - En Python, `NqlResult.data` expone el contenido del export (texto
    CSV/JSON, o la ruta escrita en Arrow/Parquet). Antes el export sin `path`
    no se podía leer.
- **Serializable: descartar o revertir una transacción ya suelta sus
  candados** (#296). Solo los soltaban un `commit` fallido y
  `rollback_async()`. El `Drop`, el `rollback()` síncrono, un `?` que
  propagaba un error (por ejemplo `Deadlock` en `add_node`) o un `commit`
  cancelado dejaban los nodos tomados, y el siguiente escritor fallaba con
  `Lock timeout` a los 5 s. El lock manager libera ahora de forma síncrona
  (`LockManager::release_locks_sync`, nuevo; `release_locks` se conserva), así
  que toda salida sin commit suelta los candados en el acto.
- **`path_anomaly_score` devolvía `1.0` fijo cuando la referencia tenía otra
  dimensión** (#297). El vector de un camino es la concatenación
  `media(nodos) ‖ media(aristas)` (`node_dim + edge_dim`, p. ej. 384 + 384 =
  768). Una referencia de otro tamaño se descartaba sin aviso y, sin ninguna
  compatible, el score era 1.0 para todos los caminos. Ahora una referencia de
  otra dimensión, aunque haya otras compatibles, es un error que nombra la
  referencia, ambas dimensiones y cómo se construye el vector, y un centroide
  de norma cero también es error. Sin referencias se mantiene `1.0`, como
  estaba documentado. `path_embedding_similarity` usa el mismo mensaje. Se
  corrigen el docstring de `add_path_reference_embedding` (daba otra fórmula)
  y el paso 6 del acto 2 del tutorial, que registraba la referencia a 384.
- **Logs de candados sin el id del nodo** (#304): CodeQL marcaba el id como
  dato sensible en los `log::debug!` de `lock_manager.rs`; bastan la
  transacción y el tipo de candado.
- **Documentación revisada contra el código de 0.9.4**: 57 archivos, con los
  ejemplos ejecutados. Lo que un lector copiaba y fallaba, o contradecía al
  código:
  - unos 90 comentarios `--` en bloques NQL (NQL solo acepta `//` y
    `/* */`), `<-> [:T] <->` en lugar de `<-[:T]->`, `<>` en lugar de `!=`,
    `create` en lugar de `add`;
  - métodos de Python que no existen (`graph.add_node`, `export_to_pandas`,
    `Transaction.delete_node`, variables `_source`/`_target`), firmas
    incorrectas (`add_node(label, properties=None)`) y "zero-copy" de Rust a
    Python (Python recibe bytes Arrow IPC);
  - la referencia NQL documentaba `delimiter` (la opción es `separator`) y
    decía que Arrow/Parquet no se exportan; `at timestamp` se documenta como
    lo que es;
  - aislamiento: ReadUncommitted no permite lecturas sucias (es
    ReadCommitted), RepeatableRead no detecta escritura-escritura y
    Serializable bloquea solo nodos; el ejemplo de reintento de deadlocks no
    capturaba el error donde nace;
  - el índice btree no acelera rangos en NQL, `QUERY_PLANNER_DOCS.MD`
    reescrito sobre lo que existe, `community_fast` descrito como lo que es,
    cifras sin medición retiradas o marcadas como ilustrativas;
  - Rust ≥ 1.89 (no 1.85) en la página de crates.io, nopaldb sí está en PyPI
    (`tutorials/`), y la guía de desarrollo con rutas, comandos y targets de
    `make` reales.

### Added
- **Guía de actualización a 0.9.4** desde 0.6–0.9.3
  ([docs/UPGRADING_0.9.md](docs/UPGRADING_0.9.md),
  [docs/es/ACTUALIZAR_0.9.md](docs/es/ACTUALIZAR_0.9.md)): datos que revisar
  (negativos sin signo antes de 0.8.3, DELETE/UPDATE con patrón antes de
  0.8.4), consultas que ahora devuelven otras filas, la exportación Arrow de
  0.9.1, los cambios de la API de Rust y qué probar después de actualizar.

## [0.9.3] - 2026-10-26

Arreglo del deadlock que podía colgar para siempre la construcción en
paralelo de un índice HNSW (#289, cierra #288). La construcción sigue en
paralelo y los volcados del índice no cambian de formato.

### Fixed
- **Construir un índice HNSW en paralelo podía quedarse colgado para siempre**
  (#288). Desde 128 vectores, `build_batch` y los lotes incrementales usan
  `parallel_insert` de `hnsw_rs`. Su actualización inversa de vecinos recorría
  la lista del punto nuevo con el candado de lectura tomado y, dentro del
  bucle, pedía el de escritura de cada vecino. Dos inserciones simultáneas
  podían esperarse para siempre (≈ 1 construcción de cada 180 bajo carga, y
  también en `hnsw_rs` 0.3.5). Ahora NopalDB usa una copia interna de
  `hnsw_rs` 0.3.4 (MIT/Apache-2.0, con atribución, en `src/embeddings/hnsw_rs`)
  que copia la lista antes de tomar ningún candado de escritura. La
  construcción sigue en paralelo y los volcados del índice no cambian de
  formato. Se descartó insertar en serie: con 16 núcleos es ~10× más lento
  (50k × 384: 33 s → 318 s).

### Changed
- `hnsw_rs` deja de ser dependencia; entran directamente las suyas
  (`anndists`, `rayon`, `parking_lot`, `mmap-rs`, `hashbrown`, `indexmap`,
  `cpu-time`), que ya llegaban por él. La feature `hnsw-simd` activa ahora
  `anndists/simdeez_f`.
- `--features embeddings-index` sin `hybrid` vuelve a compilar con
  `-D warnings` (un constructor que solo usa `search_hybrid` quedaba como
  código muerto), y el CI comprueba esa combinación.

## [0.9.2] - 2026-10-25

Conversión con NetworkX en las dos direcciones (`nopaldb.networkx`, extra
`nopaldb[networkx]`) y una lectura consistente del grafo
(`Graph.export_graph`), que ahora usa también la exportación Arrow (#285).
Todo aditivo.

### Added
- **Conversión con NetworkX** (`nopaldb.networkx`, con el extra
  `pip install "nopaldb[networkx]"`). Ver
  [docs/python/NETWORKX.md](docs/python/NETWORKX.md).
  - `to_networkx(graph, label=None, key=None)` devuelve un `MultiDiGraph`:
    conserva aristas paralelas, self-loops y nodos aislados. Las propiedades
    van en sus tipos nativos (nada pasa a texto), y un nombre reservado sale
    como `property.<nombre>`.
  - `from_networkx(graph, G, …)` es idempotente. Hace upsert de cada nodo
    por `nx_key` (su clave de NetworkX) y omite las aristas que ya existen.
    Una clave que no sea str, int o float da error con nombre.
- **`Graph.export_graph(label=None)`** (Rust `Graph::export_snapshot`): el
  grafo, o el subgrafo inducido por una etiqueta, leído como **un solo estado
  consistente**. Toma el write gate mientras lee, así que una arista nunca
  aparece sin sus extremos. La exportación Arrow (`graph_to_arrow`) también
  lo usa ahora.

## [0.9.1] - 2026-10-24

La exportación Arrow con propiedades deja de perder o inventar datos (#282):
subgrafo inducido por etiqueta, columnas en orden estable, JSON en lugar de
marcadores, y `Null` como nulo. Sin cambios de API.

### Fixed
- **La exportación Arrow con propiedades** (`graph_to_arrow`, Python
  `to_arrow_complete`, `nodes_to_arrow_with_properties`,
  `edges_to_arrow_with_properties`) **daba datos incorrectos o incompletos
  sin aviso:**
  - Con filtro de etiqueta exportaba **todas** las aristas del grafo. Ahora
    solo las que tienen los dos extremos dentro (subgrafo inducido).
  - El orden de las columnas de propiedades salía de un mapa sin orden. Ahora
    van por nombre, y el mismo grafo da el mismo esquema.
  - Una propiedad que solo tenía listas, objetos o bytes no generaba columna,
    y un valor de otro tipo en una columna entera salía nulo. Ahora una
    columna mixta es texto y no pierde valores.
  - Listas, objetos y bytes salían como `<complex>` / `<bytes:N>`, y un
    `Null` explícito como el texto `"null"`. Ahora van en JSON, y `Null` es
    nulo de Arrow.
  - Una propiedad llamada `id` o `label` (`source`, `target` o `edge_type` en
    aristas) duplicaba la columna estructural. Ahora sale como
    `property.<nombre>`.
  - Exportar sin nodos era un error. Ahora devuelve un lote vacío con su
    esquema.

  Quien leía `<complex>` o el texto `"null"` verá JSON y nulos. Contrato
  completo en `docs/arrow/03-TECHNICAL.md` § Vista analítica.

## [0.9.0] - 2026-10-24

Cierra la fase 2b de #214: buscar dentro de una comunidad o una vecindad,
desde Rust, Python y NQL (`in_community`). La semántica de patrones queda
uniforme (0.8.4) y los contratos de NQL, escritos (0.8.5). Sube la versión
menor por dos cambios de API: `ExpandOptions` gana el campo `within`, que
solo rompe una construcción sin `..Default::default()`, y se retira lo
deprecado en 0.8.1.

### Removed
- **Lo deprecado en 0.8.1 y 0.6.11, como se anunció:**
  - `Graph::update_node_with_tx` no hacía nada. Se reemplaza por
    `tx.add_node(node)` con el mismo id, que escribe una versión nueva al
    commitear.
  - `Graph::delete_node_with_tx` y `Graph::add_node_with_label_and_props`
    ignoraban la transacción y escribían directo. Se reemplazan por
    `tx.delete_node(id)` / `tx.add_node(...)` o por `Graph::delete_node` /
    `Graph::add_node`.
  - `mvcc::VersionManager` era un placeholder vacío. Se reemplaza por
    `Graph::get_node_at` y `Graph::history`.
  - `HnswIndex::repaired()` devolvía siempre 0 desde 0.6.11. Se reemplaza
    por `HnswIndex::orphans()`.

### Added
- **Buscar dentro de una comunidad o de una vecindad** (#214 fase 2b).
  - `Graph::community_members(key)` (Python `community_members(key)`)
    devuelve los miembros de una comunidad materializada.
  - `Graph::community_members_via(key, "MENTIONS")` (Python
    `community_members(key, via="MENTIONS")`) devuelve los nodos con una
    arista `MENTIONS` hacia un miembro: los chunks que mencionan las
    entidades de la comunidad. Es el salto entre donde se calculan las
    comunidades y donde se busca.

  Pasados como `candidates` a `search_hybrid`, el top-k se calcula **dentro**
  de la comunidad. Las dos leen solo las aristas de esa comunidad y de sus
  miembros, nunca todo el grafo, y una clave inexistente da error. El
  resultado es igual, como conjunto, al del patrón
  `(c)-[:MENTIONS]->(e)-[:IN_COMMUNITY]->(k:Community {key})`.
- **`ExpandOptions::within`** (Python `neighborhood(..., within=[ids])`):
  restringe el recorrido de `neighborhood` a un conjunto, como `labels`.
  Un nodo fuera del conjunto no entra ni se expande, y las semillas no se
  filtran. Se comprueba antes de leer el nodo. De paso, el BFS ya no relee
  los nodos que `labels` descartó.
- `impl From<&Neighborhood> for CandidateSet`: una vecindad como universo de
  búsqueda.
- **NQL `in_community(v, key[, via = "T"])`** (#214 fase 2b): buscar dentro
  de una comunidad en una sola condición:
  `where hybrid(c, …) and in_community(c, "leiden/L0/…", via = "MENTIONS")`.
  - Es una restricción de semilla: el top-k se calcula dentro de los chunks
    que mencionan un miembro de la comunidad, o dentro de los miembros si no
    hay `via`.
  - Se intersecta con los demás predicados empujados.
  - Es el mismo universo que `community_members(key, via=…)` e igual, como
    conjunto, al patrón explícito por `IN_COMMUNITY`.
  - Solo vale en el `and` raíz junto a `hybrid`/`similar_to` sobre la misma
    variable. En otro sitio, con una opción desconocida o con una clave
    inexistente, da error con nombre. EXPLAIN la muestra en `Search universe`.
- La receta de [GRAPHRAG.md](docs/GRAPHRAG.md) gana el paso "5. Search
  inside a community", que corre en CI como los otros cuatro.

### Changed
- **`ExpandOptions` tiene un campo nuevo, `within`.** El código que la
  construye con `..Default::default()` (la forma de la documentación) no
  cambia. Una construcción que nombre todos los campos sin `..Default` deja
  de compilar.

## [0.8.5] - 2026-10-24

`import_parquet` deja de perder filas y de pisar nodos existentes (#275), y
se publican los contratos de semántica de NQL (#274). Sin cambios de API.

### Fixed
- **`import_parquet` perdía filas y pisaba nodos existentes.**
  - Leía solo el primer lote del archivo: 1 024 filas, el tamaño de lote del
    lector. El resto se perdía sin error. `read_parquet` tenía el mismo
    problema; ahora lee y concatena todos los lotes.
  - Escribía fila por fila fuera de una transacción, así que una fila mala
    dejaba escritas las anteriores. Además se saltaba el WAL y la historia
    MVCC.
  - Un `id` ya existente se reemplazaba por un nodo sin propiedades.

  Ahora valida el archivo entero antes de escribir: solo se aceptan las
  columnas `id`, `label` y `property_count`, sin nulos, con UUID válidos, sin
  repetidos y sin ids que ya existan. Escribe todo en una transacción, o
  nada. Un archivo sin filas no importa nada y no da error. La documentación
  dice ahora que `export_parquet` e `import_parquet` llevan solo metadatos
  (id y etiqueta), no un grafo completo.

### Added
- **Contratos de NQL** (`docs/NQL_SEMANTICS.md` y `docs/es/SEMANTICA_NQL.md`):
  qué significa una consulta, independientemente de cómo se ejecute:
  - las filas son vinculaciones, una por camino (multiconjunto);
  - una búsqueda calcula su top-k dentro de un universo. Las restricciones de
    semilla lo definen; las de expansión filtran filas y nunca cambian las
    semillas;
  - el orden de evaluación, con `limit` después de `distinct`;
  - un patrón significa lo mismo en lectura y en escritura;
  - las propiedades que toda optimización preserva: seguridad del filtro,
    invariancia del empuje y de la poda, exactitud cuando se promete,
    simetría de la dirección y nada en silencio;
  - el comportamiento actual de las comparaciones (propiedad ausente,
    `null`, `3` frente a `3.0`).

  Cada contrato cita los tests que lo comprueban, y
  `scripts/check_semantics_tests.py` (en CI y en `make check-doc-links`)
  falla si alguno de esos tests desaparece.

## [0.8.4] - 2026-10-23

Arreglos de patrones NQL y poda al expandir (#214 fase 2b, #271). **DELETE
y UPDATE con patrón podían borrar o actualizar aristas que el patrón no
describía** (dirección o propiedades inline ignoradas). Si los usaste con
`<-` o con `{prop: valor}` en la arista, revisa los datos.

### Fixed
- **NQL: DELETE y UPDATE con patrón escribían de más.** Usaban solo las
  etiquetas y el tipo de la arista. Ignoraban la dirección, las propiedades
  inline del nodo origen, del destino y de la arista, y cualquier salto
  después del primero.
  - `delete (c:Chunk)<-[:MENTIONS]-(e:Entity)`, sin ninguna arista en ese
    sentido, **borraba las aristas `c -> e`**.
  - `delete …-[:MENTIONS {w: 1}]->…` borraba también las de `w = 2`.
  - Un patrón de dos saltos se truncaba al primero.

  Ahora se respeta el patrón completo, y un patrón de más de un salto o con
  cuantificador da error.
- **NQL: un patrón de un salto ignoraba la dirección.**
  `(e:Entity)<-[:MENTIONS]-(c:Chunk)` no devolvía nada y
  `(c:Chunk)<-[:MENTIONS]-(e:Entity)` devolvía filas. Ahora `<-` y `<->`
  recorren las aristas entrantes, o ambas, como ya hacían los patrones de
  varios saltos.
- **NQL: un patrón de dos o más saltos ignoraba las propiedades inline de la
  arista.** `-[:MENTIONS {w: 1}]->` encontraba también `w = 2`. Un salto y los
  cuantificadores ya las comprobaban.
- **NQL: `distinct` con `limit` sin `order by` podía devolver menos filas.**
  El límite se aplicaba al leer, antes de quitar duplicados:
  `find distinct r.region … limit 3` daba 1 fila. Pasaba en consultas de un
  nodo y de un salto. Ahora el límite cuenta filas distintas.

### Changed
- **NQL: los predicados sobre los nodos expandidos se aplican al expandir**
  (#214 fase 2b). En un patrón de varios saltos, un predicado del `where`
  que habla de una sola variable descarta ese nodo antes de expandirlo:
  - solo comparaciones con literales, unidas por `and`/`or`;
  - `e.kind = "lugar"`, `f.x < 3 or f.x = 9`;
  - `e.id = …` / `e.id in […]` se comprueba sin leer el nodo.

  Cada nodo alcanzado desde varias semillas o caminos se lee una vez. **Las
  filas no cambian:** el `where` completo se sigue aplicando a cada fila, y
  `k` sigue contando semillas. EXPLAIN lo muestra en la línea
  `Expansion filters`. Una suite de propiedades
  (`nql_pattern_properties_test`) lo comprueba sobre grafos al azar:
  - la simetría de la dirección;
  - dos saltos con predicados frente a fuerza bruta;
  - la partición del `where`;
  - DELETE con patrón frente a FIND.

## [0.8.3] - 2026-10-23

Arreglos de NQL (#255): los números negativos perdían el signo en `where`,
`set` y `delete`, los paréntesis no funcionaban, y la aritmética y la
negación se ignoraban en silencio. **Si escribiste valores negativos con
`set`, revisa los datos: se guardó el valor positivo.**

### Fixed
- **NQL: un número negativo perdía el signo** en `where`, `set` y
  `delete`, desde la primera versión pública. El `-` se leía como menos
  unario y el constructor del AST lo descartaba. Ejemplos:
  - `where m.temp = -5` encontraba los nodos con `temp = 5`.
  - `where m.temp < -1` encontraba también los que tienen 0.
  - `update … set m.temp = -7` **escribía 7**.
  - `delete … where x = -5` borraba los nodos con `x = 5`.

  Las listas (`in [-5]`) y los mapas de propiedades no estaban afectados.
  **Quien haya escrito valores negativos con `set` tiene guardado el valor
  positivo.** Ahora `-5` y `-2.5` son literales con signo en todas partes.
- **NQL: paréntesis en las expresiones** (#255). `where a and (b or c)`
  fallaba con `Invalid primary expression`; ahora agrupa. `and` sigue ligando
  más que `or`.
- **NQL: operadores que se ignoraban en silencio dan error.** La gramática
  aceptaba `+ - * / %`, `!` y `-expresión`, pero el constructor se quedaba
  con el primer operando: `where r.p * 2 = 6` se ejecutaba como `r.p = 6`, y
  `find r.p * 2` devolvía `r.p`. `not (…)` se leía como una función
  desconocida que dejaba pasar todas las filas, aunque la referencia lo
  documentaba como operador. Ahora todos dan un error de parseo que los
  nombra; la referencia ya no promete `not`. `!hybrid(…)` también da error:
  antes se aceptaba como una búsqueda positiva.

## [0.8.2] - 2026-10-23

La búsqueda híbrida con filtro puntúa exacto conjuntos más grandes (#254):
el corte crece con el índice. Sin cambios de API; la única función nueva
es `embeddings::exact_over_allowed_limit`.

### Changed
- **La búsqueda híbrida puntúa exacto conjuntos permitidos más grandes**
  (#254). El corte entre el camino exacto (coseno sobre los vectores
  permitidos, recall = 1) y el recorrido HNSW filtrado era 1024 fijo,
  medido en 0.5.5, cuando el exacto leía cada vector del storage. Desde
  0.6.16 los lee del índice en memoria, y el corte pasa a crecer con el
  índice: `100 × ∛N`, nunca menos de 1024 (2 714 a 20k vectores, 4 642 a
  100k, 10 000 a 1M).
  - Se eligió midiendo los dos caminos a 3k–1M vectores y 64, 384 y 1024
    dims (`benches/exact_threshold.rs`). El corte medido crece más despacio
    que √N, y `100 × ∛N` queda por debajo en todos los casos: donde se elige
    el exacto, también es el más barato.
  - `search_hybrid(label, props)` sobre 100k chunks con una propiedad del 2 %
    pasa de 34–36 ms a 8,4–9,4 ms, y su rama vectorial ahora es exacta
    (`vector_path = "exact_over_allowed"`, `vector_exact = True`). Ninguna
    otra selectividad cambia (`benches/hybrid_props_band.rs`).
  - `EXACT_SEARCH_THRESHOLD` sigue en 1024. Decide cuándo el índice entero
    se lee por scan lineal y cuándo se persiste su volcado, y eso no cambia.
  - El top-k exacto (`rank_exact`) selecciona antes de ordenar, en lugar de
    ordenar todo el conjunto. Da el mismo resultado.

## [0.8.1] - 2026-10-22

Limpieza de dependencias y código muerto, sin cambios de API: lo que se
retirará se marca como deprecado para 0.9. El árbol de dependencias por
defecto baja de 71 a 40 crates; los tiempos de compilación no cambian de
forma medible.

### Removed
- **Dependencias sin uso o sustituibles.**
  - Sin uso: `bytes`, `lazy_static` y `rayon` (la paralelización está dentro
    de hnsw_rs).
  - Sustituidas por std: `futures`, que solo aportaba el alias `BoxFuture`,
    y `async-recursion`, que se cambia por `Box::pin`.
  - Pasan a dev-dependencies: `anyhow` (solo la usan tests) y `rand`, ahora
    0.9 en lugar de la 0.8 duplicada; el easter egg elige con un UUID v4.
  - `bytemuck` solo se compila con `ml`.
  - Tokio pierde la feature `fs`: los tres usos (`import_owl_file`,
    `export_owl_file`, `validate_shapes_file`) pasan a `std::fs` dentro de
    `spawn_blocking`.
  - Dev-dependencies sin uso: `tokio-test`, `num_cpus`.
  - El bloque `[workspace.dependencies]` raíz, que nadie heredaba.
- **Features de terceros que no se usaban.** arrow sin sus lectores CSV y
  JSON (queda IPC). parquet lee y escribe Snappy, zstd y gzip; deja de leer
  archivos comprimidos con brotli o lz4, que `import_parquet` rechaza con un
  error del lector.
- **Código muerto.**
  - Los constructores de claves v1 que la migración de layout nunca usó.
  - Campos sin lectura en `Transaction` y `WalManager`.
  - `src/main.rs` ("Hello, world" que Cargo no compilaba).
  - El paquete huérfano `python/nopaldb/` de la raíz.
  - Los ejemplos de depuración `debug_index_manager`, `test_index_simple` y
    `test_index_usage`, y `ml_demo_pyg.py`, que usaba una API inexistente.
  - Las ramas de selección de motor que ya no podían ejecutarse con redb
    como único motor. La receta de migración de sled se conserva.

### Fixed
- **`--features embeddings` sin `embeddings-index` no compilaba**: una
  reexportación de `embeddings::index` no tenía gate. La CI ahora compila esa
  combinación y los ejemplos.
- **La guía de migración de 0.6** mandaba a `cargo install nopaldb --features
  cli`, que hoy instala una versión que no lee sled. Ahora fija `0.7.*`.
- `docs/FEATURE_TIERS.md` describía `hypergraph` como hiperaristas
  funcionales; es un tipo experimental aún no conectado a las aristas.

### Deprecated
Se retiran en 0.9:
- `Graph::update_node_with_tx`: no hace nada.
- `Graph::delete_node_with_tx` y `Graph::add_node_with_label_and_props`:
  ignoran la transacción y escriben directo.
- `mvcc::VersionManager`: placeholder vacío.

## [0.8.0] - 2026-10-21

Se retira sled (#141): redb es el único motor. Sube la versión menor porque
una base sled de 0.5.x ya no se abre con esta versión; se migra una vez con
0.7. Para bases redb no cambia nada.

### Removed
- **sled, el motor de almacenamiento de 0.5.x** (#141). redb es el único
  motor: se van la feature `storage-sled`, el adaptador `kv/sled.rs`, la
  dependencia `sled` 0.34 (congelada desde 2021, con transitivas sin
  mantenimiento), sled de las wheels de PyPI, los pasos de sled de la CI y
  del nightly, y los ejemplos `migrate_engine` y `migrate_json_to_bincode`.
  Una base sled se sigue **reconociendo** por sus archivos (`conf` + `db`):
  abrirla con cualquier motor, pedir `engine = Sled` / `engine="sled"` o
  usarla de origen en `nopaldb migrate` da un error con la receta, sin crear
  una base redb vacía al lado. La receta: migrar una vez con 0.7, la última
  versión que lee sled (`pip install "nopaldb==0.7.*"` y `Graph.migrate`, o
  `cargo install nopaldb --version "0.7.*" --features cli` y `nopaldb
  migrate`). La variante `StorageEngine::Sled` se conserva solo para dar ese
  error a quien la nombre. El plan de #141 era anunciarlo una versión menor
  antes; sin señales de uso de sled desde 0.6.0 (ningún reporte ni issue de
  usuarios) se retira directamente y 0.7 queda como puente de migración.
  `Graph.migrate` / `nopaldb migrate` siguen como copia verificada entre
  bases redb (cambio de perfil, copia en frío).

### Fixed
- **Las descripciones publicadas en PyPI y crates.io describían sled como
  motor por defecto** y redb como experimental desde 0.6.0. Igual
  `docs/python/CONFIGURATION.md` y su versión en español, que describían un
  árbol LSM con compresión (era sled). Corregidas.

## [0.7.0] - 2026-10-21

El `where` de NQL es el universo de `hybrid` / `similar_to` (#214 fase 2a).
Sube la versión menor porque cambia lo que devuelven consultas existentes:
con predicados sobre la variable buscada, `k` es ahora el top-k dentro de
ellos, y `rank`/`score` se calculan en ese universo. Sin cambios de API.

### Changed
- **NQL: los predicados del `where` sobre la variable buscada son el universo
  de `hybrid` / `similar_to`** (#214 fase 2). `where hybrid(c, …, k = 10) and
  c.tenant = "t7"` devuelve ahora los 10 mejores del tenant; hasta 0.6.20
  puntuaba la etiqueta entera y descartaba los demás tenants después, así que
  podía devolver menos de `k` filas o ninguna (medido a 100k chunks: 0 filas
  para un tenant del 1 %). Se empujan `v.p = literal` y `v.p in [literales]`
  con escalares no nulos, `v.id = …` / `v.id in [...]` y las props inline del
  patrón; también en patrones de uno y dos saltos, sobre la semilla. El resto
  del `where` (rangos, `<>`, `null`, listas, funciones, otras variables) se
  sigue aplicando a las filas, y el `where` entero se re-aplica siempre:
  empujar nunca admite una fila que no lo cumpla. **Cambia lo que devuelve
  una consulta existente con esos predicados**: más filas (hasta `k`), y
  `rank(c)` / `score(c)` calculados en el universo reducido, igual que
  `search_hybrid(props=...)` en Python; quien corta por umbral de `score(c)`
  con `hybrid` verá más filas (el coseno de `similar_to` no cambia). EXPLAIN
  gana la línea `Search universe:` con los predicados empujados. **Coste:**
  la consulta paga lo que cuesta la respuesta completa, lo mismo que
  `search_hybrid(props=...)`: a 100k chunks, de 0.3 ms (predicado del
  0.02 %) a ~2–4 ms (predicados amplios) y ~12–45 ms en la franja del 1–10 %,
  contra ~1.7 ms de la respuesta incompleta de antes. Sin predicados
  empujables no cambia nada.
- **Planificador de filtros de `search_hybrid` con propiedades: comprobado
  primero.** Un filtro con propiedades se comprueba sobre los candidatos de
  cada rama en orden de rango, como ya se hacía con la etiqueta sola, y la
  tasa de aciertos de las primeras rondas decide si construir el conjunto
  (filtro selectivo: índice de propiedades, exacto si es chico) o seguir así
  (filtro amplio). Una propiedad con a lo sumo 512 miembros lo sabe por una
  lectura acotada del índice (~0.04 ms) y construye el conjunto de inmediato,
  como antes. Hasta 0.6.20 todo filtro con propiedades construía el
  conjunto, leyendo e hidratando cada miembro. Medido a 100k chunks, k = 10:
  propiedad del 90 % 767 → 2.3 ms, 50 % 256 → 3.9 ms, 10 % 66 → 13–22 ms;
  del 2 % al 0.02 % sin cambio (40, 14, 4, 0.27 ms). Un filtro de solo
  etiqueta no cambia. Los nombres de camino (`label_checked`,
  `global_label_checked`) se conservan y ahora cubren también ese caso.

### Fixed
- **NQL: una búsqueda bajo `or` perdía la otra rama en silencio.**
  `where hybrid(c, …) or c.tenant = "B"` devolvía solo los hits de la
  búsqueda. Ahora la búsqueda tiene que ser una condición del `and` raíz;
  bajo `or`, una segunda búsqueda del mismo tipo (que no restringía nada) o
  una búsqueda dentro de otra función son errores con nombre.
- **`search_hybrid(props=...)` con un valor lista u objeto devolvía 0
  resultados**: el índice de propiedades no indexa esos valores y su lookup
  vacío vaciaba el conjunto. Ahora esos valores se comprueban sobre los nodos.
- **NQL `update … set`: orden seguro del índice de propiedades.** Retiraba la
  entrada vieja antes de escribir el nodo y tragaba los errores; un fallo a
  medias dejaba el nodo sin entrada, invisible para los filtros por
  propiedad. Ahora añade la nueva, escribe el nodo y retira la vieja, con los
  errores propagados: lo peor que deja un corte es una entrada de más, que
  se descarta al confirmar sobre el nodo.

## [0.6.20] - 2026-10-20

`close()` espera los merges del índice full-text, y GraphRAG tiene receta de
punta a punta respaldada por un sample.

### Fixed
- **`close()` retornaba con hilos de merge del índice full-text todavía
  escribiendo.** `Graph::close` publicaba lo pendiente del índice (commit
  de tantivy) pero no esperaba los merges que ese commit arranca en hilos
  propios, así que seguían apareciendo segmentos en el directorio del índice
  después de "cerrado"; quien borraba o movía el directorio justo después
  chocaba con ellos (en Python, `TemporaryDirectory` fallaba con "Directory
  not empty" en 4 de 6 corridas de un sample con varios lotes). Ahora
  `Index::close` (nuevo en el trait, por defecto = `flush`) consume el writer
  con `wait_merging_threads` y `IndexManager::close` lo corre sobre todos los
  índices al cerrar: cuando `close()` retorna nada del índice toca el disco.
  Test: tras doce commits y `close`, el listado del directorio no cambia.

### Changed
- **`docs/GRAPHRAG.md`: receta de punta a punta respaldada por
  `graphrag_sample.py`** (#230). Sección nueva "Recipe: end to end" en
  cuatro bloques con la API real: índices (`create_index` full-text con
  analyzer y hash), ingesta idempotente de chunks + entidades + relaciones
  en una llamada (`upsert_many` con `vector`/`model`/`links` y stubs),
  recuperación híbrida que siembra el patrón con `rank`/`vector_score`/
  `text_score`/`score` en cada fila y `candidates=` cuando el universo lo
  fija el caller, y multi-hop: dos saltos en NQL (0.6.19) o `neighborhood`
  como conjunto con `rank="ppr"` y presupuesto de contexto. El código de la
  doc **es** el del sample (`graphrag_sample.py` § recipe, en `PY_SAMPLES`):
  `scripts/check_graphrag_recipe.py` compara los bloques y falla si divergen
  (`make check-doc-links` y la CI). El cheat sheet de Python trae la forma
  exacta de `links` y de las filas de `upsert_many`; el README y el índice
  de docs enlazan la receta. Verificado tal cual contra la wheel 0.6.19 de
  PyPI.

## [0.6.19] - 2026-10-20

NQL siembra el multi-hop de GraphRAG en una consulta.

### Added
- **NQL: `hybrid` / `similar_to` siembran cadenas de dos saltos** (#231).
  `(a)-[:T1]->(b)-[:T2]->(c)` con `a` buscada: los candidatos de la búsqueda
  son la fuente del patrón, como en un salto, y `score(a)`, `rank(a)` y los
  scores crudos viajan con cada fila expandida; `k` acota las semillas y
  `LIMIT` las filas; el resto del `where` se aplica tras expandir. Es el
  "chunk → entidad → entidad relacionada" de GraphRAG, que hasta 0.6.18
  obligaba a salir a Python (`search_hybrid` + `neighborhood`) y perder el
  score, el `limit` y el EXPLAIN de la consulta. Tres saltos, ramas y
  cuantificadores siguen rechazados con nombre (`longer chains`,
  `quantifiers`, `several patterns`). EXPLAIN dice `PATTERN PIPELINE (seed:
  HYBRID, hops: 2)` y una línea `Seeds: k = N … LIMIT (…) applies to the
  expanded rows`. De paso, `where a.id = …` también siembra cadenas de
  cualquier longitud (antes solo un salto). Sin agregaciones en dos saltos,
  como en todo el camino lineal. Fila `nql/hybrid_two_hops` en el bench
  `retrieval`: 1.35 ms a 100k chunks (un salto 1.11 ms en la misma corrida);
  docs en `NQL_REFERENCE` EN/ES, `HYBRID_SEARCH.md` y
  `GRAPHRAG.md`. La fase 2 de #214 convertirá los predicados de `b`/`c` en
  conjunto de candidatos; hasta entonces se aplican tras expandir (la
  semántica "A" de expansión).

## [0.6.18] - 2026-10-20

SHACL reporta cada valor que falla y comprueba dominio y rango de una
propiedad; el razonador detecta clases insatisfacibles y dice por qué.

### Added
- **SHACL: `sh:targetSubjectsOf` y `sh:targetObjectsOf`** (#237). La forma
  estándar de expresar el dominio y el rango de una propiedad ("todo el que
  tenga `:usa` debe ser `:Receta`"; "todo lo que sea objeto de `:usa` debe
  ser `:Ingrediente`"). `targetSubjectsOf p` toma los nodos con una arista
  de tipo `p` o una propiedad `p` (la regla de "ambos" de `sh:path`), así que
  ve justo a los sujetos que `sh:targetClass` no ve: los que no tienen el
  tipo esperado, incluidos los que no tienen ninguno. `targetObjectsOf p`
  toma los destinos de las aristas de tipo `p`; los literales no son focus
  nodes. Varios targets en una shape validan la unión, cada nodo una vez.
  Hasta 0.6.17 ambos salían en `ignored` y la shape no validaba nada (una
  shape con solo uno de ellos ni siquiera se reconocía). `Target` gana las
  variantes `SubjectsOf` y `ObjectsOf` (enum pública sin
  `#[non_exhaustive]`: un `match` exhaustivo externo necesita los brazos
  nuevos). El predicado se resuelve como en `sh:path`, y ambos conocen ahora
  los términos que el import renombra (`rdf:type` → `instanceOf`,
  `rdfs:subClassOf` → `subClassOf`, `rdfs:label`/`rdfs:comment` →
  `rdfs_label`/`rdfs_comment`): `sh:targetSubjectsOf rdf:type` es todo nodo
  tipado, en vez de casar con nada en silencio. La lectura de nodos y
  aristas que estos targets y `sh:targetClass` necesitan se hace una vez por
  validación, no una por shape. Docs en `SHACL.md` EN/ES.

- **ELReasoner: clases disjuntas y ⊥ (`owl:Nothing`)** (#235). ⊥ viene
  incorporada (`OWL_NOTHING`, un id fijo: el UUID v5 de su IRI;
  `ELReasoner.NOTHING` en Python). `assert_disjoint([…])` declara clases
  disjuntas dos a dos (`ci ⊓ cj ⊑ ⊥` por par, sigue dentro de EL++);
  `Axiom::DisjointClasses` y `assert_axiom` para asertar cualquier axioma.
  Regla nueva **CR4**: `A ⊑ ∃R.B` con `B` insatisfacible ⇒ `A` insatisfacible.
  Consultas: `unsatisfiable_classes()`, `is_satisfiable(id)` y
  `why_unsatisfiable(id)`, que dice la causa (el par disjunto, un ⊥ asertado,
  la herencia de uno, o el existencial) sin recorrer la derivación; las tres
  clasifican antes, así que la respuesta es completa. Python:
  `assert_disjoint`, `is_satisfiable`, `unsatisfiable_classes`,
  `why_unsatisfiable` (dict) y **`ELReasoner.from_graph(graph)`**, que arma el
  razonador con la jerarquía y las disyunciones del grafo.
  `reasoner_sample.py` en CI.
- **`import_turtle` importa `owl:disjointWith` y `owl:AllDisjointClasses`**
  (#235) como aristas `disjointWith` entre los nodos clase, una por par no
  ordenado (escrito en ambos sentidos o repetido, es una; cuatro miembros son
  seis). `ImportReport.disjoint_pairs_added` las cuenta y ya no suman a
  `triples_skipped` (antes se perdían sin aviso). `ELReasoner::from_graph_at`
  las lee; `export_turtle` las escribe como `owl:disjointWith`.
  `:A rdfs:subClassOf owl:Nothing` ya no crea un nodo clase `Nothing`: sale en
  `warnings` (⊥ no es un nodo).

### Fixed
- **SHACL: una violación por cada valor que no cumple** (#236). Las
  restricciones de valor (`sh:datatype`, los cuatro rangos, `sh:minLength`/
  `sh:maxLength`, `sh:pattern`, `sh:in`, `sh:nodeKind`, `sh:class`) y los
  combinadores (`sh:and`, `sh:or`, `sh:xone`, `sh:not`, `sh:node`) se
  detenían en el primer valor malo de cada nodo y path: con dos hechos de
  rango incorrecto, el reporte señalaba uno y el otro parecía válido. Ahora
  dan una violación por valor, cada una con su `value` y sus `nested`, como
  pide SHACL §4. `sh:minCount`, `sh:maxCount` y `sh:hasValue` juzgan el
  conjunto y siguen dando una. `conforms` no cambia en ningún caso; quien
  contaba violaciones verá más, ahora correctas. `evaluate_constraint`
  devuelve `Vec` y `first_bad` pasa a `bad_values`; docs en `SHACL.md` EN/ES
  § "The report".
- **`ELReasoner::classify_all` llega al punto fijo** (#235). Era una pasada
  en orden de reglas y CR2 solo miraba la jerarquía asertada: una subsunción
  derivada por CR3 nunca alimentaba una conjunción (`A ⊑ Receta` derivada,
  `A ⊑ Utensilio` asertada, `Receta ⊓ Utensilio ⊑ ⊥` ⇒ `A ⊑ ⊥` no salía).
  Ahora es completación EL con conjuntos de subsumidores y cola de trabajo:
  cada regla ve lo que derivan las demás, `A ⊑ ∃R.B` se hereda a las
  subclases de `A`, y se clasifican también las clases que solo aparecen en
  axiomas (antes, solo las registradas con etiqueta). Una cadena de 1 000
  clases (~500 000 pares) clasifica en ~0.1 s en debug.
- `ELReasoner::derived_inferences()` dice la regla que derivó cada par (antes
  todas salían como CR1); `superclasses()`/`subclasses()` incluyen lo
  derivado, como decía su doc.

`Axiom` y `CompletionRule` ganan variantes (`DisjointClasses`, `CR4`): son
enums públicas sin `#[non_exhaustive]`, así que un `match` exhaustivo externo
necesita los brazos nuevos. `Unsatisfiable` nace `#[non_exhaustive]`.

## [0.6.17] - 2026-10-13

NQL dice de dónde sale cada score y los asistentes de código encuentran los
nombres reales de la API.

### Added
- **`rank(var)`, `text_score(var)` y `vector_score(var)` en el FIND de NQL**
  (#228). Las piezas crudas que `score(var)` resume: la posición 0-based en
  la búsqueda, el BM25 de la rama de texto y la similitud coseno de la rama
  vectorial (`null` donde esa rama no puntuó el nodo). Mismas reglas que
  `score`: solo columna del FIND, sobre la variable buscada, sin
  agregaciones; errores con el nombre real. EXPLAIN nombra la fuente de cada
  una. Con `hybrid` los valores son los de `search_hybrid_explain`; con
  `similar_to`, `vector_score = score` y `text_score = null`. Motivo: el
  score de `hybrid` es RRF (`Σ 1/(rrf_k + rango)`, como mucho `2/rrf_k`,
  0.033 por defecto) y un umbral pensado para coseno lo descartaba todo en
  silencio; la referencia EN/ES y `GRAPHRAG.md` traen ahora la tabla de
  escalas y el umbral correcto para cada caso. `nql_scores_sample.py` en CI.
- **Cheat sheet de la API Python y `llms.txt`** (#229).
  `docs/python/API_CHEATSHEET.md`: una línea por método con la firma exacta
  del stub, y la lista de nombres que NO existen con su equivalente
  (`execute` → `execute_nql`, `upsert_node` → `upsert`,
  `create_embedding_index` → nada que llamar). `llms.txt` en la raíz para los
  asistentes. Guardia `check_cheatsheet.py` en `make check-python`: falla si
  un nombre del cheat sheet deja de existir o si uno de los "no existen"
  aparece.

## [0.6.16] - 2026-10-13

La rama vectorial de `search_hybrid` con un conjunto chico (filtro selectivo
o `candidates=`) pasa de milisegundos a microsegundos: lee los vectores del
índice en memoria en vez de almacenamiento, y abrir el keyspace de
embeddings ya no cuesta una transacción de escritura por consulta.

### Performance
- **El camino exacto del vector lee del índice, no de almacenamiento**
  (#224). Con un conjunto permitido de hasta 1024 nodos sobre un índice
  mayor (`vector_path = exact_over_allowed`), `search_hybrid` cargaba cada
  vector del conjunto desde storage y lo puntuaba. El índice HNSW ya tiene
  esos vectores en memoria, pero `hnsw_rs` solo los sirve por posición
  `(capa, puesto)`, no por id; ahora `HnswIndex` guarda esa posición por
  `DataId` (`Vec<PointId>`, 8 bytes por punto, no se persiste: se
  reconstruye en una pasada al cargar) y `vectors_for` resuelve id → vector
  en O(1). Mismos ids, mismas distancias (los mismos f32), mismo desempate.
  Se confía en el índice, sin fallback por id a storage: todos los escritores
  de embeddings lo actualizan en la misma llamada y la rama `hnsw_filtered`
  ya leía solo del índice; un fallback reintroduciría una lectura de storage
  por cada candidato que simplemente no tiene embedding. Se retira el filtro
  por dimensión del camino exacto como código inalcanzable (el índice
  rechaza al insertar cualquier vector de otra dimensión).
- **El keyspace de embeddings se cachea al abrir** como los otros once
  (#224). Se pedía bajo demanda en cada operación de embeddings, y en redb
  eso era una transacción de escritura (commit incluido) más la
  invalidación de las tablas de lectura cacheadas de TODOS los keyspaces:
  4.6 ms por consulta a 100k chunks, y la hidratación posterior de 10 nodos
  pasaba de 17 µs a 5.7 ms. Afecta a `search_hybrid`, a `get_node_embedding`
  y a `has_embedding(...)` en NQL, que lo pagaba por fila. Una base abierta
  de solo lectura deja de escribir al consultar embeddings.
- **`KvKeyspace::get_many`** (#224): varias claves bajo una sola tabla de
  lectura y una sola consulta al overlay en redb (default que itera `get`
  en sled); lo usa la carga de embeddings por conjunto. Test de conformance
  en ambos motores.

  Medido (`retrieval`, 100k chunks, 64 dims, release, redb, conjunto de 65
  ids; filas nuevas `embeddings_read/*` y `hydrate/get_nodes_10_after_embeddings_read`):
  solo vector 3.5 ms → 13 µs; texto+vector 3.0 → 0.19 ms; texto+vector ∩
  tenant 4.5 → 0.80 ms; filtro `doc` (20 chunks) 2.0 → 0.20 ms; sin conjunto
  ni filtro 0.91 → 0.82 ms; `exists_sync` 4.6 ms → 0.22 µs; leer 65
  embeddings 4.8 ms → 40 µs; hidratar 10 nodos tras leer un embedding
  5.7 ms → 18 µs. En sled abrir el keyspace ya era barato y la mejora es la
  del camino exacto: solo vector 54 → 13 µs; texto+vector 235 → 182 µs.

### Changed
- `search_hybrid_explain`: la doc de `candidate_set.size` precisa que, con
  `candidates` y sin filtro, cuenta ids distintos tal cual se pasaron, no
  nodos confirmados (sin filtro los ids no se leen); solo un filtro
  confirma pertenencia. Observado por un integrador con un id inexistente.

## [0.6.15] - 2026-10-12

`search_hybrid` acepta el universo de la búsqueda como conjunto explícito de
candidatos (`candidates`): el caller que ya sabe qué nodos son elegibles (un
documento, una ACL, tenant ∩ caso ∩ documentos autorizados) obtiene el top-k
calculado dentro de ese conjunto, con los mismos scores, y el filtro solo
puede reducirlo. Es la fase 1 de #214.

### Added
- **`CandidateSet` y `HybridQuery.candidates`** (#214, fase 1). El universo
  sobre el que corre la búsqueda híbrida: `All` (el grafo, o lo que deje el
  filtro) o `Ids` (un conjunto explícito del caller, de representación
  privada). No es otro filtro: cambia el universo puntuable, nunca el score
  (`top-k(Q, C)`, no `top-k(Q, D) ∩ C`), y `{label, props}` solo lo
  mantiene o lo reduce, nunca lo amplía, así que sirve como ACL. El
  conjunto entra como semilla del mismo pipeline que construye el conjunto
  permitido (índice de propiedades ∩ ids, etiqueta confirmada sobre los
  supervivientes), y ambas ramas puntúan dentro de él por los caminos de
  #213 y 0.5.5. Motiva el caso de un integrador: 65 fragmentos del mismo
  tenant/caso, el único del documento seleccionado detrás de 64 distractores
  más cercanos; pedir 32 dentro del filtro y quedarse con el documento
  después devolvía 0, como conjunto de candidatos devuelve 1 con rango en
  las dos ramas. Python: `search_hybrid(..., candidates=[ids])` y
  `search_hybrid_explain`. Álgebra mínima (`is_all`, `len`, `contains`,
  `intersect` con `All` identidad y ∅ absorbente). Tests:
  `tests/candidate_set_test.rs` (caso 1-entre-64, nunca-ampliar, scores
  idénticos, y una propiedad con `proptest`: todo resultado ∈ C y
  `|resultados| = min(k, |C|)`). Medido (`retrieval`, 100k chunks, release,
  conjunto de 65 ids): solo texto 0.16 ms, solo vector 2.2 ms, texto+vector
  3.6 ms, texto+vector ∩ `{label, props}` 4.9 ms; sin conjunto ni filtro
  0.96 ms (sin cambio). El coste va a la rama vectorial: el camino exacto
  lee cada vector del conjunto de almacenamiento (~30 µs por id), un coste
  de 0.5.5 anterior a esta fase y candidato a optimizar leyéndolos del
  índice en memoria.
- **`candidate_set` en `search_hybrid_explain`** (#214): `{source, caller_size,
  size}` con `source` ∈ `unrestricted` | `filter` | `ids` | `ids_and_filter`.
  Distingue "quedó fuera al construir el universo" (`size < caller_size`)
  de "lo superó el ranking". `allowed_set_size` se conserva con el mismo
  valor que `size`. En Rust `HybridExplain::candidate_set`
  (`CandidateSetReport`, `CandidateSource`).

### Changed
- Con `candidates`, un filtro de solo etiqueta no toma el atajo sin
  conjunto: la etiqueta se confirma sobre los ids del caller (coste
  proporcional al conjunto). Sin `candidates` nada cambia.

## [0.6.14] - 2026-10-12

`search_hybrid_explain` dice si la rama vectorial fue exacta (`vector_exact`),
y ya no informa `hnsw_filtered` cuando un filtro sobre un índice chico se
resolvió con un scan exacto.

### Fixed
- **`vector_path` informaba `hnsw_filtered` en índices de hasta 1024 puntos
  aunque el camino era exacto** (#218). Con un filtro por propiedades y un
  índice entero bajo el umbral exacto, `search_hybrid` deja que el índice
  resuelva el filtro, y el índice puntúa todos los puntos y aplica el
  predicado, sin recorrer el grafo. Pero el híbrido etiquetaba el resultado
  como recorrido HNSW sin mirar qué camino había corrido, así que un caller
  que se abstiene con fundamento leía un `underfilled` definitivo como
  "quizá hay más" en tests, laboratorios y despliegues chicos. Ahora
  propaga el camino que el índice ya informaba (`exact_index`). Sin cambio
  de resultados ni de coste. Reportado por un integrador con un índice de
  65 vectores.
- **La tabla de explain en `HYBRID_SEARCH.md` listaba `unfiltered` como
  exacto**; lo es solo mientras el índice lee exacto (≤ 1024 puntos), por
  encima es KNN aproximado por HNSW. Corregido con la fila por tamaño.

### Added
- **`vector_exact` en `search_hybrid_explain`** (#218): `True` si la rama
  vectorial tuvo recall = 1, resuelto por camino y tamaño del índice, de
  modo que el caller decide cómo leer un resultado corto sin conocer la
  tabla de caminos; `None` sin rama vectorial. En Rust
  `HybridExplain::vector_exact` y `VectorPath::is_exact(index_exact)`;
  `HnswIndex::is_exact` expone si el índice lee exacto. Nueva variante
  `VectorPath::ExactIndex` (`"exact_index"` en Python).
  `hybrid_filter_sample.py` lo comprueba con el caso 1 permitido + 64
  distractores.

## [0.6.13] - 2026-10-12

Búsqueda híbrida con filtros selectivos. La rama de texto de `search_hybrid`
ya no pierde documentos permitidos detrás del top-1000 global (1 permitido
entre 1024 que contienen el término: 0 → 1 hit), y construir el conjunto
permitido ya no lee la etiqueta entera (100k chunks, filtro que deja 20:
193 → 0.23 ms). `search_hybrid_explain` dice por qué camino fue la rama de
texto (`text_path`).

### Fixed
- **`search_hybrid` ya no pierde documentos permitidos detrás del top-1000
  global** (#213). Con un filtro por propiedades, la rama de texto pedía a
  tantivy el top-N global por BM25 (escalando hasta 1000) y filtraba
  después, tratando el tope como "no hay más": 1 fragmento permitido entre
  1024 que contienen el término devolvía 0 hits con `underfilled = true`,
  y una capa RAG se abstenía con evidencia existente. Es el diseño que la
  rama vectorial descartó en 0.5.5. Ahora la rama responde "top-k del
  texto DENTRO del conjunto permitido": un conjunto de hasta 512 nodos se
  puntúa entero en tantivy sin consulta global (`TermSetQuery` sobre el
  `node_id` almacenado, AND con el texto, boost 0 ⇒ mismo BM25 que el
  camino global); uno mayor prueba el top-N global escalando y, si agota el
  tope corto, se puntúa entero por trozos. La rama es completa a
  cualquier selectividad y `underfilled` vuelve a significar "no hay más
  permitidos". Las consultas que ya llenaban sus candidatos devuelven lo
  mismo (documentos, orden y scores); las que salían cortas salen
  completas, y en texto+vector un documento que solo encontraba la rama
  vectorial lleva ahora también su rango de texto, así que puede subir en
  la fusión. Medido (`text_filter`, 100k documentos que contienen el
  término, release): term set 8 ids 0.09 ms, 128 0.68 ms, 512 2.2 ms, 1000
  3.8 ms; el escalado que sustituye ~2 ms. Nuevo `Index::query_scored_in` /
  `IndexManager::query_scored_in`; `tests/hybrid_selective_filter_test.rs`;
  `hybrid_filter_sample.py` en CI. Reportado por un integrador en 0.6.9 y
  0.6.12.
- **El conjunto permitido de un filtro con propiedades ya no lee la
  etiqueta entera** (#213). `search_hybrid` con `label` + `props` construía
  el conjunto leyendo y deserializando TODOS los nodos de la etiqueta para
  sacar sus ids e intersectarlos con las propiedades: 197 ms por consulta
  con una etiqueta de 100k nodos, para un filtro que deja 20. Ahora parte
  del índice de propiedades (solo ids) y confirma la etiqueta hidratando
  solo los candidatos. Mismo conjunto, mismo resultado. Medido en
  `retrieval` a 100k chunks (release): solo texto con filtro que deja 20
  chunks 193 → 0.23 ms (term set directo), con filtro que deja el 1 %
  (1000 chunks: escalado global y term set por trozos) 207 → 11.4 ms;
  híbrida texto+vector 194 → 2.4 ms y 206 → 17.5 ms. Las consultas sin
  filtro o con solo etiqueta no se mueven (0.60 / 0.87 / 1.05 ms). Con un
  término raro (1 % de los documentos) el camino global es más barato que
  el term set en conjuntos de 300–512 (0.56 vs 2.3 ms): el planificador
  por coste con frecuencia del término queda para #214.

### Added
- **`text_path` en `search_hybrid_explain`** (#213): `global` |
  `global_label_checked` | `term_set_over_allowed` | `global_then_term_set`,
  el equivalente de `vector_path` para la rama de texto (`TextPath` en
  Rust, clave `"text_path"` en Python). Constante pública
  `graph::hybrid::TEXT_EXACT_THRESHOLD` (512). Bench `text_filter` (ambos
  caminos por tamaño del conjunto) y filas `*_props_doc` / `*_props_tenant`
  en `retrieval`.

---

## [0.6.12] - 2026-10-10

Índice de etiquetas y memoria del HNSW. Buscar nodos por etiqueta ya no
recorre la base (100k nodos: una etiqueta de 1 000 nodos 41.7 → 0.64 ms),
y el índice vectorial ocupa la mitad (100k × 384: 650 → 317 MiB) al dejar
de usar una reserva de hnsw_rs mal calculada.

### Added
- **Índice de etiquetas** (#207). Buscar nodos por etiqueta ya no recorre
  la base: keyspace nuevo `label_idx` con una clave `len | label | node_id`
  por nodo. Lo usan `get_nodes_by_label`, `scan_nodes_batch(Some(label))`,
  NQL `from (n:Label)`, el filtro de `search_hybrid`/`hybrid()` con
  `props=` o con etiqueta rara, el fallback de `similar_to`, el alcance
  `labels=` de Leiden (nuevo `GraphView::get_nodes_with_labels`, con
  implementación por defecto), las comunidades y reportes, las
  transacciones, el importador RDF/OWL, SHACL y los exports. A 100k nodos
  (`graph_ops`, `label_lookup_100k`): una etiqueta de 1 000 nodos 41.7 →
  0.64 ms, una de 10 nodos 41.8 → 0.006 ms, NQL `from (n:Rare)` 38.0 →
  0.011 ms.
  - La entrada se escribe ANTES que el nodo (en el mismo lote atómico en
    commits transaccionales y cargas por lote), así que un crash nunca deja
    un nodo sin entrada; las entradas de más que pueda dejar (crash a
    mitad, cambio de etiqueta) las descartan las lecturas, que comprueban
    la etiqueta del nodo. Un upsert que cambia la etiqueta retira la vieja.
  - Al abrir: si falta el índice (base de ≤ 0.6.11) o si el reloj lógico
    pasó su marca de sincronización (`label_idx_synced_ts`, que escribe
    `persist_clocks`: una versión anterior escribió en la base), se
    reconstruye en una pasada; a 100k nodos el open pasa de 73 a 245 ms
    esa vez. Un formato más nuevo que el del build es un error.
  - Coste de escritura: una clave más por nodo; `bulk_load` sin cambio
    medible, `upsert_batch` de 1 000 filas 26.3 → 28.8 ms.
  - Crash harness (redb y sled) comprueba que el índice coincide con los
    datos tras cada recuperación; `tests/label_index_test.rs` lo compara
    con un recorrido tras altas, upserts con cambio de etiqueta, borrados,
    lotes con ids repetidos, transacciones, NQL y reapertura.

### Fixed
- **El índice HNSW ya no reserva memoria que nunca usa** (#206). hnsw_rs
  0.3.4 calcula mal la reserva inicial por capa cuando recibe el número de
  puntos esperado (a la fracción le falta un `exp`): con M = 24 apartaba
  ~3.4 KB por punto, 330 MiB para 100k, antes de insertar nada. `HnswIndex`
  le pasa ahora 0 y las capas crecen al insertar; el argumento
  `max_elements` de `HnswIndex::new` / `with_params` se acepta y se ignora.
  Medido con `retrieval_dims` a 100k: 650 → 317 MiB a 384 dimensiones (4.4×
  → 2.2× los vectores crudos) y 897 → 565 MiB a 1024 (2.3× → 1.45×), con el
  mismo recall y tiempo de build. Un índice reabierto desde su dump no lo
  sufría. `tests/hnsw_memory_test.rs` mide la reserva con un allocator
  contador.

---

## [0.6.11] - 2026-10-09

Iteración GraphRAG B: estructura global. Leiden ponderado, acotado y
jerárquico, con un núcleo reescrito según el paper; comunidades persistidas
con keys estables; reportes de comunidad y receta de búsqueda global; y
cifras del índice vectorial a 384 y 1024 dimensiones. Tres defectos de fondo
corregidos al medir a escala: el índice de propiedades cuadrático con valores
compartidos, el Leiden plano que no agregaba, y el HNSW que dejaba de escalar
con el arreglo de 0.6.10 (100k × 384: p95 167 → 0.87 ms).

### Added
- **Cifras del índice vectorial con embeddings de tamaño real** (#192,
  parte 1). `benches/retrieval_dims.rs` mide `HnswIndex` a 384 y 1024
  dimensiones, 10k y 100k vectores, datos agrupados y uniformes: build,
  huérfanos, memoria del índice, p50/p95 de KNN k = 10 con `ef_search` 30 y
  100, scan exacto y recall@10. Tabla en `docs/GRAPHRAG.md` (Measured).
  A 100k × 1024 con datos agrupados: p95 1.5 ms (ef 30) contra 81 ms del
  scan exacto, recall@10 0.988 / 0.998, 0.8% de huérfanos, 897 MiB.
- **Reportes de comunidad y búsqueda global** (#191). Convención
  `(:Report {community_key, partition, level, title, summary, rating,
  generated_at, source_version})-[:SUMMARIZES]->(:Community)`, un reporte
  por comunidad. `Graph::upsert_community_report(key, CommunityReport)` lo
  escribe con `source_version` = la huella de la comunidad
  (`community_fingerprint`: miembros con sus propiedades y aristas internas
  con las suyas). `Graph::stale_reports(partition, level)` lista las
  comunidades sin reporte (`missing`), con reporte viejo (`stale`) y los
  reportes huérfanos (`orphan`); una arista nueva dentro de una comunidad
  marca esa comunidad y sus ancestros, no sus hermanas. Python:
  `community_fingerprint`, `upsert_community_report`, `stale_reports`.
  Receta de búsqueda global (map-reduce sobre reportes, Python y NQL) en
  `docs/GRAPHRAG.md`, con `global_search_sample.py` en `make check-python`.
- **Comunidades persistidas con keys estables** (#190 c).
  `Graph::materialize_communities(&levels, &CommunityMaterializeOptions)`
  guarda una jerarquía como `(:Community {partition, level, key, size})`,
  `IN_COMMUNITY` (miembro → comunidad, un nivel por arista) y `PARENT_OF`
  (nivel L → L+1). Upsert de estado deseado en una transacción: solo
  escribe la diferencia, y recalcular sin cambios no escribe nada. Una
  comunidad que se solapa con una previa del mismo nivel (Jaccard ≥
  `min_jaccard`, 0.5) hereda su `key` y su `NodeId`; una nueva recibe una
  key derivada de sus miembros. Varias particiones conviven por nombre.
  Python: `graph.materialize_communities(levels, partition=, min_jaccard=)`.
  Con 1% de aristas nuevas se conserva la key de 87–100% de los nodos por
  nivel. Costo a 100k nodos: 44 s la primera vez (dominado por el índice
  de propiedades, #197) y 0.5 s al recalcular sin cambios.
- **Jerarquía de comunidades de Leiden** (#190 b).
  `LeidenCommunity::detect_hierarchy(&graph, &LeidenHierarchyOptions)` →
  `LeidenHierarchy { levels }`, de la más gruesa a la más fina, con
  `depth()` y `parent(level, community)`. El nivel 0 es `detect()`; cada
  nivel siguiente parte con Leiden, sobre su propio subgrafo y con
  `gamma · resolution_factor^L`, las comunidades con más de
  `max_cluster_size` nodos (defaults 10, 2.0 y `max_levels` 8). Contención
  entre niveles por construcción. Python: `graph.leiden_hierarchy(...)` →
  lista de `{node_id: community}`. Cuesta ≈ 1.1–1.4× `detect()` (100k nodos:
  872 ms contra 782 ms). Se descartaron como niveles las particiones
  refinadas internas del núcleo: son fragmentos, no subcomunidades
  (`docs/ALGORITHMS.md`, Hierarchy).
- **Leiden ponderado y acotado** (#190 a). `LeidenConfig` gana `labels`,
  `edge_types` y `weight_property`; NQL `leiden(n, labels = [...],
  edge_types = [...], weight = "w", gamma = g)`; Python
  `graph.leiden(labels=, edge_types=, weight=, gamma=)` → `{node_id:
  comunidad}`. Hasta ahora `leiden(n)` corría sobre el grafo entero (en un
  GraphRAG mezclaba los chunks con las entidades), con peso fijo 1 y sin
  opciones en NQL (gamma solo desde Rust).
  - Con `weight`, el peso de un par es la suma de sus aristas (las que no
    tienen la propiedad pesan 1); pesos negativos o no finitos son error.
    Sin `weight`, el comportamiento de siempre.
  - La partición no depende del orden de inserción: los vecinos de Leiden
    van en `BTreeMap` y se suman en orden de `NodeId`.
  - La caché de Leiden lleva las opciones en la clave: consultas seguidas
    con opciones distintas no se devuelven la partición de la otra. Una
    configuración por consulta en NQL (error con nombre si hay dos).
  - EXPLAIN muestra la configuración efectiva.
  - **Cambio de API:** `LeidenConfig` gana tres campos; un literal sin
    `..Default::default()` tiene que añadirlos.

### Fixed
- **HNSW escala otra vez** (#201). El arreglo de #184 (0.6.10) no aguantaba
  la escala: `keep_pruned` llenaba la lista de vecinos de cada nodo y el
  enlace inverso a cada punto nuevo se podaba enseguida, así que a 100k × 384
  un tercio de los puntos quedaba sin enlaces entrantes; la verificación los
  reinsertaba en cascada (hasta 73 893 copias muertas) y cada búsqueda pedía
  `k` más un vecino por reinserción, lo que anulaba el `ef_search` del
  llamador. Ahora no se usa `keep_pruned`, y el ~1% de puntos que no se
  encuentran a sí mismos tras insertarse quedan como **huérfanos** que cada
  búsqueda compara directamente (`HnswIndex::orphans()`; `repaired()` queda
  obsoleto y devuelve 0). Medido a 100k × 384 con datos agrupados: build
  139 s → 44 s, memoria 1 111 → 646 MiB, p95 de `search_knn` k = 10 167 ms →
  0.87 ms (el scan exacto tarda ~31 ms), recall@10 0.941 → 0.991 con
  `ef_search` = 30 y 0.998 con 100. Los dumps de 0.6.10 (formato 2) se
  reconstruyen una vez.
- **Índice de propiedades: una entrada por nodo** (#197). Guardaba un
  `Vec<NodeId>` por `(propiedad, valor)` y lo leía y reescribía entero en
  cada alta y baja, así que escribir nodos con un valor compartido (`type`,
  `status`, `level`) era **cuadrático** en todos los caminos de escritura.
  Formato v3: clave `(propiedad, valor, node_id)` con valor vacío; alta y
  baja son un put o un delete, y la búsqueda es un scan del prefijo con
  longitud exacta (`"PER"` no abarca a `"PERSON"`). Medido con `type` y
  `level` compartidos: `bulk_loader` de 40k nodos 18.4 s → 0.27 s,
  transacción de 40k 20.0 s → 1.2 s, `add_node` directo de 10k 1.3 s →
  0.16 s; a 100k, compartir el valor ya no cuesta nada (0.85 s contra 0.87 s
  con valores únicos). La primera materialización de comunidades a 100k
  pasa de 44 s a 21 s.

  **Migración automática** al abrir: una pasada O(n) que reconstruye el
  índice desde los nodos. El formato va en un sentinel nuevo
  (`prop_idx_entries`) para que bajar a ≤ 0.6.10 siga funcionando: esa
  versión no ve su sentinel y reconstruye su propio índice, y 0.6.11 lo
  reconstruye otra vez al volver. Una base con un formato más nuevo que el
  de la versión instalada se rechaza al abrir, en vez de leerse mal.
- **Leiden correcto: núcleo reescrito según el paper** (#194). `leiden(n)`
  (Rust `LeidenCommunity::detect`, NQL, Python `graph.leiden`) estaba roto:
  - reutilizaba ids de comunidad entre iteraciones, así que el tamaño de una
    comunidad podía bajar de cero: **pánico en debug** ("attempt to subtract
    with overflow") y **resultado corrompido en silencio en release**;
  - el refinamiento **reemplazaba** la partición (deshacía comunidades en
    nodos sueltos) en lugar de servir para agregar;
  - no había agregación: dos mitades de un grupo denso nunca se unían.

  Núcleo nuevo con índices densos (sin ids inventados), nodos con tamaño y
  el ciclo del paper: mover → refinar → agregar sobre la partición refinada
  (con la del movimiento como partición inicial del nivel agregado), hasta
  que mover no agrupe nada; luego se itera desde el propio resultado hasta
  que no cambie. Se reporta la partición del movimiento. Se conserva lo de
  #193 (alcance, pesos, caché por opciones, independencia del orden de
  inserción).

  **`leiden(n)` cambia su resultado:** menos comunidades, más grandes y con
  mayor calidad CPM. En grafos con bloques sembrados de 50 nodos (γ = 0.1):
  40 bloques densos → antes 362 comunidades (CPM 3 927), ahora 40 (CPM
  9 718, la de los bloques); 200 bloques dispersos → antes 3 451 (CPM
  7 401), ahora 632 (CPM 14 290, por encima de los bloques sembrados:
  12 031). Además es más rápido (release, bloques dispersos): 10k nodos
  1 340 → 58 ms, 100k nodos 18 067 → 761 ms (≈ 23×), por las estructuras
  densas y porque la agregación reduce el trabajo de cada nivel. El test de las familias florentinas
  pasa de 5 a 4 comunidades: la nueva partición tiene calidad CPM 11.7
  contra 11.1. Detalle en `docs/ALGORITHMS.md` (Phase 3).
- **Un nodo sin comunidad ya no aparece en la comunidad 0** (#190). Por la
  ruta de agregación de NQL, `community(n)`/`leiden(n)` promediaban sobre
  los nodos del grupo que tenían valor y devolvían 0.0 si ninguno lo tenía;
  ahora devuelven `null`. Con `leiden(n, labels = …)` un nodo fuera del
  alcance era indistinguible de uno de la comunidad 0.

---

## [0.6.10] - 2026-10-07

Iteración GraphRAG A: la ruta de recuperación local queda rápida, con
ranking y honesta. Búsqueda híbrida sin scan de etiqueta y con la rama de
texto acotada (NQL `hybrid()` + 1 salto a 100k: 132 → 0.99 ms),
`similar_to` que devuelve K filas, `score(var)` en NQL, carga de
embeddings por lote, `neighborhood` rankeado con Personalized PageRank, y
dos defectos de fondo corregidos: el commit de tantivy por documento (un
`open` con 100k nodos indexados pasaba de horas a 0.5 s) y los puntos del
HNSW que ninguna búsqueda alcanzaba.

### Added
- **`neighborhood` rankeado con Personalized PageRank** (#176):
  `ExpandOptions.rank = Rank::Ppr { alpha, iterations, candidate_factor,
  seed_weights }` (`Rank::ppr()` con 0.15 / 20 / 5 / pesos iguales); Python
  `neighborhood(..., rank="ppr", alpha=, iterations=, candidate_factor=,
  seed_weights=)`. Con `max_nodes` cortando, el BFS se quedaba con lo primero
  en orden de nivel; ahora el BFS junta hasta `candidate_factor × max_nodes`
  candidatos, un PPR sembrado en las semillas corre sobre ese subgrafo en RAM
  y se conservan los mejores por score (semillas siempre, desempate por
  `NodeId`). Un nodo a dos saltos al que apuntan varias semillas puede
  superar a uno a un salto al que apunta una sola. `Neighborhood.score_of`
  (Python `"score"`) trae el score; vacío con el default `Rank::Bfs`, que no
  cambia. El PPR es propio del módulo y determinista (índices por `NodeId`,
  suma en orden fijo, masa colgante a las semillas): no reutiliza
  `PageRank::personalized`, que vive tras `algorithms`, pierde esa masa y
  suma en el orden de las aristas. Medido a 100k chunks, 10 semillas, 2
  saltos, recortado a 50: BFS 0.149 ms, PPR 0.232 ms. **Cambio de API:**
  `ExpandOptions` gana el campo `rank`; un literal sin `..Default::default()`
  tiene que añadirlo.
- **`add_node_embeddings(model, [(id, vector), …])`: carga de embeddings por
  lote** (#175), en Rust (`Graph::add_node_embeddings`) y Python. Equivale a
  `add_node_embedding` por cada par, pero:
  - valida todo antes de escribir (misma dimensión en el lote y en el índice
    del modelo, sin nodos repetidos, todos existen): un lote inválido no
    escribe nada;
  - guarda en storage por bloques de 10k (`Storage::save_node_embeddings`,
    una transacción por bloque);
  - con el índice HNSW del modelo en caché, lo inserta en paralelo
    (`HnswIndex::insert_batch`) fuera del runtime async, con la misma
    verificación de alcanzabilidad que un build (#184).

  Medido (release, 384 dims): 10k sin índice 0.40 → 0.06 s; 100k sin índice
  4.2 → 1.9 s; 1000 más en un índice en caché de 100k 12.8 → 1.9 s (de 10k,
  5.7 → 0.72 s). Lo caro era el índice, no el storage: el "más de 2 h" de la
  carga de 100k desde Python se medía con el índice en caché, y antes de #178
  con un commit de tantivy por documento. `graphrag_sample.py` usa el lote.
- **`score(var)` en el FIND de NQL** (#174c): el score de la búsqueda
  vectorial del WHERE para el nodo de cada fila. Con `similar_to` es la
  similitud coseno (`1 - distancia`); con `hybrid`, el score RRF de
  `search_hybrid`; con ambas, el de `similar_to`, que es el orden. Mayor es
  mejor en los dos. Funciona en el camino de nodo suelto y en el patrón de
  un salto (cada fila expandida lleva el score de su nodo buscado), con o
  sin alias. Solo es columna del FIND: en WHERE, ORDER BY, GROUP BY, HAVING,
  junto a agregaciones o sin un `similar_to`/`hybrid` sobre la misma
  variable es un error con nombre, en vez de una columna `null` en silencio.
  EXPLAIN nombra la fuente (`Score: score(c) = …`). Hasta 0.6.9 los
  precomputes tiraban el score y solo quedaba el orden de las filas.

### Changed
- **La rama de texto de `search_hybrid` pide a tantivy solo lo que usa**
  (#174). `query_scored` pedía siempre los 1000 mejores documentos y leía el
  `node_id` guardado de cada uno, aunque la rama necesita `k × overfetch`
  (40 por defecto). Nuevo `Index::query_scored_top(query, limit)`: el
  full-text recolecta solo `limit`. La rama pide `k × overfetch` y escala ×4
  mientras su filtro (etiqueta o propiedades) deje menos, hasta el tope de
  siempre (1000, o `k × overfetch` si es mayor), así que nunca obtiene menos
  que antes. Medido a 100k chunks (release, `make bench BENCH=retrieval`):
  - solo texto: 1.92 → 0.62 ms;
  - `search_hybrid` con etiqueta: 2.26 → 0.95 ms;
  - NQL `hybrid()` + 1 salto: 2.42 → 0.99 ms.

  Lo que queda es tantivy puntuando todas las coincidencias: el bench
  coincide en ~57k de 100k chunks a propósito (vocabulario de 20 palabras).
  El bench gana el caso `search/text_only_k10`.
- **`search_hybrid` con filtro de solo etiqueta y NQL `hybrid()` ya no
  recorren la etiqueta** (#174a). Antes el filtro se resolvía armando el
  conjunto permitido con `get_nodes_by_label`, que lee todos los nodos; NQL
  siempre pasa la etiqueta del patrón, así que toda consulta `hybrid()`
  pagaba ese scan. Ahora cada rama comprueba la etiqueta sobre sus propios
  candidatos, en orden de rango: el texto filtra su lista leyendo los nodos
  por bloques y el vector pide `k × overfetch` vecinos y escala ×4 (hasta
  4096) mientras falten de la etiqueta. Como cada rama se restringe antes de
  la fusión, el top-k sigue calculándose dentro de la etiqueta (#115) con los
  mismos rangos que daba el conjunto permitido. Una etiqueta tan escasa entre
  los vecinos que la escalada llega al tope vuelve al conjunto permitido, así
  que el resultado no queda corto por el atajo. Un filtro con propiedades
  sigue usando el conjunto permitido. `VectorPath::LabelChecked` (Python
  `"label_checked"`) identifica el camino nuevo; ahí `allowed_set_size` es
  `None`. Medido a 10k chunks (release, máquina ociosa): `search_hybrid` con
  etiqueta 16.1 ms → 0.80 ms; NQL `hybrid()` + 1 salto 16.2 ms → 0.89 ms;
  sin etiqueta y `similar_to` sin cambio. El bench `retrieval` gana los casos
  NQL `similar_to_literal_one_hop` y `hybrid_literal_one_hop`.

### Fixed
- **HNSW: todo punto del índice es alcanzable por la búsqueda** (#184). La
  poda heurística de vecinos podía dejar un punto sin enlaces entrantes: estaba
  en el índice y ninguna búsqueda lo devolvía, ni con `ef_search` igual al
  tamaño del índice. Medido antes (384 dims): ~0.5% de los puntos de un build
  de 3000, ~1.6% de los insertados después, 1.35% de una muestra de un build
  de 100k. Era la causa de los tests inestables que #125, #144, #147 y #161
  resolvieron relajando aserciones.
  - El índice conserva los candidatos podados (`keep_pruned` de `hnsw_rs`), en
    `new`, `with_params` y al recargar un dump.
  - Tras un build y tras cada `insert`, cada punto nuevo se busca a sí mismo
    (10 vecinos, `ef = 64`, en paralelo en builds grandes) y el que no se
    encuentra se reinserta, hasta 5 rondas. `HnswIndex::repaired()` las
    cuenta; dejan su `DataId` viejo muerto en el grafo
    (`HnswIndex::dead_points()`), que la búsqueda descuenta, pero no suman a
    `tombstones` ni a `needs_rebuild` (un rebuild también repara: contarlas
    podría reconstruir en cada consulta).
  - Dump en formato 2 (guarda `repaired`): los de formato 1 se reconstruyen
    una vez al primer uso tras actualizar.

  Garantía: tras un build todo punto es alcanzable; tras un `insert`, el
  punto recién insertado. Una inserción posterior aún puede podar el último
  enlace hacia un punto anterior (medido: 0 de 10 000 inserciones a 384
  dims, 1 de 10 000 a 16 dims). Costo medido (release, 384 dims): build
  2.5 → 3.5 s a 10k y 75 → 114 s a 100k; 200 inserciones en un índice de
  100k 1.8 → 2.2 s. Con los mismos vectores aleatorios, recall@10 con el `ef`
  por defecto 0.08 → 0.98 a 100k. Tests: `hnsw_reachability_test` (falla
  contra la versión anterior); `hnsw_incremental_test` vuelve a afirmar que
  el punto recién insertado sale primero (#161 lo había relajado).
- **NQL `similar_to` con etiqueta devuelve K filas cuando la etiqueta tiene
  K nodos con embedding** (#174b). Pedía un `4·k` fijo al índice y se
  quedaba con los de la etiqueta: si otras etiquetas acaparaban los vecinos
  más cercanos devolvía menos de K filas sin avisar (en el test de
  regresión, **cero**). Ahora escala ×4 hasta 4096 mientras falten, y si la
  etiqueta es tan escasa entre los vecinos busca entre los nodos de la
  propia etiqueta (exacto cuando son pocos), con la misma ruta que la rama
  vectorial de `search_hybrid` con filtro de etiqueta
  (`Graph::knn_in_label`). Opciones nuevas `overfetch = N` (default 4) y
  `ef_search = N`, como en `hybrid`; EXPLAIN muestra la estrategia
  efectiva. El módulo `graph::hybrid` se compila con `embeddings-index`
  (antes `hybrid`); lo que usa full-text sigue bajo `hybrid`.
- **Índice full-text: un commit de tantivy por lote, no por documento**
  (#178). Cada `insert`/`remove` hacía `commit()` (fsync, segmento nuevo y
  recarga del reader), así que cualquier camino que indexa nodo por nodo
  pagaba un fsync por nodo:
  - `create_index` sobre datos existentes;
  - **cada `Graph::open`**, que reconstruye los índices desde storage;
  - `rebuild_property_index`;
  - las escrituras normales.

  Ahora `insert`/`remove` solo acumulan, y `Index::flush` publica en un
  commit:
  - el applier al cerrar cada lote, antes de los acks, así que quien recibe
    el ack encuentra lo que escribió;
  - `create_index` y el rebuild al abrir, al terminar de poblar;
  - `rebuild_property_index` y `close`.

  Como red de seguridad, una consulta con escrituras sin publicar las publica
  antes de leer. El fsync por documento no aportaba durabilidad: el
  directorio tantivy se reconstruye desde storage en cada `open` y el WAL
  repone lo que falte. Medido (release, macOS):
  - `create_index` full-text: 2k nodos 185 s → 0.23 s, 10k nodos 978 s → 0.22 s;
  - reabrir la base: 2k nodos 239 s → 0.27 s, 10k nodos 1218 s → 0.29 s;
  - a 100k: `create_index` 0.39 s y reabrir 0.47 s (antes, unas 2.7 y
    3.4 h estimadas; el bench `retrieval` pasó más de 4 h sin terminar su
    fixture).
- **Una sobrescritura del texto ya no deja el nodo un instante sin
  documento** (#178). El retract hacía `remove` (con su commit) y después
  `insert` (otro commit): entre los dos, una búsqueda no encontraba el nodo
  por ningún texto. El full-text identifica el documento por nodo y su
  `insert` ya reemplaza (`delete_term` + `add` en una llamada), así que en
  una sobrescritura que conserva la propiedad como texto el `remove` previo
  se omite (`Index::replaces_on_insert`). Se mantiene cuando la propiedad
  desaparece, cambia la etiqueta o se borra el nodo.

### Docs
- `docs/GRAPHRAG.md`: cifras de 0.6.9 a 100k — buscar + expandir en UNA
  consulta NQL con `similar_to` literal: 0.35 ms (el ciclo Python de 0.6.8
  en un solo viaje); con `hybrid` por NQL 132 ms porque la etiqueta del
  patrón llega al filtro híbrido como scan de la etiqueta (deuda de #115;
  arreglo rápido e índice de etiqueta en camino).
- Docs alineados con el comportamiento de 0.6.9 (#173):
  - ROADMAP temas 3 y 4 (inglés y español): marcados como entregados el
    analyzer (#74), los parámetros de `hybrid()` en NQL (#115), la inserción
    incremental en el HNSW (#113) y su persistencia (#114). Lo pendiente
    enlaza a #174–#176.
  - Comentario de cabecera de `graph/upsert.rs`: actualizar un embedding ya
    no invalida el índice.
  - README: el hipergrafo pasa de ✅ a experimental, porque `EdgeTarget`
    está definido pero no conectado a las aristas.
  - `docs/es/NQL_REFERENCIA.md` documenta `leiden(n)`, igual que la
    referencia en inglés.

## [0.6.9] - 2026-09-23

### Added
- **Vector literal en NQL (R7):** `similar_to(c, vector = [0.1, -0.2, …],
  model = "m", k = 10)` y `hybrid(c, text = "…", vector = [...], model = "m",
  k = 10)` toman el embedding de la pregunta escrito en la consulta (enteros
  y `1e-05` valen; la longitud debe coincidir con el índice del modelo). Las
  formas posicionales por nodo de referencia siguen igual; `k = N` vale en
  todas y gana al `LIMIT`. `hybrid` admite solo-texto o solo-vector en la
  forma con nombre.
- **Buscar y expandir en una consulta:** `similar_to`/`hybrid` siembran un
  patrón de un salto — `find c.text, e.name from (c:Chunk)-[:MENTIONS]->(e)
  where similar_to(c, vector = [...], model = "m", k = 10)` — sin scan de la
  etiqueta; `EXPLAIN` dice `PATTERN PIPELINE (seed: SIMILAR_TO|HYBRID)`. Es
  la consulta de un GraphRAG por MCP en un solo viaje.
- `EXPLAIN` imprime `SimilarTo: …` (antes `similar_to` era invisible) y
  `VECTOR SEARCH (similar_to|hybrid)` como estrategia; el vector literal
  aparece como `<literal dim=N>`, nunca sus componentes.
- NQL: los números admiten exponente (`2.5E3`, `1e-05`).

### Changed
- `similar_to`/`hybrid` ya no recorren la etiqueta y filtran por pertenencia:
  los candidatos del índice, hidratados por id, SON la fuente del stream
  (O(k) en vez de O(N)) y las filas salen mejor-primero cuando no hay `ORDER
  BY` (antes el ranking se perdía en un `HashSet`).
- `similar_to` calcula su top-K dentro de la etiqueta del patrón (pide 4·K
  al índice y se queda con los K de la etiqueta), como `hybrid` desde #115;
  antes `limit 3` podía devolver menos filas sin motivo visible.

### Fixed
- **`similar_to`/`hybrid` en un patrón con relación devolvían TODO el grafo
  en silencio:** el pipeline de patrones nunca los precomputaba y el
  predicado pasaba como `true`. Ahora un patrón de un salto se siembra desde
  la búsqueda; varios patrones, cadenas más largas, cuantificadores o buscar
  la variable destino son errores de validación con nombre.
- `similar_to` mal formado (aridad, tipos) también devolvía todas las filas:
  ahora se valida como `hybrid` desde 0.5.19. Un índice de propiedad en el
  resto del WHERE ya no se adelanta a la búsqueda vectorial. Sin la feature
  `embeddings-index` la consulta es un error, no todas las filas.

## [0.6.8] - 2026-09-23

### Added
- **Hidratación por id desde Python:** `get_node`, `get_nodes`, `get_edge`,
  `get_edges` (dicts `{id, label, properties}` / `{id, source, target, type,
  properties}`, `None` donde el id no existe, orden de entrada). `search_hybrid(...,
  hydrate=True)` y `knn_nodes(..., hydrate=True)` devuelven cada hit con su
  nodo, leído en una sola pasada; sin `hydrate` las formas son las de 0.6.x.
  Rust: `Graph::get_nodes`, `Graph::get_edges`.
- **Vecindario en una llamada:** `Graph::neighborhood(seeds, depth,
  &ExpandOptions)` (BFS por nodo con visitados global y profundidad mínima,
  filtro por tipo de arista antes de leer el nodo destino, filtro por
  etiqueta, `max_nodes` con bandera `truncated`, `max_edges_per_node` como
  freno para supernodos) y en Python `neighborhood(...)`, `neighbors(id,
  direction, edge_types)`, `degree(id, direction)`. Es el subgrafo de
  contexto de un GraphRAG.
- **NQL `in` / `not in`** con lista literal (`where e.name in ["a", "b"]`);
  las listas literales valen también en property maps. Igualdad estricta,
  como `=` (`1` no es `1.0`). Funciona en `find`, `update`, `delete` y en la
  VM de caminos.
- `benches/retrieval.rs` (búsqueda, hidratación, vecindario, NQL por id y
  patrón de un salto) y `docs/GRAPHRAG.md` con el ciclo completo.

### Performance
- **`where n.id = "…"` ya no recorre la etiqueta.** Costaba DOS scans: el
  fast-path filtraba una propiedad `id` que no existe (cero filas) y caía al
  scan estándar. Ahora `n.id = …` y `n.id in [...]` resuelven por lectura
  puntual y un id inexistente es cero filas, sin fallback. `n.prop in [...]`
  con índice hace n búsquedas; un `AND` raíz siembra con el lado indexado y
  aplica el resto como predicado. En un patrón, la misma condición sobre el
  nodo origen siembra el pipeline con esos nodos en vez de recorrer la
  etiqueta (`EXPLAIN`: `PATTERN PIPELINE (seed: ID LOOKUP)`).
- Medido con 100k chunks desde Python (wheel release, misma máquina): traer
  un hit por id 327 ms → 0.002 ms (`get_node`) / 0.013 ms (NQL); los 10 hits
  3.3 s → 0.016 ms; 1 hop desde un hit 590 ms → 0.007 ms (`neighborhood`) /
  0.01 ms (patrón NQL sembrado); 2 hops desde 10 hits ~17 s → 0.41 ms; el
  ciclo completo (híbrida k=10 hidratada + 1 hop de los 10 hits) ~9 s →
  0.27 ms. Tabla en `docs/GRAPHRAG.md`.
- `EXPLAIN` reporta los caminos nuevos (`ID LOOKUP`, `INDEX SEEK (IN)`)
  desde la misma decisión que ejecuta.

## [0.6.7] - 2026-09-22

### Fixed
- **El esquema derivado ya no se reconstruye tras cada escritura** (#164).
  Desde 0.6.6 cualquier mutación lo marcaba y la siguiente lectura
  (`get_schema`, `get_labels`, `get_label_count`, `get_edge_type_count`,
  `get_stats()["graph"]`) recorría nodos y aristas: 229 ms con 100k nodos,
  2.3 s y +430 MB con 1M. Ahora cada alta, baja o sobrescritura de nodo o
  arista, por cualquier camino (directa, transaccional, bulk loader, NQL
  `UPDATE`, replay del WAL, `import_parquet`), actualiza el esquema en
  O(propiedades) bajo el write gate, y `node_count()` / `edge_count()` leen
  sus contadores en O(1). El delta se calcula del par (nodo viejo, nodo
  nuevo), no de un "existía" que el redo del WAL haría mentir. Las
  propiedades por etiqueta o tipo son un superconjunto (no se retiran al
  borrar); una etiqueta o tipo desaparece al llegar su conteo a 0;
  `rebuild_schema()` sigue siendo la reparación exacta. Nuevo
  `Graph::schema_rebuild_count()`; test `schema_incremental_test` sobre
  todos los caminos. Medido con 1M nodos y 500k aristas (wheel release):
  `get_stats()` tras una escritura pasa de 2.45 s a 0.1 ms sin pico de RAM
  (antes +430 MB), `node_count()` de 72 ms a 0 ms, y la primera lectura
  tras `open` de 2.45 s a 0.1 ms; la carga masiva (8.2 s) y el `open`
  (2.3 s) no cambian.
- NQL `UPDATE` (`storage_insert_node` / `storage_insert_edge`) e
  `import_parquet` escribían al motor sin pasar por el write gate ni dejar
  rastro en el esquema; ahora hacen ambas cosas.
- `get_stats()` clonaba el esquema entero una vez por etiqueta; una sola
  lectura bajo el lock.
- `add_edges_batch` metía cada arista del lote en la adyacencia en memoria
  sin mirar si ya estaba: un id repetido (en el lote o ya en la base)
  dejaba la arista dos veces en `neighbors`/`degree` hasta reabrir. Solo
  las aristas nuevas estrenan entrada, como en el camino unitario.

### Changed
- **El esquema se persiste en cada checkpoint** (meta `schema_snapshot` del
  catálogo, MessagePack con versión de formato) y `open` lo carga: la
  primera lectura tras abrir es O(1). La primera escritura tras un
  checkpoint borra el snapshot antes del dato, y un `open` con
  recuperación de crash lo descarta, así que solo se reconstruye (una vez,
  recorriendo nodos y aristas) tras un crash o en la primera apertura de
  una base escrita por 0.6.6 o anterior; `close()` la reconstruye y
  persiste para que la siguiente apertura ya no lo haga. Viaja con
  `copy_database` / `migrate` (el catálogo se copia byte a byte). Un
  snapshot ilegible se descarta, nunca impide abrir.
- Retirada la nota "Known performance limitation" de `docs/OPERATIONS.md`
  y del doc de `Graph::get_schema`.

## [0.6.6] - 2026-09-22

### Fixed
- **El caché del esquema no se invalidaba tras escribir.** `SchemaManager`
  nacía sucio y nada lo volvía a marcar: `get_schema`, `get_labels`,
  `get_label_count`, `get_edge_type_count` y, desde 0.6.5, la sección
  `graph` de `get_stats()` devolvían el primer valor leído por handle,
  para siempre (desde 0.4.27). Ahora cualquier alta, baja o sobrescritura
  de nodo o arista lo marca, y la siguiente lectura reconstruye el esquema.
  Test `schema_cache_test` sobre los cinco caminos de escritura.

  **Known performance limitation:** after a graph mutation, the first read
  that needs the derived schema (`get_schema`, `get_labels`,
  `get_label_count`, `get_edge_type_count`, `get_stats()["graph"]`)
  triggers a full rebuild in O(N+E). Subsequent reads reuse it until the
  next mutation. User indexes and the property index are not affected: they
  are already maintained per operation. Incremental maintenance of the
  schema is tracked in #164.
- **`BulkLoader.add_edge` no aceptaba propiedades y devolvía `None`.** Ahora
  es `add_edge(source, target, edge_type, properties=None) -> str` (el UUID
  de la arista), igual que `Transaction.add_edge` y como el stub prometía
  desde hacía versiones. Los nombres por keyword pasan de
  `source_id/target_id/label` a `source/target/edge_type`.
- **`BulkLoader` tenía su propio conversor de propiedades:** guardaba `None`
  como `""` y rechazaba `bytes`, listas, tuplas y dicts con `TypeError`.
  Pasa por el conversor único de la frontera Python→Rust: `None` es null y
  bytes/listas/dicts anidados roundtripean con tipo exacto, como en
  `Transaction` y `upsert`. Datos ya persistidos no cambian.
- **`graph.edges_to_arrow()` lanzaba `ValueError("No edges to export")` en
  un grafo sin aristas** mientras `to_arrow_complete()` devolvía un batch
  vacío. Devuelve ahora un stream IPC vacío con columnas
  `id/source/target/edge_type` (mismo schema en ambas; quien capturaba el
  error comprueba `num_rows == 0`).
- **El stub `nopaldb.pyi` mentía en cuatro firmas** y nada lo detectaba:
  `to_arrow`/`to_arrow_complete` sin `label`, retornos `Any` donde son
  `bytes`, `bulk_loader` sin el `batch_size` obligatorio, `Property` sin
  listas/tuplas/dicts (que la API acepta), `NqlResult.write` como `Any`
  (ahora `WriteResult`), `BulkLoader.finish` como `Any` (ahora
  `BulkLoadStats`), `embedding_index_stats` como dict genérico (ahora
  `HnswStats | None`), y los nombres de parámetro de
  `ELReasoner.assert_conjunction`/`assert_existential_domain` distintos de
  los del runtime. Las clases pyo3 van marcadas `@final`.
- **GIL retenido donde más dolía:** la búsqueda HNSW de `knn_nodes` y la
  serialización IPC de `to_arrow`, `edges_to_arrow` y `to_arrow_complete`
  corrían con el GIL tomado; ahora corren fuera de él y los demás hilos
  Python avanzan mientras tanto.
- Docstrings: `embedding_index_stats` lista las siete claves que emite;
  `knn_nodes` ya no dice "sin caché"; `bulk_loader` tiene docstring.
  `nopaldb.__author__` se reexporta. Docs: `loader.commit()` y
  `load_nodes()` no existían (SCHEMA_INSPECTION, QUERY_PLANNER_DOCS);
  `API_REFERENCE` gana `BulkLoader`, `bulk_loader`, `node_count` y
  `edge_count`.

### Changed
- `graph.node_count()` ya no materializa todos los nodos: cuenta claves del
  storage sin deserializar (mismo resultado, sin `Vec<Node>`; sigue siendo
  O(N)). Nuevo `graph.edge_count()`; en Rust, `Graph::node_count` y
  `Graph::edge_count`.
- La comprobación de stubs pasa de comparar nombres a `mypy.stubtest` con
  allowlist (`nopaldb/python/scripts/stubtest_allowlist.txt`): firmas,
  defaults, propiedades y ambas direcciones contra la extensión compilada.
  Nuevo `make check-python` (stubtest + samples + `mypy --strict`), que es
  lo que corre el job `python-stubs`; `typecheck_sample.py` cubre Arrow,
  `BulkLoader`, conteos, `get_stats` y NQL.

## [0.6.5] - 2026-09-22

### Added
- **Observabilidad operativa** (#158). `Graph::stats()` en Rust y
  `graph.get_stats()` en Python devuelven el estado de la base en una sola
  llamada, estructurado: `graph` (conteos por etiqueta y tipo), `storage`
  (motor, perfil, directorio, solo lectura), `wal` (bytes, umbral del
  checkpoint automático, checkpoints en la sesión, último checkpoint,
  durabilidad de las escrituras directas), `recovery` (lo que hizo el `open`
  que creó el handle: registros del WAL leídos, operaciones reproducidas,
  transacciones sin commit descartadas, si fue crash recovery, si se
  reconstruyó la adyacencia, y milisegundos por fase: storage, replay,
  adyacencia, índices, total), `indexes` (con tamaño y analizador, el mismo
  texto que el informe de migración), `hnsw` (un elemento por modelo en
  caché) y `gc` (scheduler automático y último ciclo). Hasta ahora casi todo
  esto se calculaba y se tiraba, o solo salía por `log::info!`. Las claves
  planas de 0.6.x (`total_nodes`, `total_edges`, `avg_degree`,
  `storage_engine`, `wal_bytes`, strings) se conservan un minor más en el
  nivel superior del dict de Python. Stub `.pyi` con `TypedDict`.
- **Evento de progreso** para lo que tarda: `Graph::set_progress_callback`
  y `Graph::open_with_progress` en Rust; `graph.set_progress_callback(fn)` y
  `Graph.open_with_options(..., on_progress=fn)` en Python. Recibe
  `{phase, done, total}` cada ~1000 ítems o ~250 ms desde `replay_wal`,
  `rebuild_adjacency_from_edges`, la carga de índices al abrir,
  `create_index`, `rebuild_property_index`, `upsert_batch` y `BulkLoader`.
  Sin callback registrado cuesta una comparación por lote. Los `log::info!`
  se mantienen.
- **`nopaldb stats <dir>`** en el CLI (feature `cli`): abre en solo lectura,
  imprime las secciones y muestra el progreso del replay por stderr. Es la
  herramienta de "¿qué le espera al próximo open?" sin escribir código.
- `FullTextAnalyzer::describe()`: la línea `"default"` /
  `"spanish+stemming+…"` que ya usaba el informe de migración, compartida.
- `docs/OPERATIONS.md`: qué mirar para cada síntoma (open lento, ingesta
  lenta, WAL que crece, full-text sin resultados, disco que crece) y cómo
  leer los logs si no hay callback. Enlazado desde el índice y ADOPTION.

### Fixed
- **Soltar el `Graph` nada más recibir el ack de una escritura y reabrir la
  misma ruta fallaba con "ya está abierta en este proceso".** El applier
  enviaba cada ack con sus clones de `Graph` todavía vivos (avanzaba la marca
  del redo, persistía los relojes y hacía el checkpoint automático después),
  así que el lock del directorio, que vive en el engine, sobrevivía unos
  milisegundos al `drop` del usuario. En Rust era una carrera rara; en Python
  (`del g` seguido de `Graph.open`) era sistemático. Los acks salen ahora al
  final del lote, con el gate y los clones ya soltados. Regresión en
  `open_lock_same_process_test`.

### Changed
- `EmbeddingIndexStats` ya no lleva `cfg(feature = "embeddings-index")`: el
  tipo existe en todo build (la sección `hnsw` del reporte está vacía sin la
  feature); `SharedHnswIndex` sigue tras la feature.
- `Graph::open_read_only_with_options` delega en
  `open_read_only_with_progress(.., None)`; sin cambio de comportamiento.

## [0.6.4] - 2026-09-21

### Fixed
- **El WAL se trunca solo, y solo tras un checkpoint durable del motor**
  (#150). Hasta 0.6.3 el único truncado era `Graph::checkpoint()`, que
  ninguna ruta de producción llamaba y Python no exponía: una base con
  escrituras crecía sin límite (~250 B por operación) y el replay al abrir
  con ella. Y ese checkpoint truncaba SIN hacer durable el motor antes: un
  apagón justo después podía perder escrituras confirmadas. Ahora el applier
  hace el checkpoint cuando el log supera `StorageOptions::wal_checkpoint_bytes`
  (16 MiB por defecto; `0` lo desactiva), `close()` lo hace siempre, y el
  orden es fijo: relojes lógicos, flush durable del motor (redb
  `Durability::Immediate`, sled `flush`), y solo entonces el WAL se vacía. El
  harness de crash corre el hijo con umbral de 32 KiB y checkpoints
  explícitos, y exige tras cada kill todo lo confirmado.

- **sled: una apertura ya no falla para siempre por un snapshot a medio
  escribir.** sled escribe cada snapshot en `snap.<lsn>.generating` y lo
  renombra al terminar; si el proceso muere en medio (el harness de arranque
  en frío lo provoca matando al hijo milisegundos después de abrir), el
  archivo truncado queda y el listado de sled lo toma como el snapshot más
  reciente: `Read corrupted data` en cada apertura siguiente. El motor sled
  ahora borra los `.generating` al abrir (nunca fueron válidos; sled
  reconstruye desde el log). Lo tapaba el replay del WAL; al truncarlo en
  `close()` salió a la luz.
- **El timestamp del registro `Checkpoint` (reloj de pared, ms) ya no se
  mezcla con los timestamps lógicos** al recuperar: el reloj MVCC saltaba a
  ~1.7e12 en el primer open tras un checkpoint.

### Added
- `Graph.checkpoint()` en Python (y stub), `Graph::wal_bytes()` en Rust y
  `wal_bytes` en `graph.get_stats()`: lo que el próximo `open` tendría que
  reproducir.
- `docs/DURABILITY.md`: sección "WAL checkpoints and truncation" y tabla
  "From Python" (qué garantiza cada llamada, qué sobrevive a un apagón y
  cómo recuperar una carga interrumpida).
- `StorageOptions::wal_checkpoint_bytes` (nuevo campo; construir con
  `..Default::default()`).

---

## [0.6.3] - 2026-09-20

### Fixed
- **`upsert` ya no recorre toda la base por fila** (#151). La búsqueda de
  identidad `(label, key, value)` deserializaba TODOS los nodos de la base en
  cada upsert (no solo los del label), así que una ingesta incremental era
  cuadrática: 5.8 ms/fila con 1k nodos, 11.8 con 8k, ~160 con 100k. Ahora
  resuelve por el índice de propiedades `prop_idx_v2` (O(coincidencias); las
  claves `Bytes`/`List`/`Object`, que el índice no codifica, conservan el
  recorrido). Aplica a `upsert_node`, `upsert_batch`, `delete_node_by_key`, a
  los targets de `links` y a la validación de fantasmas de
  `get_nodes_by_label_and_property` en `RepeatableRead`/`Serializable`.
- **El bulk loader alimenta los índices.** `add_nodes_batch` (y con él
  `BulkLoader`) escribía los nodos sin entradas en `prop_idx_v2` ni en los
  índices de usuario: eran invisibles para `find_nodes_by_property`, para el
  planner y, con el cambio anterior, para `upsert`, que habría duplicado cada
  nodo cargado en bulk. Ahora indexa como cualquier escritura. Coste medido
  (redb, 1k nodos de 2 propiedades): `bulk_load/1k_nodes_open_db` 7.3 → 11.8 ms,
  `fresh_db` 48 → 54 ms; unos 4.5 µs por nodo que compran índices que dicen la
  verdad.

### Performance
- `upsert_batch` de 1 000 filas nuevas: **21.7 ms** (21.7 µs/fila) tanto con
  1k como con 8k nodos en la base, frente a 5.8 s y 11.8 s antes (5.8 y 11.8
  ms/fila): 270–540×, y plano respecto al tamaño de la base.

### Changed
- **`upsert_batch` / `upsert_many`: una transacción por bloque de
  `UPSERT_TX_ROWS` (1 024) filas** en vez de una por fila: un fsync por bloque.
  Un bloque es atómico (si una fila falla no se escribe ninguna de su bloque;
  los bloques anteriores quedan). Una clave repetida dentro de un lote actualiza
  el nodo creado por la fila anterior en vez de duplicarlo, y un `link` a una
  fila del mismo lote resuelve a su nodo (o al stub que se creó, que la fila
  rellena después). Firmas sin cambio.

- **La migración entre motores lleva los índices de usuario y el HNSW**
  (#152). `Storage::copy_database` (y con ella `nopaldb migrate`, el ejemplo y
  `Graph.migrate` en Python) copiaba los 12 keyspaces KV, pero los índices
  hash/btree/full-text/taxonomy viven en `<dir>/indexes/` (catálogo
  `metadata.bin` y, los full-text, sus segmentos y `analyzer.json`) y el dump
  HNSW en `<dir>/hnsw/`: el destino abría sin índices, sin el analizador
  configurado y con `verified=true`. Ahora los dos directorios se copian
  archivo por archivo, verificados por tamaño y checksum, y `docs/MIGRATION_0.6.md`
  deja de afirmar que "los índices sobreviven" sin más.

### Added
- `MigrationReport` con tres secciones: `keyspaces`/`verified` (datos),
  `indexes` (cada índice de usuario con tipo y analizador) y `sidecars` +
  `hnsw_copied` (si no había dump, el destino reconstruye el HNSW desde
  `embeddings` en la primera búsqueda). `render_report`, el CLI y el `dict`
  de `Graph.migrate` los muestran. Añadir campos a `MigrationReport` rompe a
  quien lo construyera a mano; nadie fuera del crate debería.
- `Graph::rebuild_indexes()` / `Graph.rebuild_indexes()` (Python): reconstruye
  los índices desde el catálogo como hace `open`, para quien copia bases con
  herramientas propias.
- Matriz de compatibilidad (versión ↔ formato en disco ↔ apertura directa o
  migración ↔ vuelta atrás) en `docs/MIGRATION_0.6.md`.
- Bench `upsert_batch` en `graph_ops` (1 000 filas nuevas sobre una base de 1k
  y de 8k) y suite `upsert_batch_test` con la regresión de complejidad.

---

## [0.6.2] - 2026-09-19

### Added

- **Binario `nopaldb`** (feature `cli`, con ambos motores; `cargo install nopaldb --features cli`) — cierra [#140](https://github.com/sharop/nopaldb/issues/140). `nopaldb migrate <src> <dst> [--from auto|sled|redb] [--to auto|sled|redb] [--profile …]` copia una base a otro motor con verificación e imprime una línea por keyspace y el veredicto; códigos de salida 0/1/2 (verificado / uso / falló). `nopaldb engine <dir>` dice qué motor tiene una base. Sin dependencias nuevas: los argumentos se parsean a mano. La misma implementación vive en el módulo público `nopaldb::migrate` (`migrate`, `render_report`, `parse_engine`, `parse_profile`), que también usa el ejemplo `migrate_engine`. `Storage::detect_engine(dir)` pasa a ser público.

## [0.6.1] - 2026-09-18

### Fixed

- **Deuda del camino de escritura** — cierra [#143](https://github.com/sharop/nopaldb/issues/143). (1) El commit transaccional escribía dos veces el registro del nodo en `entities` (una en el batch atómico de la versión y otra en `apply_add_node`); ahora tras el batch solo se registra la RAM. (2) `Storage::insert_node_version` eran cinco commits del motor no atómicos; ahora es un `apply_multi`: una versión ya no puede quedar sin puntero current ni sin su entrada del índice tras un crash. (3) La deduplicación de la adyacencia en RAM al insertar una arista era `Vec::contains`, O(grado) por arista y cuadrática en un supernodo; ahora es una lectura puntual `edge_exists`, O(1). Bench nuevo `supernode_fanout/20k_edges_one_source`. (4) `BulkLoader::finish` hace un checkpoint durable del motor: lo cargado ya no depende del flusher periódico ni de un `close()` posterior. (5) El test de "adyacencia perdida ⇒ rebuild desde aristas" corre en ambos motores mediante la seam de tests `Storage::debug_clear_adjacency` (antes solo abría sled en crudo).

## [0.6.0] - 2026-09-18

Cierra [#131](https://github.com/sharop/nopaldb/issues/131). Guía de migración: [docs/MIGRATION_0.6.md](docs/MIGRATION_0.6.md) · [docs/es/MIGRACION_0.6.md](docs/es/MIGRACION_0.6.md).

### Changed

- **redb es el motor de almacenamiento por defecto.** `default = ["storage-redb"]` y el tier `core` (y por herencia `semantic`, `full`, `python-full`) llevan redb; `storage-sled` deja de venir en los tiers y pasa a ser una feature opcional. Una base creada con 0.5.x **se abre igual**: `StorageOptions::default().engine` es ahora `StorageEngine::Auto`, que usa el motor del directorio si ya hay una base (redb deja `nopal.redb`; sled deja `conf` y `db`) y redb si el directorio es nuevo; al abrir una base sled deja un aviso con la receta de migración. Un motor explícito sobre un directorio del otro motor sigue siendo error, ahora con la receta de migración en el mensaje; un build sin `storage-sled` que encuentra una base sled dice qué feature falta y cómo migrar. Los constructores con perfil (`open_with_profile`, `in_memory_with_profile`) ya no fuerzan sled. El gate que decidió el cambio y sus números están en #131: nightly de crash sobre redb verde durante cuatro semanas, commits 2.4–2.9×, lecturas 1.03×, ingesta 1.42×, GC 69×, disco 0.03×, round-trip verificado de un millón de pares; los dos trade-offs que quedan (creación de base ~60 ms una vez por directorio; escritura directa fuera de transacción más lenta por operación) están medidos en `DURABILITY.md`.
- **Python:** `engine` vale `"auto"` por defecto en `Graph.open_with_options` e `in_memory_with_options`; `"redb"` y `"sled"` siguen aceptándose. **La wheel de PyPI lleva ambos motores** (antes solo sled), así `Graph.open` abre bases de 0.5.x y `Graph.migrate` puede convertirlas. Los mensajes de `ValueError` nombran los motores del build.
- Los sentinels de directorio de ambos motores usan la misma detección (`kv::detect_engine`) y ya no dependen de qué features estén compiladas.
- CI: el default (redb) corre la suite completa y el harness de crash; sled la corre como motor opcional (`--no-default-features --features storage-sled`), incluido el nightly de 100 rondas de SIGKILL.

### Added

- `StorageEngine::Auto` y `nopaldb::storage::kv::detect_engine(dir)`.
- `Graph.migrate(src, dst, src_engine="auto", dst_engine="auto", profile="default") -> dict` en Python: copia verificada entre motores (`Storage::copy_database`); devuelve pares y bytes por keyspace y `verified`. `Graph.storage_engine() -> "redb" | "sled"`.
- Test `engine_autodetect_test` (ambos motores) y `migrate_sample.py` en el job de stubs.

### Deprecated

- **sled como motor por defecto.** `storage-sled` sigue disponible al menos en 0.6 y 0.7 para abrir y migrar bases existentes; su retirada se decidirá con datos de uso en 0.8 y se anunciará un minor antes. Para crear una base sled nueva hay que pedir `engine = StorageEngine::Sled` / `engine="sled"` explícitamente.

## [0.5.24] - 2026-09-18

### Changed

- **redb: crear una base nueva cuesta menos.** Segundo PR del plan previo a 0.6.0. Tres cambios en `RedbEngine::open` y `Storage`: (1) en Unix el handle recién creado se conserva y se renombra el archivo abierto, en vez de cerrarlo (un fsync de cierre de redb) y volver a abrirlo; en Windows se mantiene la secuencia anterior. (2) Los diez keyspaces de `Storage` se crean en **una** transacción (`KvEngine::keyspaces`), antes eran diez commits. (3) Crear tablas ya no marca la base como pendiente de checkpoint (desde 0.5.23), así que abrir y cerrar sin escribir no fsynca. Medido en un Mac, abrir + cerrar una base nueva: 66 → 60 ms (sled: 19 ms); el `open` baja de 48 a 34 ms y el cierre sube de 18 a 26 ms porque el primer `Drop` de la base de redb (que escribe su tabla de estado del allocator y fsynca) ya no ocurre durante la creación sino al cerrar. En el bench `bulk_load/1k_nodes_fresh_db` (crear + cargar 1 000 nodos + soltar): redb 80.5 → 54.7 ms, sled 38.7 ms (de 0.48× a 0.71×). Lo que queda es de redb: la creación inicial del archivo y el `Drop` de la base hacen fsync; es un coste único por base, no por carga, y así queda documentado en `DURABILITY.md`.

## [0.5.23] - 2026-09-17

### Added

- **Las escrituras directas se registran en el WAL y se recuperan tras una caída.** `add_node`, `add_edge`, `delete_node` y `delete_edge` sin transacción no dejaban rastro en el log: su única durabilidad era el checkpoint periódico del motor (cada `flush_every_ms`), y una caída del proceso perdía hasta ese intervalo de escrituras confirmadas (en redb, incluso las que ya estaban en la caché del sistema operativo). Ahora el applier registra cada una como transacción automática (`Begin` + registro + `Commit`) en el mismo lote que los commits, antes de aplicarla, y el redo al abrir las reproduce. Nueva opción `StorageOptions::direct_write_durability`: `ProcessCrash` (default: el lote se escribe sin fsync y un sincronizador cada `flush_every_ms` más `close()` lo hacen durable; sobrevive a la muerte del proceso, ante apagón se pierde como mucho el último periodo, coste de microsegundos) o `Immediate` (fsync por lote como las transacciones). El harness de crash (`crash_commit_test`) confirma en cada ronda que todo lo confirmado antes del SIGKILL está tras reabrir, en sled y en redb. Construir `StorageOptions` con `..Default::default()`: el struct gana un campo. Coste medido de una escritura directa (mismo equipo, macOS): redb 39 µs (antes 50); **sled ~4 ms**: en macOS un `write()` al WAL espera a cualquier `F_FULLFSYNC` en curso en el volumen y sled fsynca su log continuamente bajo carga (`write_to_log` hace `sync_all` en cada búfer fuera de Linux); reproducido con redb y una tormenta de fsync artificial (39 µs → p90 4 ms). En Linux sled usa `sync_file_range` y el efecto no debería aparecer. Es el precio de registrar la escritura en el motor que se retira; con `Immediate` ambos motores pagan además el fsync del WAL.

### Changed

- **redb: ventana de commit.** El motor mantiene una write-transaction abierta y acumula en ella las escrituras de varias operaciones; la commitea a los 2 ms, a las 256 operaciones, antes de cualquier scan, en `flush()`/checkpoint y al cerrar. Las lecturas ven lo pendiente por un overlay en memoria. redb escribe al archivo toda página tocada en cada commit aunque sea `Durability::None`, así que una escritura directa pagaba ~50 µs de `pwrite`s por ~10 µs de trabajo; con la ventana el commit se reparte entre cientos de operaciones. La durabilidad no cambia: esos commits no eran durables (redb vuelve al último checkpoint ante un crash) y la recuperación la da ahora el WAL.
- **`WalManager::flush` hace fsync de verdad.** Solo vaciaba el buffer del proceso (no hacía nada con `write_all`); `close()` lo llamaba creyendo que dejaba el log durable. Ahora es alias de `sync()`.
- Un nodo creado con `add_node` directo y recuperado del WAL tras un crash queda con una primera versión MVCC (la misma reconstrucción que usan los commits); antes de 0.5.23 simplemente se perdía.

## [0.5.22] - 2026-09-16

### Changed

- **Los ids nuevos de nodos y aristas son UUID v7** (ordenados en el tiempo) — cierra [#128](https://github.com/sharop/nopaldb/issues/128). Hasta ahora eran v4: como el id encabeza las claves de `entities`, `history`, `adjacency` y `edges`, cada lote de nodos nuevos caía en posiciones aleatorias del B+tree de cada keyspace, y en redb (copy-on-write) eso cuesta 3× frente a claves ordenadas. Mismo tipo, anchura, texto y serialización; los ids v4 ya guardados conviven sin migración. Cambio de comportamiento observable: ordenar ids da orden de creación, no aleatorio, y un id revela el milisegundo en que se creó. Los ids fijados a mano (`Node::with_id`, `node.id = …`) siguen aceptando cualquier UUID. Generador: `nopaldb::types::fresh_id()`. Medido en `graph_ops` (misma máquina, 0.5.21 → esta versión): `bulk_load/1k_nodes_open_db` sled 5.30 → 3.33 ms, redb 9.89 → **2.34 ms** (redb pasa de 0.54× a 1.42× sled); `reads_64_with_writer` sled 142 → 119 µs, redb 667 → 548 µs; `commit/*` sin cambio (manda el fsync del WAL).

## [0.5.21] - 2026-09-15

### Changed

- **redb: camino de lectura sin transacción por `get`.** Cada `get`/`contains_key`/chunk de scan abría una `ReadTransaction` y hacía `open_table`; medido aparte sobre redb 4.1 eso era 0.30 µs por lectura sin escritor y 0.70 µs con uno, todo el exceso de redb frente a sled en lecturas puntuales. Ahora cada keyspace reutiliza su tabla de solo-lectura hasta el siguiente commit de datos del proceso, que vacía las de todos los keyspaces (redb es single-writer y el escritor es NopalDB, así que ninguna lectura ve un snapshot anterior al último commit, y el snapshot no retiene páginas durante fases de escritura). 64 `get_node`: redb 30 µs frente a 36 µs de sled; con un escritor de fondo, 43 µs frente a 153 µs. Sin cambio de contrato.
- **redb: checkpoint durable solo si hubo commits.** El flusher periódico, `flush()` y el cierre hacían un commit `Immediate` (un fsync) aunque la base no hubiera cambiado; `close()` seguido del `drop` eran dos fsync seguidos. Ahora se omite cuando no hay nada que persistir.
- **`add_edge` directo escribe en un solo commit.** Registro, adyacencia, primera versión MVCC, puntero current y cota del reloj eran cuatro commits del motor (`Storage::insert_edge_full` los funde en un `apply_multi`): en redb, donde cada commit escribe páginas al archivo, 4 aristas pasan de 470 a 345 µs; arista y versión quedan además atómicas entre sí. `insert_edge_with_adjacency` e `insert_versioned_edge` siguen existiendo.
- **Bench `graph_ops`:** grupo nuevo `reads_64_background_writer` (lectores con un escritor de fondo a ritmo fijo que no se espera: la degradación real del lector; `reads_64_with_writer` espera a sus escrituras y mide el máximo de ambas) y `bulk_load` dividido en `1k_nodes_fresh_db` (crear + abrir + cerrar dentro de la medición) y `1k_nodes_open_db` (solo la ingesta; lote por `NOPALDB_BENCH_BATCH`). Con esta versión, redb frente a sled: `reads_64` 1.05×, `reads_64_background_writer` 0.93×, `commit/*` 2.2–2.4×, `reads_64_with_writer` 0.21× y `bulk_load` 0.5× (el costo por commit de redb y la localidad de claves; sin cambio en esta versión).

## [0.5.20] - 2026-09-12

### Added

- **El índice HNSW se persiste y se carga al reabrir** — cierra [#114](https://github.com/sharop/nopaldb/issues/114). Hasta ahora el índice vivía solo en RAM y la primera búsqueda tras abrir lo reconstruía desde storage (2.7 s con 10k vectores, 107 s con 100k; con el dump, 108 ms y 1.02 s). Ahora el grafo de `hnsw_rs` va a disco con su `file_dump` nativo en `<data_dir>/hnsw/<modelo>.hnsw.{graph,data}` más un `.meta` (mapa `DataId → NodeId`, tombstones, huella FNV-1a de los embeddings de storage y longitud+hash de los dos archivos). Se escribe tras construir el índice completo y en `close()` si hubo `insert`/`remove` desde el último dump; todo por archivo temporal + rename. Al abrir, `get_or_build_embedding_index` calcula la huella de los embeddings en storage y carga el dump solo si coincide; si no hay dump, está desfasado (embeddings escritos sin `close`, un proceso que murió) o no pasa la verificación (bytes cambiados, truncado), reconstruye y reescribe. Solo se persisten índices con más de `EXACT_SEARCH_THRESHOLD` (1024) puntos: por debajo el rebuild cuesta milisegundos. Nuevo `Graph::persist_embedding_indices()` para escribir a mano (devuelve los modelos escritos); `EmbeddingIndexStats` y el dict de Python ganan `persisted` y `loaded_from_disk_ms`. Docs: `EMBEDDINGS.md` §"Persistence across reopens". El bench `open_first_search` mide ahora el camino con dump; `NOPALDB_HNSW_COLD=1` mide el rebuild.

### Changed

- **`embeddings::persistence` reemplazado.** El módulo anterior (metadata en bincode sin el grafo, nunca conectado a `Graph`) desaparece; sus funciones `save_index_metadata`/`load_index_metadata`/`index_metadata_exists`/`remove_index_metadata` y el tipo `HnswIndexMeta` no tienen sustituto directo: el grafo completo se persiste ahora desde `Graph`.

## [0.5.19] - 2026-09-11

### Added

- **NQL `hybrid(...)` con parámetros y filtro por el label del patrón** — cierra [#115](https://github.com/sharop/nopaldb/issues/115). Opciones con nombre tras los cuatro posicionales: `rrf_k`, `ef_search`, `overfetch`, `text_index` (`hybrid(n, "t", "ref", "m", rrf_k = 30, text_index = "Doc_body")`); hasta ahora estaban fijos en el código y afinar el híbrido obligaba a salir de NQL. El top-K se calcula **dentro del label del patrón** (`(n:Doc)` pasa como `HybridFilter`): antes la búsqueda corría sobre todos los labels y el stream tiraba los ajenos después, así que con `limit 1` un nodo de otro label que ganara dejaba la consulta en cero filas. `explain` imprime los parámetros efectivos, defaults incluidos. Gramática: argumentos con nombre en llamadas (`Expression::NamedArg`), admitidos solo en `hybrid`. Referencia NQL EN/ES: sección nueva "Búsqueda vectorial e híbrida en WHERE" (ni `similar_to` ni `hybrid` estaban documentados ahí).
- **HNSW: inserción incremental en vez de invalidar el índice** — cierra [#113](https://github.com/sharop/nopaldb/issues/113). `add_node_embedding` inserta el vector en el índice cacheado del modelo (o lo reemplaza si el nodo ya tenía uno: el punto viejo queda como tombstone) y `delete_node` lo retira; antes cada embedding nuevo tiraba el índice completo y la siguiente búsqueda pagaba un rebuild O(N): 2.4 s con 10k vectores, 103 s con 100k, medidos en #112. Las búsquedas piden `k + tombstones` al grafo para no devolver ni menos de `k` ni puntos retirados; cuando los tombstones superan el 20 % de los puntos vivos (y al menos 64), la siguiente `get_or_build_embedding_index` reconstruye desde storage. Sin índice en caché el comportamiento es el de siempre (build completo en la primera búsqueda). Nuevo `HnswIndex::{remove, contains, tombstones, needs_rebuild}` y `Graph::embedding_index_stats(model)` (también en Python).

### Changed

- `Graph::get_or_build_embedding_index` devuelve `SharedHnswIndex = Arc<std::sync::RwLock<HnswIndex>>` en vez de `Arc<HnswIndex>`: el índice ahora se muta en sitio y las búsquedas toman el read-lock (`index.read().unwrap().search_knn(..)`). Migración de una línea.

### Fixed

- **Reabrir una base en el mismo proceso tras `close()` decía "ya está abierta por otro proceso"** — cierra [#120](https://github.com/sharop/nopaldb/issues/120). `Graph::close()` solo hace flush; el lock del directorio vive hasta soltar el `Graph` (drop). Reabrir la misma ruta sin soltarlo esperaba el reintento completo del lock (1.5 s) y culpaba a otro proceso, que era falso; ningún motor puede distinguirlo (el flock no lleva PID, y en redb `DatabaseAlreadyOpen` es justamente la doble apertura en el mismo proceso). Ahora la capa KV lleva un registro de rutas abiertas en el proceso, liberado exactamente cuando el engine muere: reabrir una ruta que este proceso tiene abierta falla **de inmediato** con "ya está abierta en este proceso: el lock se libera al soltar el `Graph` (drop), no en `close()`", y el mensaje de otro proceso queda solo para un flock ajeno. La rustdoc de `open` y `close` explica el contrato (la promesa de 0.5.7 de "reabrir inmediatamente después de cerrar" solo era cierta si el valor se había soltado).
- **`hybrid(...)` mal escrito era un no-op silencioso**: `hybrid` y `similar_to` no se validaban; con una aridad incorrecta el executor no precomputaba nada y el predicado pasaba como `true`, devolviendo TODOS los nodos. Ahora la aridad, los tipos de los argumentos y las opciones se validan con un error que nombra el problema.
- **Python `knn_nodes` reconstruía el índice completo en cada llamada** (`build_embedding_index`, no la caché): con 100k vectores eran ~90 s por consulta. Ahora usa el índice cacheado del `Graph`, como NQL y `search_hybrid`.

---

## [0.5.18] - 2026-09-11

### Added

- **Bench del índice HNSW** (`benches/hnsw_ops.rs`) — cierra [#112](https://github.com/sharop/nopaldb/issues/112). Primera medición del índice vectorial: construcción, rebuild tras un embedding nuevo (el costo real de hoy), inserción incremental, búsqueda con `ef_search` default y 2×, búsqueda filtrada por selectividad (1 %, 10 %, 100 %) y apertura de base + primera búsqueda; vectores sintéticos de 384 dims con semilla fija, tamaños por `NOPALDB_HNSW_N`, motor por `NOPALDB_BENCH_ENGINE`. Los órdenes de magnitud van en `docs/EMBEDDINGS.md`: un embedding nuevo cuesta hoy un rebuild completo (~580× la inserción incremental a 10k vectores, ~10 000× a 100k) y abrir una base con 100k embeddings paga casi dos minutos antes de la primera búsqueda. Son los números que motivan [#113](https://github.com/sharop/nopaldb/issues/113) y [#114](https://github.com/sharop/nopaldb/issues/114). CI compila los tres benches (`cargo check --benches`, perfil dev: el perfil release lleva `panic = "abort"` y el harness de bench exige `unwind`) para que no vuelvan a quedar sin cobertura de compilación.

---

## [0.5.17] - 2026-09-10

### Added

- **SHACL: sequence paths `sh:path ( :a :b … )`** — cierra [#101](https://github.com/sharop/nopaldb/issues/101) y con él la serie "SHACL declarativo" (#98–#101). `PathSpec::Sequence`: los valores de un salto son los focus del siguiente, solo a través de nodos (un literal termina su rama sin error ni panic); el resultado es la unión de todas las ramas sin duplicados, así `sh:maxCount 1` sobre `( :usa :origen :en )` se cumple aunque dos ingredientes lleguen al mismo país. Las violaciones muestran el path como `usa/origen` (`PathSpec::to_sparql`, `Display`). Las demás formas de path de SHACL (`sh:inversePath`, `sh:alternativePath`, `sh:zeroOrMorePath`, `sh:oneOrMorePath`, `sh:zeroOrOnePath`) se reportan en `ShapesReport.ignored` con razón y la property shape se descarta, nunca se valida a medias: cambian lo que significa la cardinalidad sobre un cierre y merecen su propio diseño con la taxonomía.

### Changed

- `PathSpec::as_str` devuelve `Cow<str>` (una secuencia no tiene un `&str` que prestar); `PathSpec` implementa `Display`.

---

## [0.5.16] - 2026-09-09

### Added

- **SHACL: constraints lógicas, `sh:node`, `sh:severity` y `sh:message`** — cierra [#100](https://github.com/sharop/nopaldb/issues/100). `sh:and`/`sh:or`/`sh:xone ( shape shape … )` y `sh:not shape`, con shapes miembro anónimas o con nombre, property shapes incluidas; `sh:node :OtraShape` valida cada valor contra otra shape del documento (el validador carga los paths del propio valor). La violación de un combinador trae las ramas fallidas en el campo nuevo `nested`, así un `or` dice por qué falló cada rama (también en Python). `sh:severity sh:Warning|sh:Info` y `sh:message "…"` por shape y por property shape llegan al reporte: `conforms` solo mira `Violation` y el mensaje del autor sustituye al generado. `sh:deactivated true` carga la shape sin validar nada. Referencia que no existe = violación, no silencio.

### Changed

- **Un solo evaluador** (`evaluate_shape`): constraints de nodo, property shapes y combinadores pasan por el mismo código sobre `PathValue`, con los paths resueltos por adelantado (`EvalContext.paths`, acotado por nodo/shape para que un `sh:node` recursivo termine). `evaluate_constraints` sigue existiendo para las hojas.
- **`sh:pattern` se compila al cargar la shape** (`ConstraintType::pattern(src) -> Result`, `PatternConstraint`): un patrón inválido es error de carga con la shape y el patrón, en vez de un `Warning` por cada valor evaluado y una recompilación por evaluación. Migración programática: `ConstraintType::Pattern(s)` → `ConstraintType::pattern(s)?`.

---

## [0.5.15] - 2026-09-08

### Added

- **SHACL: `sh:targetClass` consciente de la taxonomía** — cierra [#99](https://github.com/sharop/nopaldb/issues/99). Los focus nodes de un shape sobre `:Comida` son sus instancias **y las de sus subclases** (`:Postre rdfs:subClassOf :Comida`), por cualquiera de sus tipos declarados, y la clase se nombra por label, `prefix:Local` o IRI completo: la misma resolución que `instanceOf` en NQL (#93), vía `TaxonomyIndex::resolve_class` + `is_instance_of`. Antes era un scan exacto por `label`: sin subclases, `sh:targetClass <IRI>` daba cero focus nodes y un individuo con dos tipos solo era focus del shape de su primer tipo. Sin taxonomía (grafo a mano) el resultado es el de siempre. Opt-out explícito: `ShaclValidator::with_target_mode(TargetMode::ExactLabel)`. Al cargar desde Turtle, `sh:targetClass` conserva el IRI completo del target.

---

## [0.5.14] - 2026-09-08

### Added

- **SHACL declarativo: shapes desde Turtle** — cierra [#98](https://github.com/sharop/nopaldb/issues/98). `Graph::validate_shapes(ttl)` / `validate_shapes_file(path)` y `ShaclValidator::from_turtle` cargan `sh:NodeShape`, `sh:targetClass`, `sh:targetNode`, `sh:property` (blank nodes o IRIs), `sh:path` de un salto, las 14 constraints por su término `sh:*`, `sh:in ( … )` y `sh:datatype xsd:*` con la misma tabla que usa el importer, reutilizando el parser Turtle del puente (misma gramática, mismos errores con línea y columna). Hasta ahora las shapes solo se construían desde Rust y `from_graph` leía tres propiedades. **Nada se ignora en silencio**: cada término `sh:*` no soportado o mal formado es una línea con razón en `ShapesReport.ignored` (`sh:closed`, `sh:and/or/not`, paths compuestos…), con el issue que lo cubre; `ignored` vacío = las shapes se aplican completas. Un `sh:targetNode` que no existe va a `ValidationReport.notes`. Python: `graph.validate_shapes(ttl) -> dict`. Docs: `docs/SHACL.md` (EN) y `docs/es/SHACL.md`.
- **Violaciones que dicen qué y sobre qué**: `ConstraintViolation` gana `constraint` (el componente, p. ej. `sh:MinCountConstraintComponent`), `value` (el literal o el `iri` del nodo culpable) y `shape_name`; `ValidationReport` y `ShapesReport` derivan `Serialize`.
- **`sh:class` y `sh:nodeKind` dentro de `sh:property` funcionan**: los valores de un path son ahora `PathValue::{Literal, Node}` y un solo evaluador juzga literales y nodos (`sh:class` por la taxonomía, directa o heredada, nombrando la clase por label, `prefix:Local` o IRI). Antes `PathSpec::Edge` convertía los destinos a UUID en texto y esas dos constraints eran un no-op silencioso en property shapes. Nuevo `PathSpec::Predicate(p)` = propiedad `p` ∪ aristas de tipo `p`, que es lo que carga `sh:path`; una propiedad `List` cuenta un valor por elemento (antes `sh:minCount 2` sobre un predicado repetido nunca se cumplía). `ConstraintType::NodeKindShacl` para `sh:nodeKind sh:IRI|sh:BlankNode|sh:Literal|…`.
- `shacl_validation_test` corre en CI por primera vez, junto al nuevo `shacl_turtle_test` (fixtures `recetario.ttl` + `recetario_shapes.ttl`).

### Changed

- La feature `shacl` pasa a `["regex", "owl-import"]`: las shapes son Turtle y `sh:class` se resuelve por la taxonomía; un `shacl` sin ellos era un motor sin puerta de entrada estándar. El tier `semantic` no cambia. `evaluate_constraints` recibe `&[PathValue]` y un `EvalContext`; `evaluate_node_kind_constraint` desaparece (mismo evaluador para los dos niveles); `Shape` gana `iri`.

### Fixed

- **`sh:targetClass` validaba el propio nodo clase**: `:Receta a owl:Class` lleva label `Receta` y `get_nodes_by_label` lo devolvía como focus node, así que cada `sh:minCount` fallaba también sobre la clase. Una clase no es instancia de sí misma; ahora se excluye.

---

## [0.5.13] - 2026-09-08

### Added

- **Analyzer full-text configurable por índice** — cierra [#74](https://github.com/sharop/nopaldb/issues/74). `create index on Nota(cuerpo) type fulltext with (language = "spanish")` activa stemmer, stopwords y plegado de acentos para ese índice (cada uno se apaga por separado; `ascii_folding` funciona sin idioma); Rust `Graph::create_index_with(.., IndexOptions { analyzer })` con `FullTextAnalyzer`, Python `create_index(..., analyzer={...})`. La consulta se analiza con el mismo analyzer del índice (tantivy lo resuelve del schema, así que no hay segundo camino que pueda divergir): `clasificacion` encuentra "clasificación", `catalogos` encuentra "catálogo", y `de`/`la`/`el` no entran al índice. Todo con piezas que tantivy ya traía: cero dependencias nuevas. El analyzer se guarda en un sidecar `analyzer.json` junto al directorio tantivy, NO en `metadata.bin`: ese archivo es bincode y un fallo de carga se traduce en "cero índices", así que añadirle un campo habría borrado en silencio los índices de toda base existente al actualizar. Sin sidecar = default = comportamiento idéntico al de 0.5.12. Introspección: `Graph::describe_index(name)` / Python `describe_index` devuelven el analyzer. Referencia NQL EN/ES: sección nueva "CREATE INDEX / DROP INDEX" (hasta ahora ninguna documentaba `type fulltext`).

### Fixed

- **`drop index` no borraba el directorio tantivy** del índice full-text: un drop + create del mismo nombre reabría los tokens viejos (y, con otro analyzer, el schema viejo). Ahora lo borra, y crear un índice cuyo nombre ya existe dice literalmente `drop index <nombre>` para cambiar el analyzer.
- `docs/INDEXING_DOCS.MD` documentaba un predicado NQL `fulltext(...)` que no existe y prometía "multilingual support" que no había; el stub Python de `create_index` decía `-> None` cuando devuelve el nombre. Los tests `hybrid_test`, `hybrid_explain_test` e `index_lifecycle_test` quedan registrados con `required-features = ["hybrid"]`, que es lo que necesitan para compilar.

---

## [0.5.12] - 2026-09-08

### Added

- **Exporter Turtle simétrico** — cierra [#70](https://github.com/sharop/nopaldb/issues/70). `export_turtle` escribe ahora TODO lo que el importer guarda: cada arista entre nodos con `iri` como object property (con el IRI del predicado que trajo el import, o el namespace de export + tipo de arista para las creadas en NopalDB), todos los tipos de cada individuo, literales tipados por la misma tabla del importer, y los **namespaces del documento** (el catálogo `rdf_prefixes()`) en vez de `http://example.org/ontology#` fijo; `café_de_olla` se escribe tal cual (los IRIs admiten no-ASCII) en lugar de `caf__de_olla`. La serialización la hace `oxttl::TurtleSerializer`, el mismo stack del parser, así que el escape y la validez de los IRIs son de la librería y no de un `format!`. Test de round-trip **import → export → import isomorfo** (nodos por IRI, aristas por sujeto/predicado/objeto, literales por valor y tipo) sobre una fixture con jerarquía, multi-tipo, relaciones, listas, datatypes y no-ASCII; el export es determinista y re-importarlo en su propio grafo no escribe nada.
- **`ExportReport`**: `classes`, `subclass_edges`, `individuals`, `type_triples`, `edges`, `literals`, `triples_written` y `skipped`, una línea con razón por cada valor o arista que Turtle no puede llevar (`Null`, `Bytes`, `Object`, listas anidadas, NaN/∞, propiedades de arista, aristas hacia nodos sin `iri`). Antes se omitían en silencio. `skipped` vacío = export fiel.
- **Export en Python**: `graph.export_turtle() -> (str, dict)` y `graph.export_owl_file(path) -> dict`, con stubs `.pyi` y un script de round-trip en CI.

### Changed

- `Graph::export_turtle` devuelve `TurtleExport { turtle, report }` en vez de `String`, y `export_owl_file` devuelve el `ExportReport`. Cambio de firma deliberado: un export que puede perder datos tiene que decir qué perdió en el mismo valor de retorno, no en un log. Migración: `graph.export_turtle().await?.turtle`.

---

## [0.5.11] - 2026-09-05

### Fixed

- **`instanceOf(n, "C")` omitía las instancias directas de `C`** — cierra [#93](https://github.com/sharop/nopaldb/issues/93). La comprobación de subclase de la taxonomía es estricta (`C` no es subclase de sí misma) y el evaluador le pasaba la clase del individuo como "hija", así que `:musgo a :Planta` nunca salía en `instanceOf(n, "Planta")` y `instanceOf(n, "Arbol")` devolvía 0 filas con `:roble a :Arbol`. El predicado solo funcionaba por herencia, y ningún test ni el tutorial lo notaron porque consultaban la raíz de la jerarquía. Ahora el tipo del nodo cuenta si es `C` o subclase de `C`; test de regresión con la fixture del issue.

### Added

- **`instanceOf` lee las aristas `instanceOf` y acepta IRIs** — [#93](https://github.com/sharop/nopaldb/issues/93). El predicado responde por TODOS los tipos declarados de un individuo (las aristas que escribe `import_turtle` desde 0.5.10), no solo el que quedó en `label`; y la clase se puede nombrar por label, por `prefijo:Local` (resuelto con el catálogo `rdf_prefixes()`) o por IRI completo, también en `path_*_instanceOf`. `subClassOf` acepta las mismas tres formas y sigue siendo estricto. Por dentro, `TaxonomyIndex` guarda el IRI de cada clase, los tipos de cada individuo y el catálogo de prefijos (`register_class_iri`, `register_instance`, `resolve_class`, `is_instance_of`), y `rebuild_taxonomy_from_graph` los reconstruye al abrir la base. Un nodo sin aristas `instanceOf` (creado a mano, por NQL o por un import anterior a 0.5.10) se sigue evaluando por su label.
- **Referencia NQL: sección "Predicados de Ontología"** (EN y ES). Hasta ahora ninguna de las dos referencias mencionaba `instanceOf`/`subClassOf` ni los filtros de camino `path_*_instanceOf`.

---

## [0.5.10] - 2026-09-04

### Added

- **Identidad por IRI y aristas en `import_turtle`** — cierra [#69](https://github.com/sharop/nopaldb/issues/69). Cada nodo importado guarda su IRI absoluto en `iri` y ESA es su identidad: `flora:Rosa` y `fauna:Rosa` son dos nodos (la segunda clase se etiqueta `fauna:Rosa` para que la taxonomía las distinga), y un segundo import del mismo documento no crea nada (cero nodos, cero aristas: hay test). Un triple con objeto-recurso (`:rosa :creceEn :jardin`) crea una **arista** `creceEn` con el IRI del predicado, consultable por NQL (`from (p)-[:creceEn]->(l)`); antes se aplanaba a string. Cada `rdf:type` crea una arista `instanceOf` al nodo clase (el primer tipo define el `label`, así `from (d:Disease)` e `instanceOf()` siguen funcionando), lo que da a NQL traversal y agregación por clase sin predicado especial. Un objeto nunca descrito es un **placeholder** (`rdf_placeholder = true`) que se actualiza en sitio cuando otro archivo lo tipa; una clase usada sin declarar se crea con warning. Los prefijos de cada documento se persisten en el catálogo (`Graph::rdf_prefixes()`, también en Python). `ImportReport` suma `edges_created` y `placeholders_created`.
- **Parser Turtle real en `import_turtle`** (`oxttl` + `oxrdf`, familia Oxigraph, tras el feature `owl-import`). El tokenizador casero no entendía `a`, desalineaba el statement con un lang tag (`"rosa"@es`), no soportaba blank nodes ni `@base`, y **no fallaba nunca**: un TTL malformado producía nodos basura en silencio. Ahora un archivo malformado devuelve `Err(RdfParseError)` con línea y columna y no escribe nada (todo o nada); `a`, lang tags, blank nodes (identidad sintética estable por documento, así que re-importar es idempotente), `@base`, colecciones y literales multilínea entran completos.
- `ImportReport.warnings`: lo que el import asumió por el documento — el prefijo vacío sin `@prefix :` (se resuelve contra `http://example.org/ontology#`, el namespace que el export siempre escribió), IRIs relativos sin `@base`, literales tipados que no parsean como su tipo. Vacío = el archivo se tomó tal cual. Expuesto en Python.
- Un predicado repetido con literal (`rdfs:label "rosa"@es, "rose"@en`) conserva todos los valores como `List`; antes ganaba el último.
- Los tres tests de integración del puente Turtle corren en CI. `--lib` no los compilaba y `synthetic_offshore_e2e_test` ni siquiera estaba registrado en `Cargo.toml`: nunca habían corrido en un job.
- **`make publish-crate` y `make check-on-main`.** `cargo publish` salió desde una rama dos veces (0.5.3 y 0.5.9); las dos veces el árbol resultó idéntico a `main` por suerte, no por diseño, y el SHA que crates.io registró apunta a un commit que ya no existe en GitHub. El paso "checkout main + pull" del runbook era el que se saltaba: ahora es una condición que falla, no un paso que se recuerda.
- docs.rs construye el crate con el tier `full` (`[package.metadata.docs.rs]`). Hasta 0.5.9 construía con features default: algoritmos, embeddings, reasoner, SHACL y aislamiento no aparecían en la documentación publicada. Y `lib.rs` tiene por fin portada: qué es NopalDB, el ejemplo de cinco minutos, la tabla de tiers y a dónde ir después.

### Changed

- **`triples_skipped` ya solo cuenta vocabulario reservado.** Todo lo que esté en un namespace de usuario se conserva (propiedades sobre clases incluidas); lo que se descarta son cabeceras `owl:Ontology`, declaraciones de propiedades, `rdfs:domain`/`rdfs:range`, `owl:equivalentClass` y demás axiomas que el puente no modela. Un conteo distinto de cero es una lista de axiomas, nunca de datos.
- `rdfs:label`/`rdfs:comment` van a las propiedades `rdfs_label`/`rdfs_comment`, no a `label`: `label` es el campo del nodo y `n.label` en NQL tiene que seguir significando eso.
- `rebuild_taxonomy_from_graph` (corre en cada `Graph::open`) solo acepta aristas `subClassOf` entre dos nodos Class, como ya hacía el reasoner. Con aristas por predicado de usuario, una `subClassOf` entre individuos habría entrado a la jerarquía en el siguiente arranque. El importer además califica un predicado de usuario llamado `instanceOf`/`subClassOf` (`ex:subClassOf`) para que nunca choque con los reservados.
- El export tipa cada individuo por sus aristas `instanceOf` (multi-tipo) y compacta los IRIs bajo el namespace por defecto (`:Alice`); sigue sin escribir aristas entre individuos ni los namespaces del documento (#70).
- **Los literales se tipan por datatype, no por la forma del texto.** `"42"` plano es `xsd:string` según RDF y queda como texto; `"42"^^xsd:integer` es `Int`; `"42"^^xsd:string` ya no se vuelve número. Mapa estable documentado en el módulo `rdf_owl`: familia integer → `Int`, decimal/double/float → `Float`, boolean → `Bool`, todo lo demás (incluidos lang tags y `xsd:date`) → `String` con el valor léxico. Fixtures y tutorial actualizados a literales tipados.
- La propiedad `iri` de un individuo guarda el **IRI absoluto** (`http://example.org/ontology#Alice`), no el token crudo (`:Alice`): el parser expande prefijos y no conserva el texto original. El export escribe esos IRIs entre `<>`. Bases importadas antes de 0.5.10 conservan el token crudo y siguen exportando bien; una re-importación sobre ellas crea nodos nuevos (identidad distinta): el upgrade path es re-importar en una base nueva.

### Removed

- `RDFTriple` (`rdf_owl::rdf`) y `local_name` público: eran el intermedio del parser casero, sin call sites fuera del módulo y sin documentación tras borrar `API_RUST.md`.
- `docs/es/API_RUST.md`. Documentaba 13 ítems, 8 con firma o descripción falsa (toda la sección `Storage` describía un KV de strings sobre `sled::Db` que el crate no es desde 0.5.0; a las `Transaction` les faltaba el `async`), cubría ~5% de la API pública y nadie lo enlazaba. La referencia Rust es docs.rs, generada del código, y la ruta de entrada a mano es `docs/ADOPTION.md`. Mantener una copia manual de firmas es exactamente cómo llegó a ese estado.
- `docs/es/API_REFERENCIA.md` pasa a llamarse `docs/es/API_PYTHON.md`: era la API de Python en español con un nombre que sugería otra cosa, y tampoco lo enlazaba nadie. Ahora lo enlaza el índice de docs en español.

### Fixed

- El ejemplo Rust de `docs/ADOPTION.md` llamaba `result.summary()`, que no existe en `QueryResult` (existe en `NqlResult`, otro tipo): no compilaba. Lo atrapó el doctest de la portada nueva de `lib.rs`, que es exactamente para lo que existen los doctests — el mismo ejemplo vive ahora en los dos sitios y se compila en `cargo test --doc`.

## [0.5.9] - 2026-09-02

### Added

- **`ROADMAP.md` público**: dirección por temas (endurecimiento del storage, ciclo semántico, retrieval, durabilidad del índice vectorial) sin fechas comprometidas, y una sección explícita de **fuera del alcance** con la alternativa recomendada para cada punto: SPARQL/named graphs (triple store al lado, NopalDB materializa el subgrafo), servidor de red (MCP o tu servicio delante), multi-proceso sobre un data-dir (un directorio por tenant, backup en frío para leer).
- `scripts/check_doc_links.py` + `make check-doc-links`, y corre en CI antes de compilar: los tres README de docs enlazaron durante meses a dos roadmaps que nunca existieron en el repo, y era lo primero que veía quien evaluaba contribuir.

### Fixed

- **`ImportReport.triples_skipped` decía que se perdían triples que estaban en el grafo.** La pasada 2 del importer Turtle contaba como descartado todo triple que no fuera `rdf:type`, incluidas las data properties de individuos que la pasada 3 importaba después. El tutorial del acto 3 propagaba el número (`18`) y lo explicaba como pérdida; el valor real para ese TTL es `0`. El conteo ahora se decide una sola vez, al final, con el mismo criterio que usó cada pasada: cuenta exactamente lo que no dejó nada en el grafo. Tests con conteo exacto (antes la aserción era `skipped > 0 || instances > 0`, que no podía fallar) y el gate del tutorial exige `0`.
- Una instancia cuya clase se declaró en un import **anterior** se descartaba en silencio, aunque el comentario del código prometía "o ya en el grafo": el importer solo miraba las clases del archivo actual. Ahora busca la clase en el grafo antes de descartar, así que ontología en un archivo e instancias en otro funciona.
- Diez links rotos en `docs/**`: los seis al roadmap, tres a un `examples/` en la raíz (viven en `nopaldb/examples/`) y uno a un `TRANSACTIONS.md` que nunca existió (ahora apunta a `DURABILITY.md`).

### Changed

- **Los límites reales del puente Turtle están escritos donde se decide.** "Lossy" se quedaba corto: los prefijos se descartan (dos IRIs con el mismo local name colapsan), un triple con objeto-recurso NO crea arista, el parser no entiende `a`/lang tags/blank nodes y no falla con Turtle malformado, y el export omite toda arista que no sea `subClassOf`. Todo eso, con lo que SÍ se conserva, está ahora en el doc del módulo `rdf_owl` (docs.rs), en `docs/ADOPTION.md` y en el README del acto 3.
- `docs/es/API_RUST.md` documentaba `Storage::insert_triple` y `Storage::get_object`, vestigios de un diseño triple-store que nunca existió en el crate (cero definiciones, cero call sites, y una firma con `sled::Error` que 0.5.0 sacó de la API pública). Eliminados.

## [0.5.8] - 2026-08-24

### Added

- **`Graph::open_read_only`: un handle que no puede modificar la base.** Toda vía de escritura falla con error tipado —escrituras directas, commits de transacción, embeddings y creación de índices— y la base queda intacta en disco. El rechazo ocurre ANTES de que el applier escriba al WAL, porque un registro de una operación rechazada lo materializaría el redo del próximo arranque. Funciona en los dos motores.
- **`docs/BACKUP_AND_READ_ONLY.md`**: el flujo de backup en frío con `copy_database` (copiar → abrir la copia), sus reglas, y qué NO hacer — copiar los archivos con la base abierta puede capturar un estado roto que parece válido.
- Test del error de doble apertura en redb, que existía sin cobertura desde 0.5.1 (el de sled llegó en 0.5.7).

### Changed

- La regla operativa de "un proceso por directorio" en `ADOPTION.md` ahora dice qué hacer cuando lo que quieres es leer, en vez de solo "cierra el otro proceso".

### Notas

- `open_read_only` **no da acceso concurrente**: toma el mismo lock exclusivo. No es una limitación temporal — sled no tiene modo de solo lectura y el de redb tampoco convive con un escritor abierto. Para leer sin tocar la base viva, backup en frío y abrir la copia.

## [0.5.7] - 2026-08-24

### ✨ Highlights

- **Honestidad del backend en el paquete de Python.** La API dejaba pedir `engine="redb"` en cualquier build, y el fallo llegaba dentro de `open()` como `RuntimeError: ... activa su feature storage-*` — un remedio imposible para quien instaló una wheel: no puede recompilar lo que ya viene compilado. Las wheels publicadas en PyPI traen **solo el backend sled** (redb es experimental y opt-in), y eso no estaba escrito en ninguna parte. Ahora el engine se valida contra lo que ESTE build contiene: pedir un backend ausente falla en la llamada con `ValueError` diciendo qué hay disponible y cómo obtener el otro.

### Added

- **`search_hybrid_explain`: por qué cada hit quedó donde quedó.** El score RRF ordena pero no explica — es una suma de recíprocos de RANGOS, así que no dice si un documento subió por texto, por vector o por ambos, ni con qué fuerza. La variante nueva conserva los números crudos que la fusión consume y descarta (score BM25, distancia coseno), la configuración que realmente se usó (índice full-text resuelto, `ef_search` efectivo, `candidates`, cardinalidad del filtro) y el underfill por rama, junto al camino vectorial —exacto o aproximado— que es lo que decide cómo leer un resultado corto. `search_hybrid` pasa a ser la vista reducida del MISMO cómputo: delega en vez de duplicar, así que pedir la explicación no puede alterar el resultado. Disponible en Rust y Python.
- `Index::query_scored` / `IndexManager::query_scored`: consulta conservando el score crudo de los índices que puntúan. El full-text ya calculaba el BM25 y lo tiraba; los índices sin score devuelven `None`, que es la respuesta honesta para una búsqueda exacta —no un cero, ni un número derivado del rango.

### Fixed

- **Un crash durante la creación de la base dejaba el directorio inservible para siempre — en LOS DOS motores.** Es el escenario del primer arranque de un despliegue nuevo (OOM, kill del contenedor, corte): la aplicación no volvía a arrancar y el mensaje se leía como corrupción de datos cuando en realidad la creación nunca terminó. No había datos que perder; lo que se perdía era poder arrancar. Cada motor lo cierra según su layout en disco:
  - **redb** (ventana ancha, determinista: 14 de 64 kills dentro de ella dejaban la base inabrible; ahora 0). Detalle abajo.
  - **sled** (ventana mucho más estrecha: ~3 de 20 barridos, con `Read corrupted data`; una primera medición de 64 intentos dio 0 y casi lo da por sano). Su base son varias entradas sueltas en el directorio, así que el rename atómico no aplica: ahora se detecta y descarta la creación incompleta al abrir. La firma es inequívoca —hay `conf` y no hay `db`, que **es** el archivo de datos— y aun así se exige que no exista ningún otro rastro de uso antes de tocar nada.
- **redb: creación publicada con un rename atómico.** Crear directamente sobre `nopal.redb` abría una ventana en la que el archivo ya existía pero todavía no era una base válida; si el proceso moría ahí (OOM, kill del contenedor, corte), toda apertura posterior fallaba con `I/O error: invalid data` y la aplicación no volvía a arrancar. Es el escenario del primer arranque de un despliegue nuevo. Ahora la creación se hace en un temporal y se publica con un `rename` atómico, así que el invariante es: si `nopal.redb` existe, es una base completa. Medido antes del arreglo: 14 de 64 kills dentro de la ventana dejaban la base inabrible; después, 0. Una base ya establecida nunca corrió ese riesgo (0 de 44), y se sigue verificando para que el arreglo no se confunda con "borrar y recrear".
- El error de apertura de redb distingue las dos formas de fallar en vez de mezclarlas: "otro proceso tiene la base abierta" y "el archivo existe pero no se puede leer" tienen remedios opuestos —cerrar el otro proceso vs. borrar el directorio— y confundirlos manda a alguien a destruir una base sana.
- **El lock de apertura se reintenta durante un tiempo acotado, en los dos motores.** Ninguno bloqueaba: pedían el lock una vez y se rendían. Reabrir inmediatamente después de cerrar o de un SIGKILL fallaba con "ya está abierta" aunque el kernel estuviera a punto de liberar el flock del proceso muerto. Le pasa a cualquier supervisor que reinicie al vuelo, y era la causa de una familia de fallos de CI que parecían aleatorios y venían de meses atrás. Si de verdad hay otro proceso, el error llega igual —un instante después— y ahora dice qué pasa y qué hacer, en vez de propagar un `WouldBlock` crudo.

### Changed

- `Graph.open_with_options` / `Graph.in_memory_with_options` (Python): `engine="redb"` en una wheel de PyPI ahora lanza `ValueError` en la llamada en vez de `RuntimeError` al abrir. Un build hecho con `--features storage-redb` lo sigue aceptando sin cambios.
- El error de Rust para un backend no compilado nombra la feature exacta (`storage-sled` / `storage-redb`) en vez del comodín `storage-*`, y está en inglés como el resto de la superficie pública.

### Docs

- `README.pypi.md`: qué backend trae la wheel, que redb es experimental y el comando de build desde fuente para evaluarlo.
- `docs/python/API_REFERENCE.md`: `open_with_options` / `in_memory_with_options` documentados en inglés por primera vez (solo existían en la referencia en español), con la tabla de disponibilidad por backend.
- `docs/FEATURE_TIERS.md`: fila `storage-redb` (faltaba desde 0.5.1), más `hybrid` y `full-isolation`; corregida la composición de `python-full`, que decía `analytics` cuando el manifiesto dice `full` — la doc subrepresentaba lo que la wheel incluye.
- `release-python.yml`: nota de que las features de la wheel salen de `pyproject.toml` y no del workflow. Corregido también el comentario del disparo, que decía que los builds corren en cada push.

## [0.5.6] - 2026-08-20

### ✨ Highlights

- **Ciclo de vida del nodo indexado.** Sobrescribir un nodo añadía sus valores nuevos a los índices sin retirar los viejos, así que el nodo seguía apareciendo en búsquedas por valores que ya no tenía; borrarlo dejaba atrás su documento full-text y sus vectores. Los tres caminos de escritura —directo, commit transaccional y redo del WAL— ahora retractan lo que la sobrescritura invalida **antes** de pisar el nodo viejo, que es la única ventana en la que los valores viejos todavía existen. Ese orden es además lo que hace converger al redo tras un crash, sea cual sea el momento en que ocurrió.
- **Las escrituras transaccionales ya indexan.** Un nodo creado con `tx.add_node`, `upsert_node` o NQL `CREATE` nunca entraba a los índices de usuario (hash/btree/full-text): el commit solo poblaba el índice de propiedades del storage, y el nodo era invisible a la búsqueda de texto hasta reabrir la base. Como `upsert_node` es la API de ingesta idempotente, esto afectaba justo al caso de uso de re-ingesta.
- **Un nodo borrado ya no reaparece en la búsqueda vectorial.** No existía forma de borrar el embedding de un nodo; el vector quedaba huérfano y, como el índice HNSW se reconstruye desde ese keyspace, el nodo volvía a los resultados en el siguiente rebuild.

### Added

- `Storage::delete_node_embeddings`: purga todos los vectores de un nodo (todos los modelos). La llama `delete_node`, que además invalida el índice HNSW en caché.
- Documentación del ciclo de re-ingesta (`content_hash` + modelo del embedding) en `docs/ADOPTION.md`, con la frontera explícita de qué mantiene el motor y qué le toca al llamador: el motor nunca re-ejecuta tu modelo de embeddings.
- Invariante nuevo en el harness de crash: el índice no conserva valores que el nodo ya no tiene, tras SIGKILL + redo.

### Fixed

- `FullTextIndex::insert` añadía un documento sin borrar el anterior del mismo nodo: reindexar dejaba DOS documentos, el texto viejo seguía matcheando y el índice crecía sin techo. Ahora reemplaza (`delete_term` + `add_document` en el mismo commit).
- `HashIndex`/`BTreeIndex::insert` acumulaban el mismo NodeId una vez por escritura; ahora son idempotentes por `(valor, nodo)`, como ya lo era el índice de propiedades.
- `delete_node` no retiraba el nodo de los índices de usuario (`IndexManager::remove` no tenía un solo call site en el crate).
- Un cambio de label en la sobrescritura dejaba colgadas las entradas del label viejo, que nombra los índices de usuario.

### Changed

- `Graph::rebuild_property_index` deja de ser el remedio de las sobrescrituras (el applier ya retracta); sigue siendo el `REINDEX` explícito y la reparación de las vías que no indexan a propósito (`add_nodes_batch`/`BulkLoader`).
- `upsert_node` retira su reconciliación post-commit: era más estrecha que el bug (solo alcanzaba el índice de propiedades, nunca los de usuario) y el applier ya hace el trabajo completo.

## [0.5.5] - 2026-08-20

### ✨ Highlights

- **Recall de la búsqueda vectorial filtrada.** Por encima del umbral exacto (1024 vectores), el filtro por label/propiedad se resolvía pidiendo `k × 4` vecinos GLOBALES al grafo HNSW y descartando después los no permitidos. Con un filtro selectivo los vecinos permitidos simplemente no estaban entre esos candidatos: la búsqueda devolvía menos de `k` —a veces cero— aunque existieran de sobra. La salida nunca fue incorrecta (nada no permitido se filtraba hacia afuera), pero perdía resultados en silencio. Medido sobre un índice de 5.000 vectores con ~1% de selectividad: **recall 0.00 antes, 1.00 ahora**.

### Added

- `HnswIndex::search_knn_filtered_explained`: además de los vecinos, devuelve cómo se resolvió la búsqueda (`FilteredSearchPath`, `ef_search` efectivo, intentos de escalada y si el resultado quedó `underfilled`). Un resultado con menos de `k` hits no se distinguía de "no hay más nodos permitidos"; ahora sí.
- `nopaldb::embeddings::rank_exact`: top-k exacto por coseno sobre un conjunto de vectores dado, con el desempate determinista por NodeId. Es la referencia compartida de los caminos exactos, para que la distancia reportada no pueda divergir entre ellos.
- `Storage::load_node_embeddings_for`: carga los embeddings de un conjunto acotado de nodos (omite los que no tienen), sin escanear el keyspace completo.

### Changed

- La búsqueda KNN filtrada usa el **filtro nativo de `hnsw_rs`** (`search_filter`), que aplica el predicado DURANTE el recorrido del grafo en vez de después, con escalada adaptativa de `ef_search` (×4 hasta `MAX_FILTERED_EF_SEARCH` = 4096) cuando no junta `k` permitidos. El comentario del código que afirmaba que ese filtro no existía en la API pública estaba obsoleto.
- `search_hybrid` elige camino según la cardinalidad de lo permitido: con un conjunto permitido chico (≤ 1024) dentro de un índice grande puntúa exactamente esos vectores en vez de recorrer el grafo — exacto y más barato. El índice no puede tomar esa decisión solo porque el predicado le llega como closure opaca.
- La garantía de que nada fuera del filtro aparece en el resultado se mantiene con un post-filtro propio, independiente del filtrado interno del motor HNSW.

## [0.5.4] - 2026-08-06

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

## [0.5.2] - 2026-08-04

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
