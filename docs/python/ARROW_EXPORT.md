# Exportación a Arrow/Pandas 📊

Integración zero-copy con PyArrow, Pandas y frameworks de ML.

---

## 🎯 Export Básico

### Grafo Completo

```python
import pyarrow as pa
import pandas as pd

# Exportar
nodes_bytes, edges_bytes = graph.to_arrow_complete()

# A Pandas
nodes_df = pa.ipc.open_stream(nodes_bytes).read_next_batch().to_pandas()
edges_df = pa.ipc.open_stream(edges_bytes).read_next_batch().to_pandas()

print(f"Nodos: {len(nodes_df)}")
print(f"Edges: {len(edges_df)}")
```

### Solo Nodos

```python
nodes_bytes = graph.to_arrow()
nodes_df = pa.ipc.open_stream(nodes_bytes).read_next_batch().to_pandas()
```

### Solo Edges

```python
edges_bytes = graph.edges_to_arrow()
edges_df = pa.ipc.open_stream(edges_bytes).read_next_batch().to_pandas()
```

### Filtrado por Label

```python
nodes_bytes, edges_bytes = graph.to_arrow_complete(label="Person")
```

---

## 🤖 Integración con PyTorch Geometric

### Crear Graph Dataset

```python
import torch
from torch_geometric.data import Data

# Export
nodes_df, edges_df = export_to_pandas(graph)

# Crear tensores
x = torch.tensor(nodes_df[['feat1', 'feat2']].values, dtype=torch.float)
edge_index = torch.tensor([edges_df['source'].values, edges_df['target'].values])

# PyG Data
data = Data(x=x, edge_index=edge_index)
```

---

## 📊 Análisis con Pandas

### Estadísticas Básicas

```python
nodes_df, edges_df = export_to_pandas(graph)

print(nodes_df['label'].value_counts())
print(edges_df['edge_type'].value_counts())
```

### Joins y Agregaciones

```python
# Join nodos con edges
df = edges_df.merge(
    nodes_df[['id', 'name']], 
    left_on='source', 
    right_on='id'
)

# Análisis
connections = df.groupby('name').size()
print(connections.sort_values(ascending=False).head(10))
```

---

## 🚀 Performance

- **Zero-copy**: Sin serialización
- **Columnar**: Eficiente para ML
- **Parallel**: Compatible con Dask/Ray

---

Ver: [API_REFERENCE.md](API_REFERENCE.md) | [EXAMPLES.md](EXAMPLES.md)
