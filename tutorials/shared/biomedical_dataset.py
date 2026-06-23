"""Genera tutorials/test_dbs/biomedical_owl.db importando tutorials/data/biomedical.ttl.

Requiere wheel compilado con --features python-owl (incluido en el tier 'semantic').
La DB es requerida por el Acto 6 (06_mcp_ontologia.ipynb) para las tools MCP:
  classify_node, list_instances, list_subclasses.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

TUTORIALS_DIR = Path(__file__).parent.parent
DEFAULT_DB = TUTORIALS_DIR / "test_dbs" / "biomedical_owl.db"
DEFAULT_TTL = TUTORIALS_DIR / "data" / "biomedical.ttl"


def generate(db_path=None, reset: bool = False) -> dict:
    """Genera biomedical_owl.db cargando el TTL via import_turtle.

    Args:
        db_path: ruta destino (default: test_dbs/biomedical_owl.db).
        reset:   si True, elimina y regenera aunque ya exista.

    Returns:
        dict con {classes_added, subclass_edges_added, instances_added, triples_skipped},
        o {} si la DB ya existía y se saltó la regeneración.
    """
    import nopaldb  # noqa: PLC0415 — import local para evitar error en módulos sin wheel

    db_path = Path(db_path) if db_path else DEFAULT_DB
    ttl_path = DEFAULT_TTL

    if not ttl_path.exists():
        raise FileNotFoundError(
            f"No se encontró el TTL fuente en {ttl_path}.\n"
            "Asegúrate de ejecutar desde el directorio tutorials/ o desde notebooks/."
        )

    # Idempotencia: skip si la DB ya existe y tiene datos
    if db_path.exists() and not reset:
        g = nopaldb.Graph.open(str(db_path))
        try:
            rows = g.execute_nql("find count(*) as n from (x)")
            count = rows[0].get("n", 0) if rows else 0
            if count > 0:
                print(f"biomedical_owl.db ya existe ({count} nodos). Usa reset=True para regenerar.")
                return {}
        except Exception:
            pass
        finally:
            g.close()
            del g

    if db_path.exists():
        shutil.rmtree(str(db_path))

    db_path.parent.mkdir(parents=True, exist_ok=True)
    g = nopaldb.Graph.open(str(db_path))
    try:
        report = g.import_turtle(ttl_path.read_text())
        g.close()
        del g
    except AttributeError:
        g.close()
        del g
        # Wheel sin python-owl — fallback al ejemplo Rust (ya disponible si compilaste nopaldb-mcp)
        return _generate_via_rust(db_path, ttl_path)

    print(f"biomedical_owl.db generada en {db_path}")
    print(
        f"  classes={report['classes_added']}, "
        f"instances={report['instances_added']}, "
        f"subclass_edges={report['subclass_edges_added']}"
    )
    return report


def _generate_via_rust(db_path: Path, ttl_path: Path) -> dict:
    """Fallback: genera la DB ejecutando el ejemplo Rust tutorial_acto_3_biomedical."""
    import subprocess

    repo_root = TUTORIALS_DIR.parent
    print("import_turtle no disponible en este wheel — usando fallback Rust...")
    print(f"  (para evitar esto: maturin develop --features python-owl,analytics,algorithms,reasoner)\n")

    result = subprocess.run(
        [
            "cargo", "run", "--example", "tutorial_acto_3_biomedical",
            "--no-default-features",
            "--features", "storage-sled,reasoner,owl-import,algorithms,analytics,hypergraph,ml",
            "--",
            str(db_path),
            str(ttl_path),
        ],
        cwd=str(repo_root),
        check=True,
    )
    print(f"\nbiomedical_owl.db generada en {db_path} (via Rust example)")
    return {}


if __name__ == "__main__":
    reset_flag = "--reset" in sys.argv
    db_arg = next((a for a in sys.argv[1:] if not a.startswith("--")), None)
    generate(db_arg, reset=reset_flag)
