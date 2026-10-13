#!/usr/bin/env python3
"""Guard: every method named in docs/python/API_CHEATSHEET.md exists in the
stub, and every name listed under "Names that do NOT exist" really does not
(#229). The cheat sheet is what assistants read first; a stale name there
is exactly the error this page exists to prevent.

    python nopaldb/python/scripts/check_cheatsheet.py
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SHEET = ROOT / "docs" / "python" / "API_CHEATSHEET.md"
STUB = ROOT / "nopaldb" / "python" / "nopaldb" / "nopaldb.pyi"


def main() -> int:
    stub = STUB.read_text(encoding="utf-8")
    defined = set(re.findall(r"^\s+def (\w+)\(", stub, re.M))
    text = SHEET.read_text(encoding="utf-8")
    head, _, tail = text.partition("## Names that do NOT exist")
    if not tail:
        print("check_cheatsheet: section 'Names that do NOT exist' missing")
        return 1
    listed = re.findall(r"^\| `(\w+)\(", head, re.M)
    missing = sorted(n for n in listed if n not in defined)
    forbidden = re.findall(r"^\| `(\w+)\(", tail, re.M)
    present = sorted(n for n in forbidden if n in defined)
    ok = True
    if not listed:
        print("check_cheatsheet: no methods found in the cheat sheet")
        ok = False
    if missing:
        print(f"check_cheatsheet: not in nopaldb.pyi: {missing}")
        ok = False
    if present:
        print(f"check_cheatsheet: listed as non-existent but defined: {present}")
        ok = False
    if ok:
        print(f"check_cheatsheet: {len(listed)} methods verified, {len(forbidden)} non-names confirmed")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
