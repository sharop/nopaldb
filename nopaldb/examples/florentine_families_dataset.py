#!/usr/bin/env python3
"""
Florentine Families dataset generator for NopalDB.

Classic Renaissance network (marriage ties) used in social network analysis.
"""

import argparse
from pathlib import Path

import nopaldb


FAMILIES = [
    ("Acciaiuoli", {"wealth_rank": 12, "faction": "Medici"}),
    ("Albizzi", {"wealth_rank": 3, "faction": "Albizzi"}),
    ("Barbadori", {"wealth_rank": 9, "faction": "Medici"}),
    ("Bischeri", {"wealth_rank": 8, "faction": "Albizzi"}),
    ("Castellani", {"wealth_rank": 10, "faction": "Albizzi"}),
    ("Ginori", {"wealth_rank": 14, "faction": "Albizzi"}),
    ("Guadagni", {"wealth_rank": 6, "faction": "Albizzi"}),
    ("Lamberteschi", {"wealth_rank": 15, "faction": "Albizzi"}),
    ("Medici", {"wealth_rank": 1, "faction": "Medici"}),
    ("Pazzi", {"wealth_rank": 11, "faction": "Albizzi"}),
    ("Peruzzi", {"wealth_rank": 7, "faction": "Albizzi"}),
    ("Ridolfi", {"wealth_rank": 5, "faction": "Medici"}),
    ("Salviati", {"wealth_rank": 13, "faction": "Medici"}),
    ("Strozzi", {"wealth_rank": 2, "faction": "Albizzi"}),
    ("Tornabuoni", {"wealth_rank": 4, "faction": "Medici"}),
]

# Classic marriage ties (undirected in literature).
# We store both directions to make neighborhood exploration intuitive in directed graphs.
MARRIAGE_EDGES = [
    ("Acciaiuoli", "Medici"),
    ("Castellani", "Peruzzi"),
    ("Castellani", "Strozzi"),
    ("Castellani", "Barbadori"),
    ("Medici", "Barbadori"),
    ("Medici", "Ridolfi"),
    ("Medici", "Tornabuoni"),
    ("Medici", "Albizzi"),
    ("Medici", "Salviati"),
    ("Salviati", "Pazzi"),
    ("Peruzzi", "Strozzi"),
    ("Peruzzi", "Bischeri"),
    ("Strozzi", "Ridolfi"),
    ("Strozzi", "Bischeri"),
    ("Ridolfi", "Tornabuoni"),
    ("Tornabuoni", "Guadagni"),
    ("Albizzi", "Ginori"),
    ("Albizzi", "Guadagni"),
    ("Bischeri", "Guadagni"),
    ("Guadagni", "Lamberteschi"),
]


def generate_dataset(db_path: str, reset: bool) -> None:
    path = Path(db_path)
    if reset and path.exists():
        # NopalDB stores directories for db paths in many setups.
        if path.is_dir():
            for child in path.iterdir():
                if child.is_file():
                    child.unlink()
            path.rmdir()
        else:
            path.unlink()

    path.parent.mkdir(parents=True, exist_ok=True)

    print("=" * 60)
    print("Generating Florentine Families dataset")
    print("=" * 60)
    print(f"DB: {db_path}")

    with nopaldb.Graph.open(db_path) as graph:
        try:
            graph.execute_nql("create index on Family(name) type hash")
            graph.execute_nql("create index on Family(faction) type hash")
        except Exception:
            # Safe if index already exists or backend differs.
            pass

        family_ids = {}
        tx = graph.begin_transaction()
        for name, attrs in FAMILIES:
            props = {"name": name, "network": "florentine_families"}
            props.update(attrs)
            family_ids[name] = tx.add_node("Family", props)
        tx.commit()
        print(f"Created families: {len(FAMILIES)}")

        tx = graph.begin_transaction()
        edge_count = 0
        for src, dst in MARRIAGE_EDGES:
            tx.add_edge(
                family_ids[src],
                family_ids[dst],
                "MARRIAGE",
                {"tie": "marriage", "undirected_pair": True},
            )
            tx.add_edge(
                family_ids[dst],
                family_ids[src],
                "MARRIAGE",
                {"tie": "marriage", "undirected_pair": True},
            )
            edge_count += 2
        tx.commit()
        print(f"Created edges: {edge_count}")

        stats = graph.get_stats()
        print("Stats:")
        print(f"  total_nodes: {int(stats['total_nodes'])}")
        print(f"  total_edges: {int(stats['total_edges'])}")

    print("=" * 60)
    print("Done.")
    print("=" * 60)
    print("Try in NDBStudio:")
    print(f"  cargo run -p ndbstudio -- {db_path}")
    print('Then run: find f.name, pagerank(f) as pr from (f:Family) order by pr desc')


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate Florentine Families graph in NopalDB")
    parser.add_argument(
        "--db",
        default="test_dbs/florentine_families.db",
        help="Output database path (default: test_dbs/florentine_families.db)",
    )
    parser.add_argument(
        "--reset",
        action="store_true",
        help="Delete existing DB path before generating dataset",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    generate_dataset(args.db, args.reset)


if __name__ == "__main__":
    main()
