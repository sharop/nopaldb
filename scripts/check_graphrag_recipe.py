#!/usr/bin/env python3
"""Verifica que la receta de docs/GRAPHRAG.md sea el código de graphrag_sample.py.

Existe porque una receta copiada a mano envejece en silencio: el sample corre
en CI y se corrige cuando la API cambia; la doc no corre y nadie la relee
(#230). Así que la doc muestra los bloques del sample y este check falla
cuando divergen.

Convención:
- En el sample, cada bloque va entre `# recipe:<nombre>:start` y
  `# recipe:<nombre>:end` (líneas de comentario sueltas).
- En la doc, entre `<!-- recipe:<nombre>:start -->` y
  `<!-- recipe:<nombre>:end -->`, y dentro hay un único fence ```python.
Se comparan tras `textwrap.dedent` y quitando espacios finales; los nombres
deben ser los mismos y en el mismo orden.

Uso: python3 scripts/check_graphrag_recipe.py
Sale con 1 y un diff por bloque si algo difiere.
"""

from __future__ import annotations

import difflib
import re
import sys
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs" / "GRAPHRAG.md"
SAMPLE = ROOT / "nopaldb" / "python" / "scripts" / "graphrag_sample.py"

SAMPLE_MARK = re.compile(r"^\s*# recipe:([a-z_]+):(start|end)\s*$")
DOC_MARK = re.compile(r"^\s*<!-- recipe:([a-z_]+):(start|end) -->\s*$")


def blocks(path: Path, mark: re.Pattern[str], fenced: bool) -> list[tuple[str, str]]:
    """[(nombre, código)] en orden de aparición."""
    out: list[tuple[str, str]] = []
    name: str | None = None
    body: list[str] = []
    in_fence = False
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        m = mark.match(line)
        if m:
            kind = m.group(2)
            if kind == "start":
                if name is not None:
                    sys.exit(f"{path}:{lineno}: '{m.group(1)}' empieza dentro de '{name}'")
                name, body, in_fence = m.group(1), [], False
            else:
                if name != m.group(1):
                    sys.exit(f"{path}:{lineno}: cierra '{m.group(1)}' pero estaba abierto '{name}'")
                out.append((name, normalize(body)))
                name = None
            continue
        if name is None:
            continue
        if fenced:
            if line.strip().startswith("```"):
                if not in_fence and line.strip() != "```python":
                    sys.exit(f"{path}:{lineno}: el fence de '{name}' debe ser ```python")
                in_fence = not in_fence
                continue
            if not in_fence:
                if line.strip():
                    sys.exit(f"{path}:{lineno}: texto fuera del fence dentro de '{name}'")
                continue
        body.append(line)
    if name is not None:
        sys.exit(f"{path}: '{name}' no se cerró")
    return out


def normalize(lines: list[str]) -> str:
    return textwrap.dedent("\n".join(line.rstrip() for line in lines)).strip("\n") + "\n"


def main() -> int:
    doc = blocks(DOC, DOC_MARK, fenced=True)
    sample = blocks(SAMPLE, SAMPLE_MARK, fenced=False)
    if not sample:
        print(f"{SAMPLE}: no hay bloques 'recipe:*'", file=sys.stderr)
        return 1
    if [n for n, _ in doc] != [n for n, _ in sample]:
        print(f"bloques en la doc {[n for n, _ in doc]} != en el sample {[n for n, _ in sample]}", file=sys.stderr)
        return 1
    bad = 0
    for (name, d), (_, s) in zip(doc, sample):
        if d != s:
            bad += 1
            print(f"== recipe:{name} difiere (doc → sample)", file=sys.stderr)
            sys.stderr.writelines(difflib.unified_diff(
                d.splitlines(keepends=True), s.splitlines(keepends=True),
                fromfile=str(DOC.relative_to(ROOT)), tofile=str(SAMPLE.relative_to(ROOT))))
    if bad:
        return 1
    print(f"graphrag recipe: {len(sample)} bloques iguales en doc y sample")
    return 0


if __name__ == "__main__":
    sys.exit(main())
