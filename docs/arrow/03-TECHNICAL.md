# Technical Details: Apache Arrow

## 🏗️ Arquitectura Interna

### Memory Layout

Arrow usa un formato de memoria **altamente optimizado** para CPUs modernas.

#### Array Structure
```
Arrow Array en Memoria:

┌─────────────────────────────────────────────────┐
│ Validity Buffer (bitmap)                        │
│ [1, 1, 0, 1, 1]  ← 1 bit por valor             │
│ Indica si valor es NULL o válido                │
└─────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────┐
│ Data Buffer (valores)                           │
│ [25, 30, ?, 28, 35]  ← Valores reales          │
│ Contiguos en memoria para SIMD                  │
└─────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────┐
│ Offset Buffer (para strings/binary)            │
│ [0, 5, 8, 13]  ← Inicio de cada string         │
└─────────────────────────────────────────────────┘
```

**Ventajas:**
- ✅ Datos contiguos → Cache-friendly
- ✅ Bitmap de validez → NULL handling eficiente
- ✅ Offsets → Strings de longitud variable
- ✅ Alineación de 64 bytes → SIMD optimization

### RecordBatch Structure
```rust
RecordBatch {
    schema: Arc<Schema>,      // Metadata compartido
    columns: Vec<ArrayRef>,   // Columnas (zero-copy)
    num_rows: usize,          // Cantidad de filas
}

Schema {
    fields: Vec<Field>,       // Definición de columnas
    metadata: HashMap<>,      // Metadata adicional
}

Field {
    name: String,             // Nombre de columna
    data_type: DataType,      // Tipo (Int64, Utf8, etc)
    nullable: bool,           // ¿Permite NULL?
}
```

**Ejemplo en NopalDB:**
```rust
// Schema de nodos
Schema {
    fields: [
        Field { name: "id", data_type: Utf8, nullable: false },
        Field { name: "label", data_type: Utf8, nullable: false },
        Field { name: "property_count", data_type: Int64, nullable: false },
    ]
}

// Schema de nodos versionados (MVCC)
Schema {
    fields: [
        Field { name: "id", data_type: Utf8, nullable: false },
        Field { name: "label", data_type: Utf8, nullable: false },
        Field { name: "version", data_type: UInt64, nullable: false },
        Field { name: "timestamp", data_type: UInt64, nullable: false },
        Field { name: "valid_from", data_type: UInt64, nullable: false },
        Field { name: "valid_to", data_type: UInt64, nullable: true },  // NULL = current
        Field { name: "is_current", data_type: Boolean, nullable: false },
    ]
}
```

## ⚡ SIMD Optimizations

### Qué es SIMD

**SIMD** = Single Instruction, Multiple Data
```
CPU Normal (Scalar):
ages[0] = 25
ages[1] = 30    } 4 operaciones separadas
ages[2] = 28
ages[3] = 35

CPU con SIMD (Vector):
[25, 30, 28, 35]  } 1 operación para 4 valores
```

### Cómo Arrow usa SIMD
```rust
// Sin SIMD (loop tradicional)
let mut sum = 0;
for age in ages {
    sum += age;
}
// Tiempo: O(n)

// Con el kernel de Arrow (vectorizable por el compilador)
use arrow::compute::sum;
let total = sum(&ages);          // ages: &Int64Array → Option<i64>
```

**Instrucciones SIMD usadas:**
- SSE2: 128-bit (4 × i32)
- AVX2: 256-bit (8 × i32)
- AVX-512: 512-bit (16 × i32)

El aprovechamiento real de SIMD depende del CPU, del tipo y de las flags de
compilación; NopalDB no publica mediciones propias de estos kernels.

## 🗜️ Compression (Parquet)

### Niveles de Compression

Parquet usa **múltiples técnicas de compresión**:

#### 1. Dictionary Encoding
```
Original:
["Person", "Person", "Person", "Person", "Company", "Person"]
Size: 6 × 10 bytes = 60 bytes

Dictionary Encoded:
Dictionary: ["Person" → 0, "Company" → 1]
Values: [0, 0, 0, 0, 1, 0]
Size: 2 × 10 bytes + 6 × 1 byte = 26 bytes

Compression: 43% del tamaño original
```

#### 2. Run-Length Encoding (RLE)
```
Original:
[1, 1, 1, 1, 1, 2, 2, 3, 3, 3]
Size: 10 × 4 bytes = 40 bytes

RLE Encoded:
[(1, count=5), (2, count=2), (3, count=3)]
Size: 3 × 8 bytes = 24 bytes

Compression: 60% del tamaño original
```

#### 3. Bit Packing
```
Values: [0, 1, 2, 3, 4, 5, 6, 7]
Max value: 7 (requires 3 bits)

Normal: 8 × 32 bits = 256 bits
Bit-packed: 8 × 3 bits = 24 bits

Compression: 9% del tamaño original
```

#### 4. SNAPPY Compression
```
After dictionary + RLE + bit packing:
Apply SNAPPY (general-purpose compression)

Typical results:
- Text data: 50-70% compression
- Numeric data: 30-50% compression
- Mixed data: 40-60% compression
```

### Parquet File Structure
```
Parquet File:
┌─────────────────────────────────────────┐
│ Magic Number: "PAR1"                    │
├─────────────────────────────────────────┤
│ Row Group 1                             │
│  ├─ Column Chunk: id                    │
│  │   ├─ Page 1 (compressed)            │
│  │   └─ Page 2 (compressed)            │
│  ├─ Column Chunk: label                 │
│  │   └─ Page 1 (compressed)            │
│  └─ Column Chunk: property_count        │
│      └─ Page 1 (compressed)            │
├─────────────────────────────────────────┤
│ Row Group 2                             │
│  └─ ...                                 │
├─────────────────────────────────────────┤
│ Footer (metadata)                       │
│  ├─ Schema                              │
│  ├─ Row group metadata                  │
│  └─ Statistics (min, max, count)       │
├─────────────────────────────────────────┤
│ Magic Number: "PAR1"                    │
└─────────────────────────────────────────┘
```

**Ventajas:**
- ✅ Columnar → Solo lee columnas necesarias
- ✅ Metadata en footer → Skip irrelevant data
- ✅ Statistics → Predicate pushdown
- ✅ Compression → Menor I/O

## 🔄 Arrow IPC (Rust → Python)

**No es zero-copy.** El wrapper de Python (`src/python/arrow.rs`) construye el
`RecordBatch` en Rust, lo serializa a un stream Arrow IPC (`StreamWriter`) y
copia esos bytes a un `bytes` de Python. `pyarrow.ipc.open_stream` los lee
sin conversión fila por fila, pero la memoria de Rust y la de Python son
distintas:

```
Rust RecordBatch ──serializa──→ Vec<u8> (IPC) ──copia──→ bytes (Python) ──→ pyarrow.Table
```

Lo que el formato ahorra es la conversión por fila (no hay dicts ni JSON de
por medio). A partir de la `pyarrow.Table`, `to_pandas()` vuelve a copiar
(`zero_copy_only=True` falla con varias columnas).

### Arrow IPC Format
```
Arrow IPC Stream:
┌─────────────────────────────────────────┐
│ Schema Message                          │
│  - Field definitions                    │
│  - Metadata                             │
├─────────────────────────────────────────┤
│ RecordBatch Message 1                   │
│  - Metadata + buffers de cada columna   │
├─────────────────────────────────────────┤
│ RecordBatch Message 2 (opcional)        │
│  - ...                                  │
└─────────────────────────────────────────┘
```

**Implementación en NopalDB:**
```rust
// Lado Rust (lo que hace el wrapper; también sirve para escribir un archivo)
use arrow::ipc::writer::StreamWriter;
let batch = graph.to_arrow().await?;
let mut buf = Vec::new();
let mut writer = StreamWriter::try_new(&mut buf, &batch.schema())?;
writer.write(&batch)?;
writer.finish()?;
```

```python
# Lado Python: el wrapper ya devuelve los bytes del stream
import pyarrow as pa
table = pa.ipc.open_stream(graph.to_arrow()).read_all()
```

## 📊 Data Types

### Arrow Type System
```rust
DataType::Utf8              // String variable
DataType::LargeUtf8         // String >4GB
DataType::Int8              // i8
DataType::Int16             // i16
DataType::Int32             // i32
DataType::Int64             // i64
DataType::UInt8             // u8
DataType::UInt16            // u16
DataType::UInt32            // u32
DataType::UInt64            // u64
DataType::Float32           // f32
DataType::Float64           // f64
DataType::Boolean           // bool
DataType::Binary            // Vec<u8>
DataType::Date32            // Days since UNIX epoch
DataType::Date64            // Milliseconds since UNIX epoch
DataType::Timestamp         // Nanoseconds with timezone
DataType::List              // Vec<T>
DataType::Struct            // Record/Tuple
DataType::Map               // HashMap
```

### NopalDB Type Mappings
```rust
// Node → Arrow
Node.id         → Utf8
Node.label      → Utf8
Node.properties → Int64 (count)

// VersionedNode → Arrow
VersionedNode.id         → Utf8
VersionedNode.label      → Utf8
VersionedNode.version    → UInt64
VersionedNode.timestamp  → UInt64
VersionedNode.valid_from → UInt64
VersionedNode.valid_to   → UInt64 (nullable)
VersionedNode.is_current → Boolean

```

### Vista analítica con propiedades (`to_arrow_complete`, `to_arrow_with_label`)

`graph_to_arrow` (Python `to_arrow_complete`) y `nodes_to_arrow_with_properties`
dan una **vista analítica**, no un intercambio reversible del grafo. Lo que
promete cada lote:

| Qué | Contrato |
|---|---|
| Subgrafo por etiqueta | Con `label`, solo las aristas con **ambos** extremos en los nodos exportados (subgrafo inducido). Se conservan los self-loops y las aristas paralelas. Hasta 0.9.0 salían todas las aristas del grafo |
| Orden de columnas | `id`, `label` (o `id`, `source`, `target`, `edge_type` en aristas) y después una columna por propiedad, **ordenadas por nombre**. El mismo grafo da el mismo esquema, sin importar el orden de inserción |
| Tipo de una columna de nodo | Si la propiedad tiene un solo tipo escalar en todos los nodos, ese tipo: `Int64`, `Float64`, `Boolean` o `Utf8`. Si mezcla tipos, o tiene listas, objetos o bytes, `Utf8` |
| Celda de texto | El escalar tal cual. Listas, objetos y bytes van **en JSON** (los bytes, como arreglo de números). Hasta 0.9.0 eran `<complex>` / `<bytes:N>`, y un valor de otro tipo en una columna entera salía nulo |
| Columnas de aristas | `Utf8`, con las mismas reglas de celda |
| `Null` y propiedad ausente | Nulo de Arrow en los dos casos (antes `Null` era el texto `"null"`). Esta vista no los distingue |
| Propiedad solo con `Null` | Si en todos los nodos exportados la propiedad vale `Null` (o falta), **no tiene columna**: sin un valor no hay tipo que inferir |
| Nombre reservado | Una propiedad llamada como una columna estructural (`id`, `label`, `source`, `target`, `edge_type`) sale como `property.<nombre>` |
| Lote vacío | Sin nodos (o sin nodos de esa etiqueta) devuelve un lote vacío con su esquema, como las aristas, en vez de un error |

No es un respaldo ni un intercambio tipado: los tipos de una columna mixta
se pierden al pasar a texto, y un `Null` no se distingue de una propiedad
ausente. Un perfil de intercambio tipado y versionado es trabajo aparte.


## 🧮 Memory Efficiency

### Memory Comparison
```
JSON (100 nodes, avg 3 properties):
{
  "id": "abc...",
  "label": "Person",
  "properties": { "name": "Alice", "age": 25, ... }
}
Size: ~200 bytes per node × 100 = 20KB

Arrow (same data):
- ID column: 100 × 36 bytes = 3.6KB
- Label column: 100 × 10 bytes = 1KB
- Property count: 100 × 8 bytes = 0.8KB
Total: 5.4KB (27% del tamaño JSON)

Parquet (same data, compressed):
Total: ~2KB (10% del tamaño JSON)
```

### Cache Efficiency
```
CPU Cache Lines: 64 bytes

Row-oriented (JSON):
Cache line 1: Node 1 (partial)
Cache line 2: Node 1 (rest) + Node 2 (partial)
Cache line 3: Node 2 (rest)
→ 3 cache lines para 2 nodes

Columnar (Arrow):
Cache line 1: 8 × Int64 ages
Cache line 2: 8 × Int64 ages
→ 2 cache lines para 16 ages

Cache utilization: 8× mejor
```

## 🔬 Micro-Optimizations

### Padding and Alignment
```rust
// Arrow garantiza alineación de 64 bytes
#[repr(C, align(64))]
struct Buffer {
    data: *mut u8,
    len: usize,
}

// Beneficios:
// - SIMD instrucciones requieren alineación
// - Cache line alignment → menos cache misses
// - Vectorization automática del compilador
```

### Filtrar y proyectar
```rust
// arrow-rs es eager: cada kernel produce un array nuevo.
use arrow::array::AsArray;
use arrow::compute::{filter_record_batch, kernels::cmp::gt};
use arrow::array::Int64Array;
use arrow::datatypes::Int64Type;

let ages = batch.column_by_name("age").unwrap().as_primitive::<Int64Type>();
let mask = gt(ages, &Int64Array::new_scalar(25))?;
let filtered = filter_record_batch(&batch, &mask)?;
let projected = filtered.project(&[0, 2])?;   // por índice de columna
```
(Evaluación diferida y optimización de consultas son de motores como
DataFusion o Polars, no de arrow-rs.)

## 📈 Performance Characteristics

### Complexity Analysis

| Operation | Row-Oriented | Arrow Columnar |
|-----------|--------------|----------------|
| Read column | O(n) | O(1) |
| Filter | O(n) | O(n/k)* |
| Aggregate | O(n) | O(n/k)* |
| Sort | O(n log n) | O(n log n) |
| Join | O(n²) | O(n log n)** |

*k = SIMD width (4-16)  
**Con sorted columns

No hay benchmarks de estas operaciones publicados para NopalDB.

## 🛠️ Implementation in NopalDB

### Conversion Pipeline
```
NopalDB Storage (redb)
         ↓
    Get all nodes
         ↓
    Vec<Node>
         ↓
Extract columns (ids, labels, counts)
         ↓
    StringArray, StringArray, Int64Array
         ↓
    RecordBatch
         ↓
    Arrow Format (memory)
         ↓
    [Optional] Write Parquet (disk)
```

El código vive en `nopaldb/src/arrow_export/mod.rs` (`nodes_to_arrow`,
`nodes_to_arrow_with_properties`, `edges_to_arrow`, `graph_to_arrow`,
`versioned_nodes_to_arrow`, `write_parquet`, `read_parquet`) y en los métodos
de `Graph` (`to_arrow`, `to_arrow_with_label`, `history_to_arrow`,
`export_parquet`, `import_parquet`).

Las propiedades como columnas ya existen: `graph.to_arrow_with_label(Some("Person"))`
(o `nodes_to_arrow_with_properties`) da `id | label | <propiedades por nombre>`,
con el contrato de la [vista analítica](#vista-analítica-con-propiedades-to_arrow_complete-to_arrow_with_label).
No existen exportación incremental, exportación con filtro arbitrario ni
exportación paralela.

## 📚 References

- [Apache Arrow Format](https://arrow.apache.org/docs/format/Columnar.html)
- [Arrow Rust Docs](https://docs.rs/arrow/)
- [Parquet Format](https://parquet.apache.org/docs/)
- [SIMD in Rust](https://rust-lang.github.io/packed_simd/packed_simd/)

---

**Siguiente: [Examples](04-EXAMPLES.md)** - Casos de uso reales

---

**Creado con 🦀 Rust & ❤️ en 🇲🇽**