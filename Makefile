SHELL := /bin/bash

CRATE_DIR := nopaldb
DIST_DIR := dist
FEATURES ?= python-full
ALLOW_DIRTY ?= 0
OS := $(shell uname -s)
ARCH := $(shell uname -m)

CARGO_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(CRATE_DIR)/Cargo.toml | head -1)
PY_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(CRATE_DIR)/pyproject.toml | head -1)
WORKSPACE_VERSION := $(shell awk '/^\[workspace.package\]/{flag=1;next}/^\[/{flag=0}flag && /^version = /{gsub(/"/,""); sub(/^version = /,""); print; exit}' Cargo.toml)

.PHONY: help check-tools check-clean check-version-sync \
	test test-core test-semantic test-full \
	clippy clippy-core clippy-semantic clippy-full \
	build-rust build-wheel build-wheel-all \
	package-bin package-qa checksums clean

help:
	@echo "Targets:"
	@echo "  make test               - tests default (sled only)"
	@echo "  make test-core          - tests tier core"
	@echo "  make test-semantic      - tests tier semantic"
	@echo "  make test-full          - tests full public feature set"
	@echo "  make clippy             - clippy default"
	@echo "  make clippy-core        - clippy tier core"
	@echo "  make clippy-semantic    - clippy tier semantic"
	@echo "  make clippy-full        - clippy full public feature set"
	@echo "  make package-qa         - valida y empaqueta nopaldb (binario + wheel python)"
	@echo "  make build-wheel        - wheel para PYTHON (default: python3), ej: PYTHON=python3.12"
	@echo "  make build-wheel-all    - wheels para Python 3.10, 3.11, 3.12 y 3.13 (los que existan)"

check-tools:
	@command -v cargo >/dev/null || (echo "cargo no encontrado" && exit 1)
	@command -v python3 >/dev/null || (echo "python3 no encontrado" && exit 1)
	@command -v maturin >/dev/null || (echo "maturin no encontrado (pip3 install maturin)" && exit 1)
	@command -v shasum >/dev/null || (echo "shasum no encontrado" && exit 1)

check-clean:
ifeq ($(ALLOW_DIRTY),1)
	@echo "ALLOW_DIRTY=1: se omite validacion de git limpio"
else
	@test -z "$$(git status --porcelain)" || (echo "Hay cambios sin commit. Limpia el árbol antes de empaquetar." && exit 1)
endif

check-version-sync:
	@echo "nopaldb Cargo version: $(CARGO_VERSION)"
	@echo "nopaldb Python version: $(PY_VERSION)"
	@echo "workspace version: $(WORKSPACE_VERSION)"
	@[ "$(CARGO_VERSION)" = "$(PY_VERSION)" ] || (echo "Version mismatch entre nopaldb/Cargo.toml y nopaldb/pyproject.toml" && exit 1)
	@[ "$(CARGO_VERSION)" = "$(WORKSPACE_VERSION)" ] || (echo "Version mismatch entre nopaldb y el workspace" && exit 1)

# --- Tests por tier ---
# semantic/full son tiers Rust-only. Los bindings PyO3 se validan
# por separado con build-wheel, que usa maturin y enlaza contra Python.

test:
	cargo test -p nopaldb --lib

test-core:
	cargo test -p nopaldb --features core --lib

test-semantic:
	cargo test -p nopaldb --features semantic --lib

test-full:
	cargo test -p nopaldb --features full --lib

# --- Clippy por tier ---

clippy:
	cargo clippy -p nopaldb -- -D warnings

clippy-core:
	cargo clippy -p nopaldb --features core -- -D warnings

clippy-semantic:
	cargo clippy -p nopaldb --features semantic -- -D warnings

clippy-full:
	cargo clippy -p nopaldb --features full -- -D warnings

# --- Build ---

build-rust:
	cargo build -p nopaldb --release

PYTHON ?= python3

build-wheel:
	@mkdir -p $(DIST_DIR)/wheels
	cd $(CRATE_DIR) && maturin build --release --features $(FEATURES) --interpreter $(PYTHON) -o ../$(DIST_DIR)/wheels

# Construye wheels para todas las versiones de Python >= 3.10 que estén instaladas.
# Usa: make build-wheel-all [FEATURES=python-full]
build-wheel-all:
	@mkdir -p $(DIST_DIR)/wheels
	@INTERPS=""; \
	for py in python3.10 python3.11 python3.12 python3.13; do \
		if command -v $$py >/dev/null 2>&1; then \
			INTERPS="$$INTERPS $$py"; \
			echo "Encontrado: $$($$py --version)"; \
		else \
			echo "No encontrado: $$py (se omite)"; \
		fi; \
	done; \
	if [ -z "$$INTERPS" ]; then \
		echo "No se encontró ningún intérprete Python 3.10-3.13" && exit 1; \
	fi; \
	cd $(CRATE_DIR) && maturin build --release --features $(FEATURES) --interpreter $$INTERPS -o ../$(DIST_DIR)/wheels
	@echo "Wheels generados en $(DIST_DIR)/wheels/"
	@ls $(DIST_DIR)/wheels/

# --- Package ---

package-bin: build-rust
	@mkdir -p $(DIST_DIR)
	tar -czf $(DIST_DIR)/nopaldb-v$(CARGO_VERSION)-$(OS)-$(ARCH).tar.gz -C target/release nopaldb

checksums:
	@mkdir -p $(DIST_DIR)
	@find $(DIST_DIR) -type f ! -name SHA256SUMS.txt -print0 | xargs -0 shasum -a 256 > $(DIST_DIR)/SHA256SUMS.txt

package-qa: check-tools check-clean check-version-sync test-full clippy-full package-bin build-wheel checksums
	@echo "Artefactos QA generados en $(DIST_DIR)/"

clean:
	rm -rf $(DIST_DIR)
