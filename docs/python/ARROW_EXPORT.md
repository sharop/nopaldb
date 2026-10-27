# Exportación a Arrow/Pandas 📊

Integración con PyArrow, Pandas y frameworks de ML. Cada método devuelve
`bytes` con un stream Arrow IPC: Python recibe una copia en formato columnar y
PyArrow la lee sin parsear fila por fila.

---

## 🧭 Qué devuelve cada método

| método | nodos | aristas | grafo vacío |
|---|---|---|---|
| `to_arrow()` | `id`, `label`, `property_count` (sin propiedades) | — | `RuntimeError` |
| `to_arrow(label)` | `id`, `label` + una columna por propiedad de ese label | — | batch vacío (`id`, `label`) |
| `edges_to_arrow()` | — | `id`, `source`, `target`, `edge_type` + propiedades | batch vacío (`id`, `source`, `target`, `edge_type`) |
| `to_arrow_complete()` | `id`, `label` + propiedades de todos los nodos | igual que `edges_to_arrow()` | dos batches vacíos |
| `to_arrow_complete(label)` | los nodos de ese label | solo las aristas con **los dos extremos** de ese label (subgrafo inducido) | dos batches vacíos |

`to_arrow_complete(label)` lee nodos y aristas como un único estado
consistente: los escritores esperan mientras lee, así que una arista nunca
aparece sin sus extremos. Una propiedad que se llama como una columna
estructural (`id`, `label`, …) sale como `property.<nombre>`.

---

## 🎯 Export Básico

### Grafo Completo

```python
import pyarrow as pa
import pandas as pd

# Exportar
nodes_bytes, edges_bytes = graph.to_arrow_complete()

# A Pandas (read_all lee todos los batches del stream)
nodes_df = pa.ipc.open_stream(nodes_bytes).read_all().to_pandas()
edges_df = pa.ipc.open_stream(edges_bytes).read_all().to_pandas()

print(f"Nodos: {len(nodes_df)}")
print(f"Edges: {len(edges_df)}")
```

Un helper para el resto de la página:

```python
def export_to_pandas(graph, label=None):
    """(nodes_df, edges_df) del grafo, o del subgrafo inducido por `label`."""
    nodes_bytes, edges_bytes = graph.to_arrow_complete(label)
    return (
        pa.ipc.open_stream(nodes_bytes).read_all().to_pandas(),
        pa.ipc.open_stream(edges_bytes).read_all().to_pandas(),
    )
```

### Solo Nodos

```python
# Sin label: solo id, label y property_count. Con label: sus propiedades.
people_df = pa.ipc.open_stream(graph.to_arrow("Person")).read_all().to_pandas()
```

### Solo Edges

```python
edges_bytes = graph.edges_to_arrow()
edges_df = pa.ipc.open_stream(edges_bytes).read_all().to_pandas()
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

# Export (subgrafo de un label, para que todas las filas tengan las mismas columnas)
nodes_df, edges_df = export_to_pandas(graph, label="Person")

# Los ids son UUID en texto: PyG quiere índices enteros 0..N-1
index = {node_id: i for i, node_id in enumerate(nodes_df["id"])}
src = edges_df["source"].map(index).tolist()
dst = edges_df["target"].map(index).tolist()

# Tensores: características numéricas reales del nodo
x = torch.tensor(nodes_df[["age"]].fillna(0).values, dtype=torch.float)
edge_index = torch.tensor([src, dst], dtype=torch.long)

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

- **Una copia, sin parseo por fila**: el stream IPC llega como `bytes` y PyArrow lo mapea a columnas
- **Columnar**: Eficiente para ML
- **Parallel**: Compatible con Dask/Ray

---

Ver: [API_REFERENCE.md](API_REFERENCE.md) | [EXAMPLES.md](EXAMPLES.md) | [NETWORKX.md](NETWORKX.md)
