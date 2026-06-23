# Examples: Apache Arrow en NopalDB

## 🎯 Casos de Uso Reales

Este documento muestra ejemplos prácticos de cómo usar Arrow en escenarios reales.

---

## 📊 Ejemplo 1: Analytics Dashboard

### Objetivo
Crear un dashboard en tiempo real con métricas del grafo.

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./social_network").await?;
    
    // Export a Arrow
    let batch = graph.to_arrow().await?;
    
    println!("📊 Social Network Analytics Dashboard\n");
    println!("=" .repeat(50));
    
    // Métrica 1: Total de nodos
    let total_nodes = batch.num_rows();
    println!("📈 Total Nodes: {}", total_nodes);
    
    // Métrica 2: Distribución por label
    let labels = batch.column(1).as_string::<i32>();
    let mut label_counts = HashMap::new();
    
    for i in 0..labels.len() {
        *label_counts.entry(labels.value(i)).or_insert(0) += 1;
    }
    
    println!("\n🏷️  Node Types:");
    for (label, count) in label_counts.iter() {
        let percentage = (*count as f64 / total_nodes as f64) * 100.0;
        println!("   {}: {} ({:.1}%)", label, count, percentage);
    }
    
    // Métrica 3: Promedio de properties
    let prop_counts = batch.column(2).as_primitive::<arrow::datatypes::Int64Type>();
    let total_props: i64 = (0..prop_counts.len())
        .map(|i| prop_counts.value(i))
        .sum();
    let avg_props = total_props as f64 / total_nodes as f64;
    
    println!("\n📦 Average Properties: {:.2}", avg_props);
    
    // Export a Parquet para visualización externa
    graph.export_parquet("dashboard_snapshot.parquet").await?;
    println!("\n💾 Snapshot saved: dashboard_snapshot.parquet");
    
    Ok(())
}
```

### Output Esperado
```
📊 Social Network Analytics Dashboard

==================================================
📈 Total Nodes: 10000

🏷️  Node Types:
   Person: 7500 (75.0%)
   Company: 1500 (15.0%)
   Location: 1000 (10.0%)

📦 Average Properties: 4.2

💾 Snapshot saved: dashboard_snapshot.parquet
```

### Visualización en Python
```python
import pyarrow.parquet as pq
import pandas as pd
import matplotlib.pyplot as plt

# Leer Parquet
table = pq.read_table('dashboard_snapshot.parquet')
df = table.to_pandas()

# Gráfica de distribución
df['label'].value_counts().plot(kind='bar')
plt.title('Node Distribution')
plt.xlabel('Node Type')
plt.ylabel('Count')
plt.show()
```

---

## 🎮 Ejemplo 2: Game Analytics

### Objetivo
Analizar progreso de jugadores en un RPG.

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};

async fn analyze_player_progress(graph: &Graph) -> nopaldb::Result<()> {
    println!("🎮 Player Progress Analytics\n");
    
    // Export historial de versiones (MVCC)
    let history = graph.history_to_arrow().await?;
    
    println!("📜 Version History:");
    println!("   Total versions: {}", history.num_rows());
    
    // Analizar versiones por nodo
    let ids = history.column(0).as_string::<i32>();
    let versions = history.column(2).as_primitive::<arrow::datatypes::UInt64Type>();
    let timestamps = history.column(3).as_primitive::<arrow::datatypes::UInt64Type>();
    
    // Agrupar por ID
    let mut version_counts = std::collections::HashMap::new();
    for i in 0..ids.len() {
        let id = ids.value(i);
        *version_counts.entry(id).or_insert(0) += 1;
    }
    
    // Top 5 nodos más modificados (jugadores más activos)
    let mut sorted: Vec<_> = version_counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    
    println!("\n🏆 Most Active Players:");
    for (id, count) in sorted.iter().take(5) {
        println!("   Player {}: {} changes", &id[..8], count);
    }
    
    // Analizar frecuencia de updates
    let mut time_diffs = Vec::new();
    for i in 1..timestamps.len() {
        if ids.value(i) == ids.value(i - 1) {
            let diff = timestamps.value(i) - timestamps.value(i - 1);
            time_diffs.push(diff);
        }
    }
    
    if !time_diffs.is_empty() {
        let avg_time: u64 = time_diffs.iter().sum::<u64>() / time_diffs.len() as u64;
        println!("\n⏱️  Average time between updates: {}s", avg_time);
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./game_data").await?;
    analyze_player_progress(&graph).await?;
    Ok(())
}
```

### Output Esperado
```
🎮 Player Progress Analytics

📜 Version History:
   Total versions: 5420

🏆 Most Active Players:
   Player abc12345: 87 changes
   Player def67890: 76 changes
   Player ghi11121: 65 changes
   Player jkl31415: 58 changes
   Player mno16171: 52 changes

⏱️  Average time between updates: 342s
```

---

## 🧬 Ejemplo 3: Knowledge Graph Analysis

### Objetivo
Analizar un grafo de conocimiento científico.

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};

async fn analyze_knowledge_graph(graph: &Graph) -> nopaldb::Result<()> {
    println!("🧬 Knowledge Graph Analysis\n");
    
    // Export a Arrow
    let batch = graph.to_arrow().await?;
    
    // Análisis por categorías
    let labels = batch.column(1).as_string::<i32>();
    let prop_counts = batch.column(2).as_primitive::<arrow::datatypes::Int64Type>();
    
    // Calcular complejidad (más properties = más detallado)
    let mut category_complexity = std::collections::HashMap::new();
    
    for i in 0..labels.len() {
        let label = labels.value(i);
        let props = prop_counts.value(i);
        
        let entry = category_complexity.entry(label).or_insert((0, 0));
        entry.0 += 1;  // count
        entry.1 += props;  // total props
    }
    
    println!("📚 Category Complexity:");
    let mut sorted: Vec<_> = category_complexity.iter().collect();
    sorted.sort_by(|a, b| {
        let avg_a = a.1.1 as f64 / a.1.0 as f64;
        let avg_b = b.1.1 as f64 / b.1.0 as f64;
        avg_b.partial_cmp(&avg_a).unwrap()
    });
    
    for (category, (count, total_props)) in sorted.iter() {
        let avg = *total_props as f64 / *count as f64;
        println!("   {}: {:.1} avg properties ({} entities)", 
                 category, avg, count);
    }
    
    // Export para análisis ML
    graph.export_parquet("knowledge_graph.parquet").await?;
    println!("\n💾 Exported for ML analysis: knowledge_graph.parquet");
    
    Ok(())
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./scientific_kg").await?;
    analyze_knowledge_graph(&graph).await?;
    Ok(())
}
```

---

## 📈 Ejemplo 4: Time-Series Analysis

### Objetivo
Analizar cómo evoluciona el grafo en el tiempo (MVCC).

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};
use std::collections::HashMap;

async fn time_series_analysis(graph: &Graph) -> nopaldb::Result<()> {
    println!("📈 Time-Series Analysis\n");
    
    let history = graph.history_to_arrow().await?;
    
    let timestamps = history.column(3).as_primitive::<arrow::datatypes::UInt64Type>();
    let is_current = history.column(6).as_boolean();
    
    // Agrupar por timestamp (redondear a horas)
    let mut hourly_activity = HashMap::new();
    
    for i in 0..timestamps.len() {
        if !is_current.value(i) {
            continue;  // Solo contar versiones activas
        }
        
        let hour = timestamps.value(i) / 3600;  // Redondear a hora
        *hourly_activity.entry(hour).or_insert(0) += 1;
    }
    
    // Ordenar por hora
    let mut sorted: Vec<_> = hourly_activity.iter().collect();
    sorted.sort_by_key(|x| x.0);
    
    println!("🕐 Activity by Hour:");
    println!("   Hour | Activity | Graph");
    println!("   -----|----------|{}","-".repeat(30));
    
    for (hour, count) in sorted.iter() {
        let bar = "█".repeat(*count / 10);
        println!("   {:4} | {:8} | {}", hour, count, bar);
    }
    
    // Calcular picos de actividad
    let max_activity = sorted.iter().map(|x| x.1).max().unwrap_or(&0);
    let peak_hours: Vec<_> = sorted.iter()
        .filter(|x| x.1 == max_activity)
        .collect();
    
    println!("\n🔝 Peak Activity:");
    for (hour, count) in peak_hours {
        println!("   Hour {}: {} updates", hour, count);
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./time_series_data").await?;
    time_series_analysis(&graph).await?;
    Ok(())
}
```

### Output Esperado
```
📈 Time-Series Analysis

🕐 Activity by Hour:
   Hour | Activity | Graph
   -----|----------|------------------------------
     10 |      120 | ████████████
     11 |      340 | ██████████████████████████████████
     12 |      280 | ████████████████████████████
     13 |      150 | ███████████████
     14 |       90 | █████████

🔝 Peak Activity:
   Hour 11: 340 updates
```

---

## 🔍 Ejemplo 5: Anomaly Detection

### Objetivo
Detectar nodos anómalos usando estadísticas del grafo.

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};

async fn detect_anomalies(graph: &Graph) -> nopaldb::Result<()> {
    println!("🔍 Anomaly Detection\n");
    
    let batch = graph.to_arrow().await?;
    let prop_counts = batch.column(2).as_primitive::<arrow::datatypes::Int64Type>();
    
    // Calcular estadísticas
    let values: Vec<i64> = (0..prop_counts.len())
        .map(|i| prop_counts.value(i))
        .collect();
    
    let mean = values.iter().sum::<i64>() as f64 / values.len() as f64;
    
    let variance: f64 = values.iter()
        .map(|x| {
            let diff = *x as f64 - mean;
            diff * diff
        })
        .sum::<f64>() / values.len() as f64;
    
    let std_dev = variance.sqrt();
    
    println!("📊 Statistics:");
    println!("   Mean properties: {:.2}", mean);
    println!("   Std deviation: {:.2}", std_dev);
    
    // Detectar outliers (>2 std deviations)
    let threshold = mean + (2.0 * std_dev);
    
    let ids = batch.column(0).as_string::<i32>();
    let labels = batch.column(1).as_string::<i32>();
    
    println!("\n🚨 Anomalies Detected:");
    println!("   (Nodes with >{:.0} properties)\n", threshold);
    
    let mut anomaly_count = 0;
    for i in 0..prop_counts.len() {
        let count = prop_counts.value(i);
        if count as f64 > threshold {
            anomaly_count += 1;
            println!("   {} [{}]: {} properties",
                     &ids.value(i)[..8],
                     labels.value(i),
                     count);
        }
    }
    
    println!("\n   Total anomalies: {} ({:.1}%)",
             anomaly_count,
             (anomaly_count as f64 / values.len() as f64) * 100.0);
    
    Ok(())
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./network_data").await?;
    detect_anomalies(&graph).await?;
    Ok(())
}
```

---

## 💾 Ejemplo 6: Batch Export para Big Data

### Objetivo
Exportar grandes grafos a Parquet para procesamiento distribuido.

### Código
```rust
use nopaldb::{Graph, Node, PropertyValue};
use std::time::Instant;

async fn batch_export(graph: &Graph, output_dir: &str) -> nopaldb::Result<()> {
    println!("💾 Batch Export to Parquet\n");
    
    let start = Instant::now();
    
    // Export completo
    let batch = graph.to_arrow().await?;
    let export_time = start.elapsed();
    
    println!("📦 Exported {} nodes in {:?}", 
             batch.num_rows(), 
             export_time);
    
    // Calcular tamaño en memoria
    let memory_size = estimate_memory_size(&batch);
    println!("   Memory size: {:.2} MB", memory_size as f64 / 1_000_000.0);
    
    // Escribir a Parquet
    let parquet_path = format!("{}/export.parquet", output_dir);
    graph.export_parquet(&parquet_path).await?;
    
    // Comparar tamaños
    let file_size = std::fs::metadata(&parquet_path)?.len();
    let compression_ratio = memory_size as f64 / file_size as f64;
    
    println!("\n💽 Parquet File:");
    println!("   Path: {}", parquet_path);
    println!("   Size: {:.2} MB", file_size as f64 / 1_000_000.0);
    println!("   Compression: {:.1}x", compression_ratio);
    
    // Export historial (si existe)
    if let Ok(history) = graph.history_to_arrow().await {
        let history_path = format!("{}/history.parquet", output_dir);
        nopaldb::arrow_export::write_parquet(&history, &history_path)?;
        
        let history_size = std::fs::metadata(&history_path)?.len();
        println!("\n📜 History File:");
        println!("   Path: {}", history_path);
        println!("   Size: {:.2} MB", history_size as f64 / 1_000_000.0);
        println!("   Versions: {}", history.num_rows());
    }
    
    Ok(())
}

fn estimate_memory_size(batch: &arrow::record_batch::RecordBatch) -> usize {
    batch.columns().iter()
        .map(|col| col.get_array_memory_size())
        .sum()
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::open("./large_graph").await?;
    
    std::fs::create_dir_all("./exports")?;
    batch_export(&graph, "./exports").await?;
    
    println!("\n✅ Batch export complete!");
    println!("   Ready for: Spark, Dask, DuckDB");
    
    Ok(())
}
```

### Output Esperado
```
💾 Batch Export to Parquet

📦 Exported 1000000 nodes in 2.34s
   Memory size: 45.23 MB

💽 Parquet File:
   Path: ./exports/export.parquet
   Size: 12.45 MB
   Compression: 3.6x

📜 History File:
   Path: ./exports/history.parquet
   Size: 34.56 MB
   Versions: 3500000

✅ Batch export complete!
   Ready for: Spark, Dask, DuckDB
```

---

## 🎯 Casos de Uso por Industria

### 🎮 Gaming
- Player analytics
- Quest progression
- Skill tree optimization
- Leaderboards

### 📱 Mobile Apps
- Social graph analytics
- User behavior patterns
- Feature usage
- A/B testing

### 🏢 Organizations
- Organization charts
- Dependency graphs
- Resource allocation
- Impact analysis

### 🧬 Knowledge Work
- Citation networks
- Protein interactions
- Knowledge graphs
- Collaboration patterns

### 💰 Finance
- Transaction networks
- Risk analysis
- Fraud detection
- Portfolio dependencies

---

## 📚 Próximo Paso

- [ML Integration](05-ML-INTEGRATION.md) - PyTorch y TensorFlow

---

**Creado con 🦀 Rust & ❤️ en 🇲🇽**
