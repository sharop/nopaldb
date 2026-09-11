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

.PHONY: help check-tools check-clean check-version-sync check-changelog-dates date-changelog \
	test test-core test-semantic test-full \
	clippy clippy-core clippy-semantic clippy-full \
	build-rust build-wheel build-wheel-all \
	package-bin package-qa checksums clean check-on-main publish-crate bench

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
	@echo "  make check-doc-links    - links relativos de docs/ y READMEs apuntan a archivos que existen"
	@echo "  make publish-crate      - cargo publish SOLO desde main al dia (check-on-main + checks)"
	@echo "  make bench BENCH=x      - cargo bench con panic=unwind (hnsw_ops | gc_removals | graph_ops)"
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

bench:
	@# `cargo bench` a secas falla desde un target limpio: el perfil release
	@# lleva panic = "abort" y el harness de criterion exige unwind, así que
	@# las dependencias compiladas para release chocan ("requires panic
	@# strategy abort"). El override compila las deps con unwind solo aquí.
	@# Uso: make bench BENCH=hnsw_ops   (o gc_removals, graph_ops)
	@#      NOPALDB_BENCH_ENGINE=redb make bench BENCH=gc_removals
	@test -n "$(BENCH)" || { echo "uso: make bench BENCH=hnsw_ops|gc_removals|graph_ops"; exit 1; }
	CARGO_PROFILE_RELEASE_PANIC=unwind cargo bench -p nopaldb --features core,storage-redb --bench $(BENCH)

check-on-main:
	@# `cargo publish` salió desde una rama dos veces (0.5.3 y 0.5.9). Las dos
	@# veces el árbol resultó byte-idéntico a main, por suerte y no por diseño:
	@# el SHA que crates.io registró apunta a un commit que ya no existe en
	@# GitHub. El paso "checkout main + pull" del runbook es el que se salta;
	@# aquí deja de ser un paso y pasa a ser una condición.
	@b=$$(git branch --show-current); \
	[ "$$b" = "main" ] || { echo "Estás en '$$b'. Se publica SOLO desde main: git checkout main && git pull --ff-only"; exit 1; }
	@git fetch -q origin main; \
	[ "$$(git rev-parse HEAD)" = "$$(git rev-parse origin/main)" ] || { echo "main local no es origin/main: git pull --ff-only"; exit 1; }
	@echo "git: en main y al día con origin/main"

publish-crate: check-on-main check-clean check-version-sync check-changelog-dates
	cargo publish -p nopaldb --dry-run
	cargo publish -p nopaldb

check-version-sync:
	@echo "nopaldb Cargo version: $(CARGO_VERSION)"
	@echo "nopaldb Python version: $(PY_VERSION)"
	@echo "workspace version: $(WORKSPACE_VERSION)"
	@[ "$(CARGO_VERSION)" = "$(PY_VERSION)" ] || (echo "Version mismatch entre nopaldb/Cargo.toml y nopaldb/pyproject.toml" && exit 1)
	@[ "$(CARGO_VERSION)" = "$(WORKSPACE_VERSION)" ] || (echo "Version mismatch entre nopaldb y el workspace" && exit 1)

date-changelog:
	@# Pone en el CHANGELOG la fecha REAL de publicación, tomada de
	@# crates.io. Existe porque el paso manual falla de dos formas: se
	@# olvida (cinco releases quedaron como "unreleased" hasta que lo
	@# reportó alguien de fuera) o se pega el marcador literal en vez de
	@# la fecha. Ninguna de las dos puede pasar si el dato lo trae el make.
	@test -n "$(VERSION)" || { echo "uso: make date-changelog VERSION=0.5.8"; exit 1; }
	@fecha=$$(curl -s -H "User-Agent: nopaldb-release" \
	    https://crates.io/api/v1/crates/nopaldb \
	  | python3 -c "import json,sys; v=[x for x in json.load(sys.stdin)['versions'] if x['num']=='$(VERSION)']; print(v[0]['created_at'][:10] if v else '')"); \
	if [ -z "$$fecha" ]; then \
	  echo "$(VERSION) no está publicada en crates.io todavía; publicar antes de fechar."; \
	  exit 1; \
	fi; \
	if ! grep -q "^## \[$(VERSION)\]" CHANGELOG.md; then \
	  echo "no hay entrada '## [$(VERSION)]' en CHANGELOG.md"; exit 1; \
	fi; \
	sed -i '' -E "s/^## \[$(VERSION)\] - .*/## [$(VERSION)] - $$fecha/" CHANGELOG.md; \
	echo "CHANGELOG: $(VERSION) fechada $$fecha"
	@$(MAKE) --no-print-directory check-changelog-dates

check-doc-links:
	@# Tres READMEs de docs enlazaron meses a dos roadmaps que no existían.
	@# Un link roto en el índice es lo primero que ve quien evalúa contribuir.
	@python3 scripts/check_doc_links.py

check-changelog-dates:
	@# Toda versión con tag debe tener FECHA en el CHANGELOG, no "unreleased".
	@# El chore del bump escribe "unreleased" y hasta 0.5.7 nadie ponía la
	@# fecha al publicar: cinco releases quedaron rotuladas como no liberadas
	@# en el artefacto que la gente lee. Lo reportó un integrador externo, no
	@# nosotros — de ahí que esto sea un check y no una nota en el runbook.
	@#
	@# Solo se exige fecha a las versiones que TIENEN entrada: el CHANGELOG
	@# empieza en 0.4.28 y los tags anteriores nunca tuvieron una.
	@fail=0; \
	for tag in $$(git tag --list 'v*'); do \
	  ver=$${tag#v}; \
	  if grep -q "^## \[$$ver\]" CHANGELOG.md && \
	     ! grep -q "^## \[$$ver\] - [0-9]" CHANGELOG.md; then \
	    echo "  ✗ $$ver tiene tag pero su entrada del CHANGELOG no tiene fecha"; \
	    fail=1; \
	  fi; \
	done; \
	if [ $$fail -eq 1 ]; then \
	  echo "Corregir: '## [X.Y.Z] - unreleased' → la fecha real de publicación."; \
	  exit 1; \
	fi; \
	echo "CHANGELOG: todas las versiones con tag y entrada tienen fecha"

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

package-qa: check-tools check-clean check-version-sync check-changelog-dates check-doc-links test-full clippy-full package-bin build-wheel checksums
	@echo "Artefactos QA generados en $(DIST_DIR)/"

clean:
	rm -rf $(DIST_DIR)
