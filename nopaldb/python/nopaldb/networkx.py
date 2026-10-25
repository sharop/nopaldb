"""Conversion between a NopalDB graph and a NetworkX graph.

Requires NetworkX: ``pip install "nopaldb[networkx]"``. It is imported only
when one of these functions runs.

    import nopaldb
    from nopaldb.networkx import to_networkx, from_networkx

    G = to_networkx(graph)                 # nx.MultiDiGraph
    from_networkx(other_graph, G)          # idempotent: run it twice, same graph

**Shape of the NetworkX graph** (``to_networkx``):

- a ``networkx.MultiDiGraph``: directed, parallel edges and self-loops kept;
- node key: the NopalDB id, or with ``key="nx_key"`` the value of that
  property (the key a graph imported with ``from_networkx`` had);
- node attributes: ``id`` and ``label``, then the node's properties in their
  native types (int, float, str, bool, None, list, dict, bytes);
- edge key: the NopalDB edge id; edge attributes: ``id`` and ``type``, then
  the edge's properties;
- a property named like a structural attribute (``id``/``label`` on nodes,
  ``id``/``type`` on edges) is exported as ``property.<name>``, as in the
  Arrow export, so it never shadows the structural one;
- isolated nodes are included.

The graph is read as one consistent state (``Graph.export_graph``): writers
wait while it reads.

**Import** (``from_networkx``): every node is upserted by the property
``key_property`` (``"nx_key"``), holding its NetworkX key, so importing the
same graph again writes nothing new. Node keys must be ``str``, ``int`` or
``float``. Edges are written in one transaction; an edge already present
(same source, target, type and, for multigraphs, the same NetworkX edge key)
is skipped. An undirected graph is imported with each edge once, in the
direction NetworkX lists it.
"""

from __future__ import annotations

from typing import Any, Optional

NODE_RESERVED = ("id", "label")
EDGE_RESERVED = ("id", "type")
PREFIX = "property."


def _require_networkx() -> Any:
    try:
        import networkx  # noqa: PLC0415 - optional dependency, imported on use
    except ImportError as exc:  # pragma: no cover - depends on the environment
        raise ImportError(
            'nopaldb.networkx needs NetworkX: pip install "nopaldb[networkx]"'
        ) from exc
    return networkx


def _attrs(structural: dict[str, Any], properties: dict[str, Any], reserved: tuple[str, ...]) -> dict[str, Any]:
    out = dict(structural)
    for name, value in properties.items():
        out[PREFIX + name if name in reserved else name] = value
    return out


def to_networkx(graph: Any, label: Optional[str] = None, key: Optional[str] = None) -> Any:
    """The graph (or the subgraph induced by ``label``) as a ``networkx.MultiDiGraph``.

    ``key``: the node property to use as the NetworkX node key (e.g.
    ``"nx_key"`` after ``from_networkx``); a node without it keeps its id.
    """
    nx = _require_networkx()
    snapshot = graph.export_graph(label)
    G = nx.MultiDiGraph()
    key_of: dict[str, Any] = {}
    for node in snapshot["nodes"]:
        props = node["properties"]
        node_key = props[key] if key is not None and key in props else node["id"]
        if node_key in G:
            raise ValueError(f"to_networkx: two nodes have the key {node_key!r} (property {key!r})")
        key_of[node["id"]] = node_key
        G.add_node(node_key, **_attrs({"id": node["id"], "label": node["label"]}, props, NODE_RESERVED))
    for edge in snapshot["edges"]:
        G.add_edge(
            key_of[edge["source"]],
            key_of[edge["target"]],
            key=edge["id"],
            **_attrs({"id": edge["id"], "type": edge["type"]}, edge["properties"], EDGE_RESERVED),
        )
    return G


def _properties(attrs: dict[str, Any], structural: tuple[str, ...]) -> dict[str, Any]:
    """Attributes back to properties: drop the structural ones, undo ``property.``."""
    out: dict[str, Any] = {}
    for name, value in attrs.items():
        if name in structural:
            continue
        out[name[len(PREFIX):] if name.startswith(PREFIX) else name] = value
    return out


def from_networkx(
    graph: Any,
    G: Any,
    label: str = "Node",
    edge_type: str = "RELATED",
    key_property: str = "nx_key",
) -> dict[Any, str]:
    """Load a NetworkX graph into ``graph``; returns ``{networkx key: NopalDB id}``.

    A node's label is its ``label`` attribute, else ``label``; an edge's type
    is its ``type`` attribute, else ``edge_type``. Other attributes become
    properties (``property.<name>`` goes back to ``<name>``). See the module
    docstring for idempotence and the accepted keys.
    """
    _require_networkx()
    requests = []
    keys = []
    for node_key, attrs in G.nodes(data=True):
        if isinstance(node_key, bool) or not isinstance(node_key, (str, int, float)):
            raise ValueError(
                f"from_networkx: node key {node_key!r} is a {type(node_key).__name__}; "
                "only str, int and float keys can be stored (relabel the graph first)"
            )
        props = _properties(attrs, ("id", "label"))
        if key_property in props and props[key_property] != node_key:
            raise ValueError(f"from_networkx: node {node_key!r} already has a different {key_property!r} attribute")
        props[key_property] = node_key
        requests.append({"label": attrs.get("label", label), "key": key_property, "props": props})
        keys.append(node_key)
    outcomes = graph.upsert_many(requests) if requests else []
    ids = {k: node_id for k, (_, node_id) in zip(keys, outcomes)}

    multigraph = G.is_multigraph()
    imported = set(ids.values())
    existing = set()
    for edge in graph.export_graph()["edges"]:
        if edge["source"] in imported and edge["target"] in imported:
            existing.add((edge["source"], edge["target"], edge["type"], edge["properties"].get(key_property)))

    edges = G.edges(keys=True, data=True) if multigraph else ((u, v, None, d) for u, v, d in G.edges(data=True))
    tx = graph.begin_transaction()
    try:
        for u, v, k, attrs in edges:
            props = _properties(attrs, ("id", "type"))
            if multigraph:
                props[key_property] = k if isinstance(k, (str, int, float)) and not isinstance(k, bool) else str(k)
            identity = (ids[u], ids[v], attrs.get("type", edge_type), props.get(key_property))
            if identity in existing:
                continue
            existing.add(identity)
            tx.add_edge(ids[u], ids[v], identity[2], props)
        tx.commit()
    except BaseException:
        tx.rollback()
        raise
    return ids
