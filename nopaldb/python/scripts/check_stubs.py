#!/usr/bin/env python3
"""Guard: the stub `nopaldb.pyi` must match the built extension.

Runs `mypy.stubtest` against the compiled module: every public class,
method, property and module attribute is compared both ways (in the stub
but not at runtime, at runtime but not in the stub), and for callables the
parameter names, order, kinds and defaults are checked against the
`__text_signature__` pyo3 generates. Until 0.6.5 this script only compared
names, which let four wrong signatures (`bulk_loader`, `to_arrow`,
`to_arrow_complete`, `BulkLoader.add_edge`) ship for several releases.

Names that exist only for typing (TypedDicts, aliases) are listed in
`stubtest_allowlist.txt` next to this script, one regex per line with a
comment saying why. Add there, never `--ignore-missing-stub`: that flag
silences exactly the drift this guard exists to catch.

Run after `maturin develop -m nopaldb/Cargo.toml` (pyproject builds with
`python-full`, so the reasoner classes are present):

    python nopaldb/python/scripts/check_stubs.py
"""
from __future__ import annotations

import importlib
import subprocess
import sys
from pathlib import Path

ALLOWLIST = Path(__file__).resolve().with_name("stubtest_allowlist.txt")


def main() -> int:
    try:
        mod = importlib.import_module("nopaldb.nopaldb")
    except ImportError as e:
        print(f"check_stubs: cannot import the extension ({e}); build it first with "
              "`maturin develop -m nopaldb/Cargo.toml`")
        return 2
    if not hasattr(mod, "ELReasoner"):
        print("check_stubs: this build has no `ELReasoner` (built without python-full); "
              "the reasoner part of the stub is not verified")
    cmd = [sys.executable, "-m", "mypy.stubtest", "nopaldb.nopaldb",
           "--allowlist", str(ALLOWLIST), "--concise"]
    code = subprocess.call(cmd)
    if code == 0:
        print("check_stubs: OK — nopaldb.pyi matches the built extension (mypy stubtest)")
    else:
        print("check_stubs: stub out of date — fix nopaldb.pyi (or, for a type-only "
              f"name, add it with a reason to {ALLOWLIST.name})")
    return code


if __name__ == "__main__":
    sys.exit(main())
