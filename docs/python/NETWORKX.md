# NetworkX

`nopaldb.networkx` converts a NopalDB graph to a NetworkX graph and back. The
conversion is meant to be faithful: identity, parallel edges, self-loops,
isolated nodes, labels, and properties in their native types all survive. It
is a conversion, not a NetworkX backend. Run NetworkX algorithms on the
converted graph, or use the native ones (`pagerank`, `leiden`, …) for large
graphs.

```bash
pip install "nopaldb[networkx]"
```

```python
import nopaldb
from nopaldb.networkx import to_networkx, from_networkx

G = to_networkx(graph)                  # the whole graph
P = to_networkx(graph, label="Planta")  # the subgraph induced by a label
ids = from_networkx(other, G)           # {networkx key: nopaldb id}
```

## NopalDB → NetworkX

`to_networkx(graph, label=None, key=None)` returns a `networkx.MultiDiGraph`.

| NetworkX | From |
|---|---|
| node key | the NopalDB id, or the value of the property `key` (e.g. `key="nx_key"` after an import) |
| node attributes | `id`, `label`, then the node's properties |
| edge key | the NopalDB edge id |
| edge attributes | `id`, `type`, then the edge's properties |

- Properties keep their Python types: `int`, `float`, `str`, `bool`, `None`, `list`, `dict` and `bytes`. Nothing is turned into text.
- A property named like a structural attribute (`id`/`label` on nodes, `id`/`type` on edges) is exported as `property.<name>`, as in the [Arrow export](../arrow/03-TECHNICAL.md).
- With `label`, only the edges whose two endpoints have that label are exported.
- The graph is read as **one consistent state** (`Graph.export_graph`). Writers wait while it reads, so an edge never appears without its endpoints.

## NetworkX → NopalDB

`from_networkx(graph, G, label="Node", edge_type="RELATED", key_property="nx_key")`:

- **Nodes.** Each node is upserted by `key_property`, which holds its NetworkX key, so importing the same graph twice writes nothing new.
  - Keys must be `str`, `int` or `float`. Anything else, such as a tuple, is a `ValueError`; relabel the graph first.
  - A node's label is its `label` attribute, else `label`.
- **Edges.** An edge's type is its `type` attribute, else `edge_type`.
  - Other attributes become properties, and `property.<name>` goes back to `<name>`.
  - The edges are written in one transaction. An edge already present is skipped: same source, target, type and, for a multigraph, the same NetworkX edge key, stored in `key_property`.
- **Undirected graphs** are imported with each edge once, in the direction NetworkX lists it.

## Raw data

`Graph.export_graph(label=None)` returns the same consistent read as plain Python: `{"nodes": [...], "edges": [...]}`, with the dict shapes of `get_node` and `get_edge`. Use it to feed other libraries without NetworkX.
