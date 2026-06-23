"""Path resolution + NQL loader.

Núcleo del anti-drift entre los 4 medios del tutorial. Cada query NQL vive una
sola vez en `docs/tutorial/acto_N/queries/*.nql` y todos los medios la leen
desde ahí. Si la query cambia, cambia en un solo lugar.
"""

from __future__ import annotations

from pathlib import Path
from typing import List


REPO_ROOT: Path = Path(__file__).resolve().parents[2]
DOCS_TUTORIAL_DIR: Path = REPO_ROOT / "docs" / "tutorial"
TEST_DBS_DIR: Path = REPO_ROOT / "test_dbs"
FIXTURES_DIR: Path = REPO_ROOT / "nopaldb" / "tests" / "fixtures"
PRECOMPUTED_DIR: Path = REPO_ROOT / "tutorials" / "precomputed"
DATA_DIR: Path = REPO_ROOT / "tutorials" / "data"


_ACTO_DIRS = {
    "acto_0": "acto_0_setup",
    "acto_1": "acto_1_florentine",
    "acto_2": "acto_2_synthetic_offshore",
    "acto_3": "acto_3_biomedical_owl",
    "acto_4": "acto_4_synthetic_fraud",
}


def _resolve_acto(acto: str) -> Path:
    if acto in _ACTO_DIRS:
        return DOCS_TUTORIAL_DIR / _ACTO_DIRS[acto]
    candidate = DOCS_TUTORIAL_DIR / acto
    if candidate.is_dir():
        return candidate
    raise ValueError(
        f"Acto desconocido: {acto!r}. Opciones: {sorted(_ACTO_DIRS)}"
    )


def load_nql(acto: str, name: str) -> str:
    """Lee un archivo .nql canónico desde docs/tutorial/<acto>/queries/.

    Args:
        acto: identificador corto ("acto_1") o nombre del directorio
            ("acto_1_florentine").
        name: nombre del archivo, con o sin extensión .nql.

    Returns:
        Contenido del archivo como string (UTF-8), sin trailing whitespace.
    """
    queries_dir = _resolve_acto(acto) / "queries"
    if not name.endswith(".nql"):
        name = f"{name}.nql"
    path = queries_dir / name
    if not path.is_file():
        available = ", ".join(sorted(p.name for p in queries_dir.glob("*.nql")))
        raise FileNotFoundError(
            f"No existe {path}. Disponibles en {queries_dir}: {available}"
        )
    return path.read_text(encoding="utf-8").rstrip()


def list_queries(acto: str) -> List[str]:
    """Lista nombres de archivos .nql disponibles en un acto, ordenados."""
    queries_dir = _resolve_acto(acto) / "queries"
    return sorted(p.name for p in queries_dir.glob("*.nql"))


def db_path(name: str) -> str:
    """Path absoluto de una DB en test_dbs/, como str.

    Acepta `florentine_families.db` o `florentine_families` (sufijo opcional).
    """
    if not name.endswith(".db"):
        name = f"{name}.db"
    TEST_DBS_DIR.mkdir(parents=True, exist_ok=True)
    return str(TEST_DBS_DIR / name)
