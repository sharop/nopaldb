# Apache Arrow en NopalDB

## 🎯 ¿Qué es Apache Arrow?

Apache Arrow es un **formato de datos columnar en memoria** diseñado para análisis de alto rendimiento. En lugar de almacenar datos fila por fila (como JSON o CSV), Arrow los organiza **columna por columna**.

## 🔄 Row-Oriented vs Columnar

### Row-Oriented (Tradicional)
```
Storage en Memoria:
┌─────────────────────────────────────────┐
│ Node 1: {id: "abc", name: "Alice", age: 25} │
│ Node 2: {id: "def", name: "Bob", age: 30}   │
│ Node 3: {id: "ghi", name: "Carol", age: 28} │
└─────────────────────────────────────────┘

Query: "Calcular promedio de edad"
Proceso:
1. Lee Node 1 COMPLETO (id + name + age)
2. Extrae age (25)
3. Lee Node 2 COMPLETO
4. Extrae age (30)
5. Lee Node 3 COMPLETO
6. Extrae age (28)
7. Calcula: (25 + 30 + 28) / 3 = 27.67

Datos leídos: 3 nodos completos (mucho desperdicio)
```

### Columnar (Apache Arrow)
```
Storage en Memoria:
┌──────────┬──────────┬──────────┐
│ IDs      │ Names    │ Ages     │
├──────────┼──────────┼──────────┤
│ "abc"    │ "Alice"  │ 25       │
│ "def"    │ "Bob"    │ 30       │
│ "ghi"    │ "Carol"  │ 28       │
└──────────┴──────────┴──────────┘

En memoria (contiguous arrays):
ids:   ["abc", "def", "ghi"]
names: ["Alice", "Bob", "Carol"]
ages:  [25, 30, 28]  ← SOLO leemos esta columna

Query: "Calcular promedio de edad"
Proceso:
1. Lee columna ages completa: [25, 30, 28]
2. SIMD: suma todos a la vez → 83
3. Divide por 3 → 27.67

Datos leídos: SOLO la columna ages (eficiente)
```

## ⚡ Ventajas de Arrow

### 1. **SIMD (Single Instruction Multiple Data)**
```
Sin SIMD (procesamiento secuencial):
ages[0] + ages[1] + ages[2]
→ 3 operaciones

Con SIMD (procesamiento paralelo):
sum(ages)  # CPU procesa 4-8 valores a la vez
→ 1 operación

Velocidad: depende del CPU y del tipo de dato (ilustrativo, no medido en NopalDB)
```

### 2. **Cache-Friendly**
```
Row-Oriented:
Lee Node 1 → Cache miss
Lee Node 2 → Cache miss
Lee Node 3 → Cache miss

Columnar:
Lee ages completa → Datos contiguos en memoria
→ Cache hit rate alto
→ Menos accesos a RAM

Velocidad: depende del acceso (ilustrativo, no medido en NopalDB)
```

### 3. **Un formato estándar entre lenguajes**

Entre Rust y Python **no hay zero-copy**: el wrapper serializa el
`RecordBatch` a un stream Arrow IPC y Python recibe esos bytes (una copia).
Lo que sí ganas es no convertir fila por fila: `pyarrow` lee el stream
directamente.

```python
import pyarrow as pa
import nopaldb

graph = nopaldb.Graph.open("./mi_grafo")
nodes_ipc, edges_ipc = graph.to_arrow_complete("Person")   # bytes (Arrow IPC)
nodes = pa.ipc.open_stream(nodes_ipc).read_all()            # pyarrow.Table
df = nodes.to_pandas()                                      # copia a pandas
```

Ver [docs/python/ARROW_EXPORT.md](../python/ARROW_EXPORT.md).

### 4. **Interoperabilidad**
```
Arrow → Pandas (Python)
Arrow → PyTorch (ML)
Arrow → TensorFlow (ML)
Arrow → Polars (Rust DataFrames)
Arrow → DuckDB (SQL)
Arrow → DataFusion (SQL)
Arrow → Spark (Big Data)
```

## 📊 Performance

No hay benchmarks de exportación Arrow publicados para NopalDB; las cifras
de las secciones anteriores son ilustrativas del formato columnar en
general, no mediciones de este proyecto.

## 🎯 Casos de Uso en NopalDB

### 1. **Análisis de Grafos**
```rust
// Export todo el grafo a Arrow
let batch = graph.to_arrow().await?;

// Columna 2 = property_count (Int64). Requiere `arrow = "59"` en tu Cargo.toml.
use arrow::array::AsArray;
use arrow::compute::sum;
use arrow::datatypes::Int64Type;

let total_nodes = batch.num_rows();
let total_props = sum(batch.column(2).as_primitive::<Int64Type>()).unwrap_or(0);
let avg_properties = total_props as f64 / total_nodes as f64;
```

### 2. **Machine Learning**
```python
# Desde Python: grafo → Arrow IPC → pyarrow → numpy/PyTorch
import pyarrow as pa
nodes = pa.ipc.open_stream(graph.to_arrow("Person")).read_all()
ages = nodes.column("age").to_numpy()      # columna de una propiedad
# torch.from_numpy(ages) ...
```

### 3. **Snapshots para Analytics**
```rust
// Snapshot de metadatos (id, label, property_count; sin propiedades ni aristas)
graph.export_parquet("snapshot_2024_01.parquet").await?;

// Análisis en DuckDB:
// SELECT label, COUNT(*) FROM 'snapshot_2024_01.parquet'
// GROUP BY label;
```

### 4. **Time-Travel Analytics**
```rust
// Export historial MVCC
let history = graph.history_to_arrow().await?;

// Análisis temporal:
// - Cuántas versiones por nodo
// - Frecuencia de updates
// - Patrones de cambio
```

## 🔧 Cuándo Usar Arrow

### ✅ **Usar Arrow cuando:**
- Análisis masivo de datos (>10K nodos)
- Machine Learning pipelines
- Exportar a Python/Pandas
- Agregaciones (SUM, AVG, COUNT)
- Dashboards y visualizaciones
- Data warehousing

### ❌ **NO usar Arrow cuando:**
- Operaciones transaccionales (usa Graph API)
- Queries de un solo nodo (usa get_node)
- Updates frecuentes (usa Transaction)
- Binarios mínimos: Arrow/Parquet vienen con la feature `analytics`
  (incluida en `core`); sin ella no se compilan

## 📦 Feature Flags

NopalDB usa feature flags para mantener el binario pequeño:
```toml
# Mínimo (SIN Arrow): grafo + NQL + MVCC + WAL con redb
[dependencies]
nopaldb = "0.9"

# Con Arrow/Parquet (tier core: + algoritmos, embeddings, full-text)
[dependencies]
nopaldb = { version = "0.9", features = ["core"] }

# Todo (semantic + full-isolation)
[dependencies]
nopaldb = { version = "0.9", features = ["full"] }
```

## 🚀 Próximos Pasos

1. [Quickstart Guide](02-QUICKSTART.md) - Primeros pasos
2. [Technical Details](03-TECHNICAL.md) - Detalles técnicos
3. [Examples](04-EXAMPLES.md) - Casos de uso reales
4. [ML Integration](05-ML-INTEGRATION.md) - PyTorch y TensorFlow
5. [Performance](06-PERFORMANCE.md) - Benchmarks

---

**Creado con 🦀 Rust & ❤️ en 🇲🇽**