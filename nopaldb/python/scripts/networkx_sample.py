"""Guard: NetworkX conversion (nopaldb.networkx), both ways.

A graph with a shared component, parallel edges, a cycle, a self-loop and an
isolated node goes NopalDB → NetworkX → NopalDB → NetworkX with the same
structure, labels and properties in their native types; importing twice
writes nothing new; a label exports its induced subgraph.
"""
import networkx as nx

import nopaldb
from nopaldb.networkx import from_networkx, to_networkx

g = nopaldb.Graph.in_memory()
tx = g.begin_transaction()
nopal = tx.add_node("Planta", {"nombre": "nopal", "altura": 1.5, "usos": ["tuna", "verdura"], "label": "mi-etiqueta"})
maguey = tx.add_node("Planta", {"nombre": "maguey", "edad": 7, "meta": {"region": "centro"}})
tuna = tx.add_node("Fruto", {"nombre": "tuna", "comestible": True, "foto": b"\x01\x02"})
solo = tx.add_node("Planta", {"nombre": "biznaga"})                     # isolated
tx.add_edge(nopal, tuna, "DA", {"mes": 8})
tx.add_edge(nopal, tuna, "DA", {"mes": 9})                              # parallel
tx.add_edge(nopal, maguey, "JUNTO_A", {})
tx.add_edge(maguey, nopal, "JUNTO_A", {})                               # cycle
tx.add_edge(maguey, maguey, "MISMA", {"type": "custom"})                # self-loop + reserved name
tx.commit()

G = to_networkx(g)
assert isinstance(G, nx.MultiDiGraph)
assert G.number_of_nodes() == 4 and G.number_of_edges() == 5, (G.number_of_nodes(), G.number_of_edges())
assert solo in G and G.degree(solo) == 0, "isolated node kept"
assert G.number_of_edges(nopal, tuna) == 2 and G.has_edge(maguey, maguey)
n = G.nodes[nopal]
assert n["label"] == "Planta" and n["property.label"] == "mi-etiqueta", n        # no shadowing
assert n["altura"] == 1.5 and n["usos"] == ["tuna", "verdura"]                  # native types
assert G.nodes[maguey]["meta"] == {"region": "centro"} and G.nodes[maguey]["edad"] == 7
assert G.nodes[tuna]["foto"] == b"\x01\x02" and G.nodes[tuna]["comestible"] is True
loop = next(iter(G.get_edge_data(maguey, maguey).values()))
assert loop["type"] == "MISMA" and loop["property.type"] == "custom", loop
assert sorted(d["mes"] for d in G.get_edge_data(nopal, tuna).values()) == [8, 9]

# Into another graph, twice: the second import writes nothing.
h = nopaldb.Graph.in_memory()
ids = from_networkx(h, G)
assert set(ids) == set(G.nodes)
counts = (h.node_count(), h.edge_count())
assert counts == (4, 5), counts
from_networkx(h, G)
assert (h.node_count(), h.edge_count()) == counts, "idempotent"

# Back to NetworkX, keyed by the original keys: same structure and data.
H = to_networkx(h, key="nx_key")
assert set(H.nodes) == set(G.nodes)
for k in G.nodes:
    a = {x: y for x, y in G.nodes[k].items() if x != "id"}
    b = {x: y for x, y in H.nodes[k].items() if x not in ("id", "nx_key")}
    assert a == b, (k, a, b)
def edge_bag(graph):
    return sorted((u, v, d["type"], tuple(sorted((x, repr(y)) for x, y in d.items() if x not in ("id", "type", "nx_key"))))
                  for u, v, d in graph.edges(data=True))
assert edge_bag(G) == edge_bag(H), (edge_bag(G), edge_bag(H))

# A label exports its induced subgraph only.
P = to_networkx(g, label="Planta")
assert set(P.nodes) == {nopal, maguey, solo} and not P.has_edge(nopal, tuna)
assert P.number_of_edges() == 3, P.number_of_edges()

# An undirected graph from NetworkX, and keys NopalDB cannot store.
U = nx.Graph()
U.add_edge("a", "b", peso=2)
U.add_node("c", label="Aislado")
k = nopaldb.Graph.in_memory()
from_networkx(k, U, label="Punto", edge_type="CONECTA")
assert (k.node_count(), k.edge_count()) == (3, 1)
labels = sorted(r["n.label"] for r in k.execute_nql("find n.label from (n)"))
assert labels == ["Aislado", "Punto", "Punto"], labels
try:
    from_networkx(nopaldb.Graph.in_memory(), nx.path_graph([(1, 2), (3, 4)]))
    raise SystemExit("tuple keys must be rejected")
except ValueError as exc:
    assert "only str, int and float" in str(exc), exc

print("networkx_sample: OK")
