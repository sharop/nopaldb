# Arrow ML Integration

This guide shows the minimal path from NopalDB to common Python ML tooling:
export graph data as Arrow IPC bytes, load it with PyArrow, and convert it to
Pandas, NumPy, or tensors.

## Setup

```bash
pip install pyarrow pandas numpy

# Optional, only if your project uses tensors:
pip install torch
```

Build or install the Python wrapper with Arrow support:

```bash
make build-wheel
pip install dist/wheels/nopaldb-*.whl
```

For local development:

```bash
cd nopaldb
maturin develop --release --features python-full
```

## Export Nodes And Edges

```python
import nopaldb

graph = nopaldb.Graph.open("data/my_graph.db")

nodes_bytes = graph.to_arrow()
edges_bytes = graph.edges_to_arrow()
```

Filter nodes by label when you only need one entity type:

```python
people_bytes = graph.to_arrow(label="Person")
```

Export both streams at once:

```python
nodes_bytes, edges_bytes = graph.to_arrow_complete()
```

## Load With PyArrow

```python
import pyarrow.ipc as ipc

nodes_reader = ipc.open_stream(nodes_bytes)
nodes_table = nodes_reader.read_all()

edges_reader = ipc.open_stream(edges_bytes)
edges_table = edges_reader.read_all()
```

## Convert To Pandas

```python
nodes_df = nodes_table.to_pandas()
edges_df = edges_table.to_pandas()

print(nodes_df.head())
print(edges_df.head())
```

## Convert To NumPy Or Torch

```python
import numpy as np

numeric_cols = ["age", "score"]
features = nodes_df[numeric_cols].fillna(0).to_numpy(dtype=np.float32)
```

Optional tensor conversion:

```python
import torch

x = torch.from_numpy(features)
```

## Minimal Graph ML Flow

```python
import nopaldb
import pyarrow.ipc as ipc
import numpy as np

graph = nopaldb.Graph.open("data/my_graph.db")

nodes_bytes, edges_bytes = graph.to_arrow_complete()
nodes = ipc.open_stream(nodes_bytes).read_all().to_pandas()
edges = ipc.open_stream(edges_bytes).read_all().to_pandas()

node_index = {node_id: i for i, node_id in enumerate(nodes["id"])}

edge_pairs = []
for _, edge in edges.iterrows():
    source = node_index.get(edge["source"])
    target = node_index.get(edge["target"])
    if source is not None and target is not None:
        edge_pairs.append((source, target))

edge_index = np.array(edge_pairs, dtype=np.int64).T
```

At this point your project can pass `nodes`, feature arrays, and `edge_index`
to the ML framework of your choice.

## Notes

- Arrow export returns IPC stream bytes, not filenames.
- Keep labels and property names stable if downstream training code depends on
  specific columns.
- For large graphs, export the smallest label/property set required by the ML
  job.
