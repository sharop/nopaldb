# Machine Learning Examples

NopalDB can participate in graph ML workflows by storing graph structure,
exporting data through Arrow, and storing embeddings when the corresponding
features are enabled.

This document is intentionally small: it shows supported integration patterns
without publishing unverified result tables.

## Available Patterns

| Pattern | NopalDB role |
|---------|--------------|
| Node features | Store node properties and export them through Arrow |
| Edge lists | Store relationships and export them through Arrow |
| Embeddings | Store vectors per node, edge, or path reference |
| Similarity search | Query vectors with HNSW-enabled embeddings APIs |

## Export Graph Data

```python
import nopaldb
import pyarrow.ipc as ipc

graph = nopaldb.Graph.open("data/my_graph.db")

nodes_bytes, edges_bytes = graph.to_arrow_complete()
nodes = ipc.open_stream(nodes_bytes).read_all().to_pandas()
edges = ipc.open_stream(edges_bytes).read_all().to_pandas()
```

## Build A Feature Matrix

```python
import numpy as np

feature_columns = ["age", "score"]
features = nodes[feature_columns].fillna(0).to_numpy(dtype=np.float32)
```

## Build An Edge Index

```python
node_index = {node_id: i for i, node_id in enumerate(nodes["id"])}

pairs = []
for _, edge in edges.iterrows():
    source = node_index.get(edge["source"])
    target = node_index.get(edge["target"])
    if source is not None and target is not None:
        pairs.append((source, target))

edge_index = np.array(pairs, dtype=np.int64).T
```

## Store And Query Embeddings

Build the Python wrapper with `python-full` before using embedding APIs:

```bash
cd nopaldb
maturin develop --release --features python-full
```

```python
graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "example-model")

vector = graph.get_node_embedding(node_id, "example-model")

matches = graph.knn_nodes(
    [0.1, 0.2, 0.3],
    5,
    "example-model",
)
```

## Example: Simple Similarity Workflow

```python
import nopaldb

graph = nopaldb.Graph.in_memory()

tx = graph.begin_transaction()
a = tx.add_node("Document", {"title": "Alpha"})
b = tx.add_node("Document", {"title": "Beta"})
tx.commit()

graph.add_node_embedding(a, [1.0, 0.0, 0.0], "demo")
graph.add_node_embedding(b, [0.9, 0.1, 0.0], "demo")

for node_id, score in graph.knn_nodes([1.0, 0.0, 0.0], 2, "demo"):
    print(node_id, score)
```

## Related Docs

- [Arrow ML Integration](arrow/05-ML-INTEGRATION.md)
- [Embeddings](EMBEDDINGS.md)
- [Feature Tiers](FEATURE_TIERS.md)
