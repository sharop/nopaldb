#!/usr/bin/env python3
"""Every test cited by docs/NQL_SEMANTICS.md (and its Spanish twin) exists.

The contracts are only as good as the tests behind them. If a test is
renamed or deleted, the contract it backs would silently lose its check:
this fails instead.

A citation is a backticked `suite_test::fn_name` (an integration test in
nopaldb/tests/suite_test.rs), a bare `suite_test` (the whole file), or
`graph::hybrid::tests::fn_name` (a unit test under nopaldb/src).
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TESTS = ROOT / "nopaldb" / "tests"
SRC = ROOT / "nopaldb" / "src"
DOCS = [ROOT / "docs" / "NQL_SEMANTICS.md", ROOT / "docs" / "es" / "SEMANTICA_NQL.md"]

CITATION = re.compile(r"`([a-z0-9_]+(?:::[a-z0-9_]+)*)`")


def fn_defined(text: str, name: str) -> bool:
    return re.search(rf"\bfn {re.escape(name)}\b", text) is not None


def check(citation: str) -> str | None:
    parts = citation.split("::")
    if parts[0].endswith("_test"):
        path = TESTS / f"{parts[0]}.rs"
        if not path.is_file():
            return f"{citation}: no existe {path.relative_to(ROOT)}"
        if len(parts) == 2 and not fn_defined(path.read_text(), parts[1]):
            return f"{citation}: {path.relative_to(ROOT)} no define fn {parts[1]}"
        return None
    if len(parts) >= 3 and parts[-2] == "tests":
        module = SRC.joinpath(*parts[:-2])
        candidates = [module.with_suffix(".rs"), module / "mod.rs"]
        for path in candidates:
            if path.is_file() and fn_defined(path.read_text(), parts[-1]):
                return None
        return f"{citation}: ningún test unitario {parts[-1]} en {module.relative_to(ROOT)}"
    return None  # no es una cita de test (nombres de ramas, columnas, etc.)


def main() -> int:
    errors, seen = [], 0
    for doc in DOCS:
        for citation in sorted(set(CITATION.findall(doc.read_text()))):
            if citation.endswith("_test") or "_test::" in citation or "::tests::" in citation:
                seen += 1
                error = check(citation)
                if error:
                    errors.append(f"{doc.relative_to(ROOT)}: {error}")
    for e in errors:
        print(e)
    if errors:
        return 1
    print(f"semantics: {seen} citas de tests, todas existen")
    return 0


if __name__ == "__main__":
    sys.exit(main())
