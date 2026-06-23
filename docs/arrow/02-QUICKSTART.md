# Quickstart: Apache Arrow en NopalDB

## ⚡ 5 Minutos para Empezar

Este tutorial te enseña a usar Arrow en NopalDB en menos de 5 minutos.

## 📋 Prerequisitos
```toml
# Cargo.toml
[dependencies]
nopaldb = { version = "0.1", features = ["analytics"] }
tokio = { version = "1", features = ["full"] }
```

## 🚀 Ejemplo Básico

### Paso 1: Crear Grafo con Datos
```rust
use nopaldb::{Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    // Crear grafo
    let graph = Graph::in_memory().await?;
    
    // Agregar nodos
    let mut tx = graph.begin_transaction().await?;
    
    for i in 0..100 {
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("User{}", i)))
            .with_property("age", PropertyValue::Int(20 + (i % 50)))
            .with_property("active", PropertyValue::Bool(i % 2 == 0));
        
        tx.add_node(node).await?;
    }
    
    tx.commit().await?;
    
    println!("✅ Created 100 nodes");
    
    Ok(())
}
```

### Paso 2: Export a Arrow
```rust
// Export todo el grafo a formato columnar
let batch = graph.to_arrow().await?;

println!("📦 Arrow RecordBatch:");
println!("   Rows: {}", batch.num_rows());
println!("   Columns: {}", batch.num_columns());

// Output:
// 📦 Arrow RecordBatch:
//    Rows: 100
//    Columns: 3
```

### Paso 3: Inspeccionar Schema
```rust
let schema = batch.schema();

println!("\n📋 Schema:");
for field in schema.fields() {
    println!("   - {}: {:?}", field.name(), field.data_type());
}

// Output:
// 📋 Schema:
//    - id: Utf8
//    - label: Utf8
//    - property_count: Int64
```

### Paso 4: Acceder a Columnas
```rust
use arrow::array::AsArray;

// Acceder a columna específica
let labels = batch.column(1).as_string::<i32>();

println!("\n🏷️  Labels:");
for i in 0..5 {
    println!("   {}: {}", i, labels.value(i));
}

// Output:
// 🏷️  Labels:
//    0: Person
//    1: Person
//    2: Person
//    3: Person
//    4: Person
```

## 💾 Export a Parquet
```rust
// Guardar en archivo Parquet
graph.export_parquet("snapshot.parquet").await?;

println!("✅ Exported to Parquet: snapshot.parquet");

// Verificar tamaño
let metadata = std::fs::metadata("snapshot.parquet")?;
println!("   File size: {} bytes", metadata.len());

// Output:
// ✅ Exported to Parquet: snapshot.parquet
//    File size: 2048 bytes
```

## ⏰ MVCC: Export Historia
```rust
// Crear nodo con versiones
let node_id = {
    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Counter")
        .with_property("value", PropertyValue::Int(0));
    let id = tx.add_node(node).await?;
    tx.commit().await?;
    id
};

// Update (crea v2)
tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
{
    let mut tx = graph.begin_transaction().await?;
    let mut node = graph.get_node(node_id).await?;
    node.properties.insert("value".into(), PropertyValue::Int(100));
    tx.add_node(node).await?;
    tx.commit().await?;
}

// Update (crea v3)
tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
{
    let mut tx = graph.begin_transaction().await?;
    let mut node = graph.get_node(node_id).await?;
    node.properties.insert("value".into(), PropertyValue::Int(200));
    tx.add_node(node).await?;
    tx.commit().await?;
}

// Export historial completo
let history_batch = graph.history_to_arrow().await?;

println!("\n📜 History RecordBatch:");
println!("   Versions: {}", history_batch.num_rows());
println!("   Columns: {}", history_batch.num_columns());

// Output:
// 📜 History RecordBatch:
//    Versions: 3
//    Columns: 7
```

## 🐍 Interoperabilidad con Python

### Rust: Export a Parquet
```rust
// En Rust
graph.export_parquet("my_graph.parquet").await?;
```

### Python: Import y Análisis
```python
import pyarrow.parquet as pq
import pandas as pd

# Leer Parquet (zero-copy)
table = pq.read_table('my_graph.parquet')
df = table.to_pandas()

# Análisis
print(df.head())
print(df.describe())

# Filtrar
active_users = df[df['label'] == 'Person']
print(f"Total persons: {len(active_users)}")
```

## 📊 Ejemplo Completo
```rust
use nopaldb::{Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    // 1. Crear grafo
    let graph = Graph::in_memory().await?;
    
    // 2. Insertar datos
    let mut tx = graph.begin_transaction().await?;
    for i in 0..1000 {
        let node = Node::new("User")
            .with_property("id", PropertyValue::Int(i))
            .with_property("score", PropertyValue::Int(i * 10));
        tx.add_node(node).await?;
    }
    tx.commit().await?;
    println!("✅ Inserted 1000 nodes");
    
    // 3. Export a Arrow
    let batch = graph.to_arrow().await?;
    println!("📦 Exported to Arrow: {} rows", batch.num_rows());
    
    // 4. Export a Parquet
    graph.export_parquet("users.parquet").await?;
    println!("💾 Saved to Parquet: users.parquet");
    
    // 5. Verificar
    let metadata = std::fs::metadata("users.parquet")?;
    println!("   Size: {} bytes", metadata.len());
    println!("   Compression: ~70% (estimated)");
    
    Ok(())
}
```

### Output Esperado
```
✅ Inserted 1000 nodes
📦 Exported to Arrow: 1000 rows
💾 Saved to Parquet: users.parquet
   Size: 4096 bytes
   Compression: ~70% (estimated)
```

## 🎯 Casos de Uso Rápidos

### Analytics: Contar por Label
```rust
let batch = graph.to_arrow().await?;
let labels = batch.column(1).as_string::<i32>();

let mut counts = std::collections::HashMap::new();
for i in 0..labels.len() {
    let label = labels.value(i);
    *counts.entry(label).or_insert(0) += 1;
}

println!("📊 Label counts:");
for (label, count) in counts {
    println!("   {}: {}", label, count);
}
```

### Time-Travel: Ver Cambios
```rust
let history = graph.history_to_arrow().await?;
let versions = history.column(2).as_primitive::<arrow::datatypes::UInt64Type>();
let timestamps = history.column(3).as_primitive::<arrow::datatypes::UInt64Type>();

println!("⏰ Version timeline:");
for i in 0..versions.len() {
    println!(
        "   v{} at t={}",
        versions.value(i),
        timestamps.value(i)
    );
}
```

### Snapshot: Guardar Estado
```rust
use std::time::{SystemTime, UNIX_EPOCH};

let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_secs();

let filename = format!("snapshot_{}.parquet", timestamp);
graph.export_parquet(&filename).await?;

println!("📸 Snapshot saved: {}", filename);
```

## 🚨 Errores Comunes

### Error 1: Feature no habilitada
```toml
# ❌ INCORRECTO
[dependencies]
nopaldb = "0.1"

# ✅ CORRECTO
[dependencies]
nopaldb = { version = "0.1", features = ["analytics"] }
```

### Error 2: Grafo vacío
```rust
let graph = Graph::in_memory().await?;

// ❌ Error: Cannot convert empty node list
let batch = graph.to_arrow().await?;

// ✅ Primero agregar nodos
let mut tx = graph.begin_transaction().await?;
tx.add_node(Node::new("Test")).await?;
tx.commit().await?;

let batch = graph.to_arrow().await?; // OK
```

### Error 3: Historial sin versiones
```rust
// ❌ Error: No versioned nodes found
let history = graph.history_to_arrow().await?;

// ✅ Primero crear versiones con transactions
let mut tx = graph.begin_transaction().await?;
// ... add/update nodes ...
tx.commit().await?;
```

## 🔧 Compilación
```bash
# Compilar con Arrow
cargo build --release --features analytics

# Tests
cargo test --features analytics

# Binary size
ls -lh target/release/nopaldb
# ~15-20 MB con analytics
```

## 📚 Siguiente Paso

- [Technical Details](03-TECHNICAL.md) - Detalles profundos
- [Examples](04-EXAMPLES.md) - Casos de uso avanzados
- [ML Integration](05-ML-INTEGRATION.md) - PyTorch y TensorFlow

## 💡 Tips

1. **Use Parquet para persistencia**: Más eficiente que JSON
2. **Export por lotes**: Para grafos grandes (>1M nodos)
3. **Combine con MVCC**: Time-travel analytics poderoso
4. **Zero-copy cuando posible**: Usa PyArrow directamente

---

**¿Listo para más?** 🚀

Continúa con los [detalles técnicos](03-TECHNICAL.md) para entender cómo funciona Arrow internamente.

---

**Creado con 🦀 Rust & ❤️ en 🇲🇽**