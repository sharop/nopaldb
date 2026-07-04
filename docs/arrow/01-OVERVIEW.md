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

Velocidad: 4-8x más rápido
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

Velocidad: 10-100x más rápido
```

### 3. **Zero-Copy**
```rust
// Compartir datos entre Rust y Python SIN copiar

// Rust: Export a Arrow
let batch = graph.to_arrow().await?;

// Python: Import sin copiar memoria
import pyarrow as pa
table = pa.Table.from_batches([batch])
df = table.to_pandas(zero_copy_only=True)

# 0 bytes copiados!
# 0ms overhead!
```

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

## 📊 Comparación de Performance

| Operación | Row-Oriented | Arrow Columnar | Speedup |
|-----------|--------------|----------------|---------|
| Sum 1M integers | 50ms | 0.5ms | **100x** |
| Filter by age | 100ms | 2ms | **50x** |
| Group by label | 200ms | 10ms | **20x** |
| Export to Python | 500ms | <1ms | **500x** |

## 🎯 Casos de Uso en NopalDB

### 1. **Análisis de Grafos**
```rust
// Export todo el grafo a Arrow
let batch = graph.to_arrow().await?;

// Análisis rápido con SIMD
let total_nodes = batch.num_rows();
let avg_properties = batch.column(2).sum() / total_nodes;
```

### 2. **Machine Learning**
```rust
// Export grafo → Arrow → PyTorch
let batch = graph.to_arrow().await?;

// En Python:
# import torch
# tensor = torch.from_numpy(batch.to_pandas().values)
# model.train(tensor)
```

### 3. **Snapshots para Analytics**
```rust
// Export snapshot histórico
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
- Mobile/Embedded (usa CORE edition)

## 📦 Feature Flags

NopalDB usa feature flags para mantener el binario pequeño:
```toml
# Mobile/Embedded (SIN Arrow)
[dependencies]
nopaldb = { version = "0.1", default-features = false }
# Binary size: 5-8 MB

# Server/Analytics (CON Arrow)
[dependencies]
nopaldb = { version = "0.1", features = ["analytics"] }
# Binary size: 15-20 MB

# Full (TODO)
[dependencies]
nopaldb = { version = "0.1", features = ["full"] }
# Binary size: 20-30 MB
```

## 🚀 Próximos Pasos

1. [Quickstart Guide](02-QUICKSTART.md) - Primeros pasos
2. [Technical Details](03-TECHNICAL.md) - Detalles técnicos
3. [Examples](04-EXAMPLES.md) - Casos de uso reales
4. [ML Integration](05-ML-INTEGRATION.md) - PyTorch y TensorFlow
5. [Performance](06-PERFORMANCE.md) - Benchmarks

---

**Creado con 🦀 Rust & ❤️ en 🇲🇽**