# Índice de Propiedades (Property Indexing)

Este documento describe el índice secundario de propiedades de NopalDB:
el **formato v3** en disco (0.6.11+, una entrada por nodo sobre las claves
tipadas del v2), la migración automática desde los formatos anteriores y las
herramientas de reconstrucción.

## Qué resuelve

Buscar nodos por `(propiedad, valor)` sin escanear toda la base: un
**índice invertido** que mapea `(propiedad, valor)` → `[NodeId, ...]`.

Consumidores: `get_node_by_property`, `get_all_nodes_by_property`, la
post-verificación de transacciones y el filtro del hybrid search.

## Formato v2 (0.4.36+)

Las entradas viven en un keyspace propio del motor (`prop_idx_v2`), separado del
tree default. La clave la produce una única función
(`encode_property_index_key`, `src/storage/mod.rs`):

```
key = [len(prop): u16 BE][prop utf8][type_tag: u8][valor canónico]

  tag 0x00 Null    → (sin bytes)
  tag 0x01 Bool    → 0x00 / 0x01
  tag 0x02 Int     → i64 BE con bit de signo invertido
  tag 0x03 Float   → f64 con transform IEEE754 total-order;
                     -0.0 normalizado a 0.0; NaN canónico único
  tag 0x04 String  → utf8 crudo
  Bytes/List/Object → no se indexan
```

En v2 el valor era un `Vec<NodeId>` en MessagePack, con read-modify-write
bajo el applier single-writer. v3 lo reemplaza (abajo).

### Por qué cada pieza

- **Length-prefix del nombre**: elimina la inyección de separador del
  formato legado (prop `a` + valor `b:c` colisionaba con prop `a:b` +
  valor `c`).
- **Type tag**: elimina las colisiones de tipo (`Int(1)`, `Float(1.0)` y
  `String("1")` compartían la clave `"1"`; ídem `Bool(true)` /
  `String("true")` y `Null` / `String("null")`).
- **Encoding order-preserving**: el orden de bytes coincide con el orden
  numérico (enteros con bit de signo invertido; floats con el transform
  total-order de IEEE754). Hoy nadie hace range scans sobre este índice,
  pero el formato ya los permite sin otra migración.
- **Canonicalización de floats**: `-0.0` y `0.0` son la misma clave; todo
  NaN colapsa a un NaN canónico (el legado indexaba `"-0"` ≠ `"0"`).

## Formato v3 (0.6.11+, #197): una entrada por nodo

```
key   = [clave v2 de (prop, valor)][node_id: 16 bytes]
value = vacío
```

- **Alta**: un put. **Baja**: un delete. Ninguna lee antes de escribir.
- **Búsqueda**: scan del prefijo `clave v2`, quedándose solo con las claves de
  longitud exacta `prefijo + 16`. El `String` se codifica sin terminador, así
  que el prefijo de `"PER"` abarca también las entradas de `"PERSON"`; el
  filtro de longitud las descarta. Los ids salen en orden de `NodeId`
  (con UUID v7, ≈ orden de creación).
- **Por qué**: el v2 leía, deserializaba, buscaba linealmente y reescribía la
  lista entera en cada alta y baja. Con un valor compartido por muchos nodos
  (`type`, `status`, `level`) cada escritura costaba O(n) y la carga entera era
  cuadrática. Medido (release, en disco, nodos con `type` y `level`
  compartidos):

| escritura | v2 | v3 |
|---|---|---|
| `bulk_loader`, 40k nodos | 18.4 s | 0.27 s |
| una transacción, 40k nodos | 20.0 s | 1.2 s |
| `add_node` directo, 10k nodos | 1.3 s | 0.16 s |

  Con v3 compartir el valor no cuesta nada: 100k nodos en 0.85 s con valores
  compartidos y en 0.87 s con valores únicos (`bulk_loader`).

El keyspace conserva el nombre `prop_idx_v2`: la migración lo vacía y lo
reconstruye en v3.

### Formato legado (≤0.4.35), solo referencia

```
idx:prop:{nombre}:{valor_stringificado} -> [NodeId, ...]   (tree default)
```

Stringificar el valor causaba las tres clases de colisión de arriba y
hacía imposible el orden numérico (`"10" < "9"` lexicográfico).

## Migración automática

Hay dos sentinels en `catalog`:

- `prop_idx_entries` = 3: el índice está en v3. Lo escribe 0.6.11+.
- `prop_idx_format` = 2: lo escribían (y lo escriben) las versiones ≤ 0.6.10
  al construir el índice en v2. **Desde 0.6.11 su presencia significa "una
  versión anterior reconstruyó el índice en v2"**.

Al abrir una base (`Graph::open*`), después del replay del WAL:

1. Si `prop_idx_entries` es mayor que el que conoce esta versión, el open
   **falla** con un error claro: la base es de una versión más nueva.
2. Si `prop_idx_entries` = 3 y `prop_idx_format` no existe, no hay nada que
   hacer.
3. Si no: se borran las claves legadas (`idx:prop:*` del tree default), se
   reconstruye el índice **desde los nodos** (fuente de verdad) en chunks de
   memoria acotada (`scan_nodes_batch`), se borra `prop_idx_format` y se
   escribe `prop_idx_entries` — **al final**, así que un crash a media
   migración la repite en el próximo open. Los índices de propiedades son
   datos derivados: los nodos y aristas jamás se tocan.

**Por qué un sentinel nuevo y no `prop_idx_format = 3`**: una versión ≤
0.6.10 vería `3 ≥ 2`, no migraría y buscaría blobs que ya no existen
(búsquedas vacías en silencio, y el upsert crearía duplicados). Con la clave
nueva:

- **Bajar a ≤ 0.6.10**: esa versión no ve su sentinel, reconstruye el índice
  en v2 desde los nodos y escribe `prop_idx_format = 2`. Funciona sin pasos
  manuales; cuesta una pasada O(n) en esa apertura.
- **Volver a 0.6.11+**: el `prop_idx_format` presente fuerza la
  reconstrucción en v3, que incluye lo que escribió la versión anterior.

**Downgrade a ≤0.4.35**: una base migrada abierta con ese binario no se
corrompe, pero el índice legado ya no existe → los lookups por propiedad
dan falsos negativos. Para volver atrás: reabrir con ≥0.4.36 (re-migra
solo si hace falta) o reconstruir manualmente.

## Reconstrucción manual

```rust
let procesados = graph.rebuild_property_index().await?;
```

Vacía el índice y lo reconstruye completo desde los nodos. Es la base de
un futuro `REINDEX` y repara un índice desalineado, por ejemplo tras
escrituras directas contra `Storage::insert_node`. Todos los caminos de
`Graph` indexan, `add_nodes_batch` / `BulkLoader` incluidos.

Ya no hace falta tras una sobrescritura normal: desde 0.5.6 el applier retira
las entradas que un overwrite invalida antes de pisar el nodo viejo, en los
tres caminos de escritura (directo, commit transaccional y redo del WAL).

## Semántica de lookup

Los lookups son **tipados**: buscar `Int(1)` no regresa nodos con
`Float(1.0)` ni `String("1")`. `get_node_by_property(prop, &str)` busca
`String` estricto (documentado en el método; hasta 0.4.35 «encontraba»
otros tipos vía la colisión del formato legado).

## Limitaciones actuales

1. **Solo búsqueda exacta** vía el API público; el formato ya soporta
   rangos en disco pero no están cableados al ejecutor.
2. **Bytes/List/Object no se indexan** (decisión F2: sin semántica clara
   de igualdad/orden para claves).
3. **Sin scoping por label**: el índice es global por propiedad. La
   familia de claves `(label, prop, valor)` está diferida al trabajo de
   índices únicos transaccionales (M1-8/M1-9); la maquinaria de rebuild
   de este formato hace esa migración futura barata.

## Tests

- `nopaldb/tests/prop_index_v2_test.rs` — contrato observable: lookups
  tipados sin colisiones, inyección resuelta, `-0.0`==`0.0`,
  persistencia tras reopen, rebuild repara índice desactualizado.
- Unit tests en `src/storage/mod.rs` — encode (tags disjuntos, orden
  preservado, canonicalización), migración (legado→v3, blobs v2→v3,
  idempotencia, crash-safety), ida y vuelta con una versión ≤ 0.6.10,
  rechazo de un formato más nuevo y búsqueda exacta (`"PER"` no abarca a
  `"PERSON"`, `Int(1)` ≠ `Float(1.0)` ≠ `String("1")`).