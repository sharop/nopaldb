# Migrar a 0.6.0: redb es el motor por defecto

> English: [docs/MIGRATION_0.6.md](../MIGRATION_0.6.md).

Desde 0.6.0 las bases nuevas se crean con **redb**. **sled**, el motor de toda
base 0.5.x, sigue disponible tras la feature `storage-sled` para abrir y
migrar esas bases, al menos hasta 0.7. Nada más cambia: mismo layout, mismo
WAL, mismo NQL, misma API de Python.

## Nada se rompe al actualizar

`Graph::open(path)` (Rust) y `Graph.open(path)` (Python) usan
`StorageEngine::Auto`: miran el directorio y usan el motor que ya está ahí.
Una base 0.5.x sigue funcionando con sled y deja un aviso en el log con la
receta de migración. Un directorio nuevo se crea con redb.

| El directorio contiene | `Auto` abre con |
|---|---|
| `nopal.redb` | redb |
| `conf` y `db` | sled |
| nada | redb (el default del build) |

Lo explícito sigue siendo explícito: `engine = Redb` sobre un directorio sled
(o al revés) es un error que dice cómo migrar. Un build compilado sin
`storage-sled` no puede abrir una base sled y lo dice, nombrando la feature y
las herramientas de migración. Las wheels de PyPI llevan ambos motores.

`graph.storage().backend_name()` (Rust) y `graph.storage_engine()` (Python)
dicen con qué motor abrió de verdad un handle.

## Por qué migrar

Medido en la misma máquina (0.5.24; ver la [tabla de rendimiento en el issue #131](https://github.com/sharop/nopaldb/issues/131)):
commits transaccionales 2.4–2.9× más rápidos, lecturas 1.03×, ingesta 1.42×,
GC de 20k versiones 69× más rápido, disco tras GC 0.03× del de sled; reabrir
una base grande tarda milisegundos en vez de segundos, y sled no tiene
desarrollo upstream desde 2021. Dos costes se quedan con redb y están
documentados en [DURABILITY.md](../DURABILITY.md): crear una base **nueva**
cuesta ~60 ms de fsync (una vez por directorio), y un `add_edge` directo fuera
de transacción es más lento por operación que en sled (168 µs por cuatro
aristas) porque redb escribe sus páginas en cada ventana de commit.

## Cómo migrar

La migración es una copia byte a byte de cada keyspace (nodos, versiones MVCC,
aristas, adyacencia, índice de propiedades, relojes, embeddings) seguida de un
re-escaneo del destino que comprueba conteos y checksums, más una copia archivo
por archivo de los dos directorios que viven fuera del KV: `indexes/` (índices
de usuario y sus analizadores) y `hnsw/` (dump del índice vectorial), cada
archivo verificado por tamaño y checksum. Time-travel sobrevive porque nada se
reinterpreta; un round trip sled → redb → sled de una base de un millón de
pares está verificado en la suite. Ver [Qué viaja y qué se reconstruye](#qué-viaja-y-qué-se-reconstruye).

Precondiciones:

1. Ningún proceso tiene abierto ninguno de los dos directorios.
2. El origen se abrió y cerró limpiamente al menos una vez con NopalDB, así su
   WAL está aplicado (ábrelo y llama a `close()` si dudas).
3. El directorio destino está vacío o no existe. La copia nunca mezcla.

### Python

```python
import nopaldb

report = nopaldb.Graph.migrate("data/plantas.db", "data/plantas_redb.db")
assert report["verified"]                 # conteos y checksums coinciden
graph = nopaldb.Graph.open("data/plantas_redb.db")
graph.storage_engine()                    # "redb"
```

`src_engine` y `dst_engine` valen `"auto"` por defecto (detectar el origen;
redb para el destino). Una verificación fallida lanza error y el destino no
debe usarse.

### Rust

```rust
use nopaldb::{Storage, StorageEngine, StorageOptions};

let report = Storage::copy_database(
    "data/plantas.db",       StorageOptions::default(),                                   // Auto: detecta sled
    "data/plantas_redb.db",  StorageOptions { engine: StorageEngine::Redb, ..Default::default() },
).await?;
assert!(report.verified);
```

### Línea de comandos

El binario `nopaldb` (feature `cli`, con ambos motores) envuelve la misma
función:

```bash
cargo install nopaldb --features cli
nopaldb engine data/plantas.db                       # sled | redb | ninguno
nopaldb migrate data/plantas.db data/plantas_redb.db  # --from auto --to redb por defecto
```

Imprime una línea por keyspace (pares y bytes), los totales y el resultado
de la verificación. Códigos de salida: 0 verificado; 1 uso o argumentos
inválidos; 2 la copia falló o el destino no estaba vacío. Sin instalar nada,
el ejemplo del repositorio hace lo mismo:

```bash
cargo run --example migrate_engine --features storage-sled -- data/plantas.db auto data/plantas_redb.db redb
```

### Volver atrás

La copia funciona en ambos sentidos: `dst_engine="sled"` recrea una base sled a
partir de una redb, verificada igual.

## Qué viaja y qué se reconstruye

| Parte de la base | Dónde vive | Migración | Al abrir el destino |
|---|---|---|---|
| Nodos, aristas, historia MVCC, adyacencia, relojes, índice de propiedades, embeddings | los 12 keyspaces KV | copia par a par, verificada por conteo y checksum | se usa tal cual |
| Índices de usuario (hash, btree, full-text, taxonomy): catálogo `indexes/metadata.bin`, segmentos full-text y `analyzer.json` bajo `indexes/fulltext_<nombre>/` | `<dir>/indexes/` | copia archivo por archivo, verificada por tamaño y checksum (desde 0.6.3) | hash/btree/taxonomy se reconstruyen en memoria desde el catálogo y los nodos, como en cada apertura; full-text abre sus segmentos con el analizador con que se creó |
| Índice HNSW | `<dir>/hnsw/` | se copia si el origen tenía dump (desde 0.6.3) | se carga de disco en la primera búsqueda; sin dump, se reconstruye desde `embeddings` en la primera búsqueda, igual que haría el origen |
| WAL | `<dir>/wal/` | no se copia: la precondición 2 significa que ya está aplicado | arranca vacío |

El informe tiene las tres secciones: `keyspaces` + `verified`, `indexes`
(nombre, label, propiedad, tipo, analizador) y `sidecars` + `hnsw_copied`.
Antes de 0.6.3 solo existía la primera, y `verified=true` no decía nada de los
índices, que no viajaban. Si copias una base con tus herramientas, coloca
`indexes/` y llama a `Graph::rebuild_indexes()` / `graph.rebuild_indexes()`, o
reabre.

## Matriz de compatibilidad

| Base escrita por | Abre directo en 0.6.x | Notas y vuelta atrás |
|---|---|---|
| ≤ 0.4.35 | sí, sled (`Auto` la detecta) | la primera apertura migra el índice de propiedades a `prop_idx_v2` (0.4.36) y el layout a v2 (0.5.3), in-place e idempotente; **después no la lee ≤ 0.5.2**. Guarda copia del volumen si pudieras volver. |
| 0.5.3 – 0.5.12, sled | sí, sled | los índices full-text no tienen `analyzer.json`: conservan el analizador por defecto. |
| 0.5.13 – 0.5.24, sled o redb | sí, cualquiera de los dos | 0.5.20+ escribe `hnsw/`; 0.5.22+ genera ids UUID v7 (las versiones anteriores los leen como UUID normales). |
| 0.6.x, redb | sí | un build 0.5.x con `storage-redb` también la abre, pero las wheels solo-sled de 0.5.x no: migra a sled antes (`dst_engine="sled"`). |
| 0.6.x, sled | sí (con aviso de migración en el log) | la lee 0.5.3+. |

Volver una versión atrás: restaura la imagen anterior y la copia intacta del
volumen, o migra al motor que esa versión soporte. Nunca abras con una versión
vieja una base modificada por una nueva sin copia verificada.

## Crear una base sled a propósito

Solo si hace falta (por ejemplo, para entregar una base a un build 0.5.x):
pasa `engine = StorageEngine::Sled` / `engine="sled"` explícitamente. `Auto`
nunca crea una base sled nueva.

## Política

- 0.6 y 0.7 conservan `storage-sled` y ambos motores en las wheels.
- Su retirada se decidirá con datos de uso en 0.8 y se anunciará un minor
  antes en el CHANGELOG.
