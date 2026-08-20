# Índice de Propiedades (Property Indexing)

Este documento describe el índice secundario de propiedades de NopalDB:
el **formato v2** de claves en disco (0.4.36+), la migración automática
desde el formato legado y las herramientas de reconstrucción.

## Qué resuelve

Buscar nodos por `(propiedad, valor)` sin escanear toda la base: un
**índice invertido** que mapea `(propiedad, valor)` → `[NodeId, ...]`.

Consumidores: `get_node_by_property`, `get_all_nodes_by_property`, la
post-verificación de transacciones y el filtro del hybrid search.

## Formato v2 (0.4.36+)

Las entradas viven en un árbol sled propio (`prop_idx_v2`), separado del
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

El payload es el mismo del formato anterior: `Vec<NodeId>` en MessagePack,
con read-modify-write bajo el applier single-writer.

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

### Formato legado (≤0.4.35), solo referencia

```
idx:prop:{nombre}:{valor_stringificado} -> [NodeId, ...]   (tree default)
```

Stringificar el valor causaba las tres clases de colisión de arriba y
hacía imposible el orden numérico (`"10" < "9"` lexicográfico).

## Migración automática

Al abrir una base (`Graph::open*`), después del replay del WAL:

1. Se lee el sentinel `meta:prop_idx_format`. Si es ≥ 2, no hay nada que
   hacer.
2. Se borran las claves legadas (`idx:prop:*` del tree default).
3. Se reconstruye el índice v2 **desde los nodos** (fuente de verdad) en
   chunks de memoria acotada (`scan_nodes_batch`), con log de progreso.
4. Se escribe el sentinel — **al final**, así que un crash a media
   migración simplemente la repite en el próximo open. Los índices de
   propiedades son datos derivados: los nodos y aristas jamás se tocan.

**Downgrade**: una base migrada abierta con un binario ≤0.4.35 no se
corrompe, pero el índice legado ya no existe → los lookups por propiedad
dan falsos negativos. Para volver atrás: reabrir con ≥0.4.36 (re-migra
solo si hace falta) o reconstruir manualmente.

## Reconstrucción manual

```rust
let procesados = graph.rebuild_property_index().await?;
```

Vacía el índice v2 y lo reconstruye completo desde los nodos. Es la base de
un futuro `REINDEX` y repara un índice desalineado por las vías que **no**
indexan a propósito: `add_nodes_batch` / `BulkLoader`, o escrituras directas
contra `Storage::insert_node`.

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
  preservado, canonicalización) y migración (legado→v2, idempotencia,
  crash-safety).