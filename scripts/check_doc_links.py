#!/usr/bin/env python3
"""Verifica que los links relativos de la documentación apunten a archivos que existen.

Existe porque tres READMEs de docs enlazaron durante meses a dos archivos de
roadmap que nunca estuvieron en el repo: es lo primero que ve quien evalúa
contribuir. Un check que corre en CI no se olvida; una nota en CONTRIBUTING sí.

Solo mira links relativos (`[texto](ruta.md)`, `[texto](../dir/)`): los `http(s)`,
`mailto:` y los anclas puras (`#seccion`) se ignoran. Los fragmentos (`ruta.md#x`)
se recortan antes de resolver la ruta.

Uso: python3 scripts/check_doc_links.py [ruta ...]
Sin argumentos revisa docs/ y los .md de la raíz. Sale con 1 si hay algún roto.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_TARGETS = ["docs", "README.md", "ROADMAP.md", "CONTRIBUTING.md", "SECURITY.md"]

# [texto](destino) y [texto](destino "título"). Las imágenes ![..](..) caen también.
LINK_RE = re.compile(r"\[[^\]]*\]\(\s*([^)\s]+)(?:\s+\"[^\"]*\")?\s*\)")
SKIP_PREFIXES = ("http://", "https://", "mailto:", "#", "tel:")


def markdown_files(targets: list[str]) -> list[Path]:
    files: list[Path] = []
    for target in targets:
        path = ROOT / target
        if path.is_dir():
            files.extend(sorted(path.rglob("*.md")))
        elif path.is_file():
            files.append(path)
    # Los planes privados no se publican, así que tampoco se verifican.
    return [f for f in files if not f.name.endswith(".local.md")]


def broken_links(md_file: Path) -> list[tuple[int, str]]:
    broken: list[tuple[int, str]] = []
    in_fence = False
    for lineno, line in enumerate(md_file.read_text(encoding="utf-8").splitlines(), 1):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for raw in LINK_RE.findall(line):
            if raw.startswith(SKIP_PREFIXES):
                continue
            dest = raw.split("#", 1)[0]
            if not dest:
                continue
            resolved = (md_file.parent / dest).resolve()
            if not resolved.exists():
                broken.append((lineno, raw))
    return broken


def main(argv: list[str]) -> int:
    targets = argv[1:] or DEFAULT_TARGETS
    total = 0
    for md_file in markdown_files(targets):
        for lineno, raw in broken_links(md_file):
            print(f"{md_file.relative_to(ROOT)}:{lineno}: link roto → {raw}")
            total += 1
    if total:
        print(f"\n{total} link(s) roto(s).")
        return 1
    print("docs: cero links relativos rotos")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
