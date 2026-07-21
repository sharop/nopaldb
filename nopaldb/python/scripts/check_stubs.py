#!/usr/bin/env python3
"""Guard: fail if the native module exposes a public symbol the .pyi omits.

Run after `maturin develop`:
    python nopaldb/python/scripts/check_stubs.py
"""
from __future__ import annotations

import importlib
import re
import sys
from pathlib import Path

STUB = Path(__file__).resolve().parents[1] / "nopaldb" / "nopaldb.pyi"

# Data-model dunders we intentionally type; other dunders are noise from object.
KEPT_DUNDERS = {"__len__", "__iter__", "__getitem__", "__enter__", "__exit__", "__repr__", "__init__"}

CLASSES = [
    "Graph",
    "Transaction",
    "QueryResult",
    "ProfileResult",
    "NqlResult",
    "BulkLoader",
    "ELReasoner",
    "Inference",
]


def main() -> int:
    mod = importlib.import_module("nopaldb.nopaldb")
    text = STUB.read_text(encoding="utf-8")
    present = set(re.findall(r"def (\w+)", text)) | set(re.findall(r"class (\w+)", text))

    missing: list[str] = []
    for cname in CLASSES:
        cls = getattr(mod, cname, None)
        if cls is None:
            continue  # feature-gated class not built in this wheel
        if cname not in present:
            missing.append(f"class {cname}")
        for name in dir(cls):
            if name.startswith("__"):
                if name not in KEPT_DUNDERS:
                    continue
            elif name.startswith("_"):
                continue
            if name not in present:
                missing.append(f"{cname}.{name}")

    if missing:
        print("Stub out of date — missing symbols in nopaldb.pyi:")
        for m in sorted(missing):
            print(f"  - {m}")
        return 1
    print("check_stubs: OK — nopaldb.pyi covers all public symbols")
    return 0


if __name__ == "__main__":
    sys.exit(main())
