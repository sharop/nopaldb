"""Visualización ligera para los notebooks del tutorial.

Helpers que toman resultados NQL (filas como dicts) y los convierten en
DataFrames de pandas o grafos de networkx para plots con matplotlib.
"""

from __future__ import annotations

from typing import Iterable, List, Optional, Tuple

import pandas as pd


def result_to_df(query_result) -> pd.DataFrame:
    """Convierte un QueryResult a pandas.DataFrame preservando el orden de columnas."""
    rows = list(query_result)
    if not rows:
        return pd.DataFrame(columns=list(query_result.columns))
    return pd.DataFrame(rows, columns=list(query_result.columns))


def _unwrap_query(res):
    """Desempaqueta el resultado de execute_nql al objeto iterable de filas.

    Compatible con ambas APIs:
      - v0.4.16 y anteriores: execute_nql() devuelve QueryResult directo.
      - v0.4.20+: execute_nql() devuelve NqlResult con .query (None para writes).
    """
    if hasattr(res, "kind") and hasattr(res, "query"):
        if res.kind != "query":
            raise ValueError(
                f"Esperaba kind='query', recibí kind={res.kind!r}. "
                f"Para writes/index usa execute_nql() directo y mira res.summary."
            )
        return res.query
    return res


def execute_to_df(graph, nql: str) -> pd.DataFrame:
    """Ejecuta una query NQL y retorna DataFrame. Solo para statements FIND."""
    res = graph.execute_nql(nql)
    return result_to_df(_unwrap_query(res))


def edges_to_networkx(
    edge_rows: Iterable[dict],
    source_col: str = "source",
    target_col: str = "target",
    edge_attr_cols: Optional[List[str]] = None,
    directed: bool = False,
):
    """Construye un grafo de networkx desde filas con columnas (source, target, ...)."""
    import networkx as nx

    g = nx.DiGraph() if directed else nx.Graph()
    extra = edge_attr_cols or []
    for row in edge_rows:
        attrs = {k: row[k] for k in extra if k in row}
        g.add_edge(row[source_col], row[target_col], **attrs)
    return g


def plot_network(
    graph_nx,
    node_color_map: Optional[dict] = None,
    title: str = "",
    figsize: Tuple[int, int] = (10, 8),
    seed: int = 42,
    with_labels: bool = True,
    node_size: int = 800,
):
    """Plot básico de un grafo networkx con layout spring determinista."""
    import matplotlib.pyplot as plt
    import networkx as nx

    fig, ax = plt.subplots(figsize=figsize)
    pos = nx.spring_layout(graph_nx, seed=seed)
    if node_color_map is not None:
        colors = [node_color_map.get(n, "#cccccc") for n in graph_nx.nodes()]
    else:
        colors = "#88aadd"
    nx.draw_networkx_edges(graph_nx, pos, alpha=0.4, ax=ax)
    nx.draw_networkx_nodes(
        graph_nx, pos, node_color=colors, node_size=node_size, ax=ax
    )
    if with_labels:
        nx.draw_networkx_labels(graph_nx, pos, font_size=8, ax=ax)
    ax.set_title(title)
    ax.set_axis_off()
    plt.tight_layout()
    return fig, ax
