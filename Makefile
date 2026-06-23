SHELL := /bin/bash

CRATE_DIR := nopaldb
STUDIO_DIR := ndbstudio
DIST_DIR := dist
FEATURES ?= python-full
STUDIO_FEATURES ?= web
ALLOW_DIRTY ?= 0
OS := $(shell uname -s)
ARCH := $(shell uname -m)

CARGO_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(CRATE_DIR)/Cargo.toml | head -1)
PY_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(CRATE_DIR)/pyproject.toml | head -1)
STUDIO_VERSION_RAW := $(shell sed -n 's/^version = "\(.*\)"/\1/p' $(STUDIO_DIR)/Cargo.toml | head -1)
WORKSPACE_VERSION := $(shell awk '/^\[workspace.package\]/{flag=1;next}/^\[/{flag=0}flag && /^version = /{gsub(/"/,""); sub(/^version = /,""); print; exit}' Cargo.toml)
STUDIO_VERSION := $(if $(STUDIO_VERSION_RAW),$(STUDIO_VERSION_RAW),$(WORKSPACE_VERSION))

.PHONY: help check-tools check-clean check-version-sync \
	test test-core test-semantic test-full \
	clippy clippy-core clippy-semantic clippy-full \
	build-rust build-studio build-wheel build-wheel-all run-studio-web smoke-studio-web \
	package-bin package-studio package-studio-web package-studio-web-app sign-studio-web-app notarize-studio-web-app package-bundle package-qa package-qa-web package-qa-web-app checksums clean cargo-check-studio-web

help:
	@echo "Targets:"
	@echo "  make test               - tests default (sled only) + ndbstudio"
	@echo "  make test-core          - tests tier core"
	@echo "  make test-semantic      - tests tier semantic"
	@echo "  make test-full          - tests full public feature set"
	@echo "  make clippy             - clippy default"
	@echo "  make clippy-core        - clippy tier core"
	@echo "  make clippy-semantic    - clippy tier semantic"
	@echo "  make clippy-full        - clippy full public feature set"
	@echo "  make run-studio-web DB=path/to.db [BIND=127.0.0.1:3737]"
	@echo "  make smoke-studio-web DB=path/to.db [BIND=127.0.0.1:3737]"
	@echo "  make package-studio-web - empaqueta binario NDBStudio con soporte web"
	@echo "  make package-studio-web-app - genera NDBStudio Web.app para macOS"
	@echo "  make notarize-studio-web-app - firma, notariza y verifica NDBStudio Web.app"
	@echo "  make package-qa         - empaqueta nopaldb + wrapper python + ndbstudio"
	@echo "  make package-qa-web     - valida y empaqueta artefacto QA enfocado en NDBStudio Web"
	@echo "  make package-qa-web-app DB=path/to.db - smoke test + .app bundle para macOS"
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
	@echo "ndbstudio version: $(STUDIO_VERSION)"
	@[ "$(CARGO_VERSION)" = "$(PY_VERSION)" ] || (echo "Version mismatch entre nopaldb/Cargo.toml y nopaldb/pyproject.toml" && exit 1)
	@[ "$(CARGO_VERSION)" = "$(STUDIO_VERSION)" ] || (echo "Version mismatch entre nopaldb y ndbstudio" && exit 1)

# --- Tests por tier ---
# semantic/full son tiers Rust-only. Los bindings PyO3 se validan
# por separado con build-wheel, que usa maturin y enlaza contra Python.

test:
	cargo test -p nopaldb --lib
	cargo test -p ndbstudio

test-core:
	cargo test -p nopaldb --features core --lib

test-semantic:
	cargo test -p nopaldb --features semantic --lib

test-full:
	cargo test -p nopaldb --features full --lib
	cargo test -p ndbstudio

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

build-studio:
	cargo build -p ndbstudio --release --features $(STUDIO_FEATURES)

run-studio-web:
	@test -n "$(DB)" || (echo "Debes pasar DB=/ruta/a/base.db" && exit 1)
	cargo run -p ndbstudio --features web -- --web $(DB) $(if $(BIND),--bind $(BIND),)

smoke-studio-web:
	@test -n "$(DB)" || (echo "Debes pasar DB=/ruta/a/base.db" && exit 1)
	@command -v curl >/dev/null || (echo "curl no encontrado" && exit 1)
	@PORT="$${BIND##*:}"; \
	if [ -z "$$PORT" ] || [ "$$PORT" = "$(BIND)" ]; then PORT=3737; fi; \
	ADDR="$${BIND:-127.0.0.1:3737}"; \
	URL="http://$$ADDR"; \
	echo "Levantando NDBStudio Web en $$URL con DB=$(DB)"; \
	cargo run -p ndbstudio --features web -- --web $(DB) $(if $(BIND),--bind $(BIND),) >/tmp/ndbstudio-web-smoke.log 2>&1 & \
	PID=$$!; \
	trap 'kill $$PID >/dev/null 2>&1 || true' EXIT; \
	for i in 1 2 3 4 5 6 7 8 9 10; do \
		if curl -sf "$$URL/api/health" >/dev/null; then break; fi; \
		sleep 1; \
	done; \
	curl -sf "$$URL/api/health" >/dev/null || (echo "health check fallo"; cat /tmp/ndbstudio-web-smoke.log; exit 1); \
	curl -sf -X POST "$$URL/api/session/open" >/dev/null || (echo "session/open fallo"; cat /tmp/ndbstudio-web-smoke.log; exit 1); \
	curl -sf "$$URL/api/graph/subgraph?depth=1&limit=25" >/dev/null || (echo "graph/subgraph fallo"; cat /tmp/ndbstudio-web-smoke.log; exit 1); \
	curl -sf -X POST "$$URL/api/query/run" -H 'Content-Type: application/json' -d '{"query":"find n from (n) limit 5","run_mode":"run"}' >/dev/null || (echo "query/run fallo"; cat /tmp/ndbstudio-web-smoke.log; exit 1); \
	echo "Smoke test NDBStudio Web OK"

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

package-studio: build-studio
	@mkdir -p $(DIST_DIR)
	tar -czf $(DIST_DIR)/ndbstudio-v$(STUDIO_VERSION)-$(OS)-$(ARCH).tar.gz -C target/release ndbstudio

package-studio-web: build-studio
	@mkdir -p $(DIST_DIR)
	tar -czf $(DIST_DIR)/ndbstudio-web-v$(STUDIO_VERSION)-$(OS)-$(ARCH).tar.gz -C target/release ndbstudio

package-studio-web-app: build-studio
	@mkdir -p $(DIST_DIR)/NDBStudioWeb.app/Contents/MacOS
	@mkdir -p $(DIST_DIR)/NDBStudioWeb.app/Contents/Resources/bin
	sed 's/__VERSION__/$(STUDIO_VERSION)/g' packaging/macos/NDBStudioWeb/Info.plist > $(DIST_DIR)/NDBStudioWeb.app/Contents/Info.plist
	cp packaging/macos/NDBStudioWeb/NDBStudioWeb $(DIST_DIR)/NDBStudioWeb.app/Contents/MacOS/NDBStudioWeb
	cp target/release/ndbstudio $(DIST_DIR)/NDBStudioWeb.app/Contents/Resources/bin/ndbstudio
	chmod +x $(DIST_DIR)/NDBStudioWeb.app/Contents/MacOS/NDBStudioWeb
	chmod +x $(DIST_DIR)/NDBStudioWeb.app/Contents/Resources/bin/ndbstudio
	cd $(DIST_DIR) && zip -qry NDBStudioWeb-v$(STUDIO_VERSION)-$(OS)-$(ARCH).zip NDBStudioWeb.app

sign-studio-web-app: package-studio-web-app
	@test -n "$(CODESIGN_IDENTITY)" || (echo "Debes pasar CODESIGN_IDENTITY='Developer ID Application: ...'" && exit 1)
	codesign --force --deep --options runtime --timestamp --sign "$(CODESIGN_IDENTITY)" $(DIST_DIR)/NDBStudioWeb.app
	codesign --verify --deep --strict --verbose=2 $(DIST_DIR)/NDBStudioWeb.app

notarize-studio-web-app: package-studio-web-app
	@test -n "$(CODESIGN_IDENTITY)" || (echo "Debes pasar CODESIGN_IDENTITY='Developer ID Application: ...'" && exit 1)
	@test -n "$(NOTARY_PROFILE)" || (echo "Debes pasar NOTARY_PROFILE=<perfil notarytool>" && exit 1)
	CODESIGN_IDENTITY="$(CODESIGN_IDENTITY)" NOTARY_PROFILE="$(NOTARY_PROFILE)" packaging/macos/sign_notarize_ndbstudioweb.sh $(DIST_DIR)/NDBStudioWeb.app $(DIST_DIR)/NDBStudioWeb-notarize.zip

package-bundle: package-bin package-studio
	@mkdir -p $(DIST_DIR)/bundle
	cp $(DIST_DIR)/nopaldb-v$(CARGO_VERSION)-$(OS)-$(ARCH).tar.gz $(DIST_DIR)/bundle/
	cp $(DIST_DIR)/ndbstudio-v$(STUDIO_VERSION)-$(OS)-$(ARCH).tar.gz $(DIST_DIR)/bundle/
	tar -czf $(DIST_DIR)/nopal-suite-v$(CARGO_VERSION)-$(OS)-$(ARCH).tar.gz -C $(DIST_DIR)/bundle .

checksums:
	@mkdir -p $(DIST_DIR)
	@find $(DIST_DIR) -type f ! -name SHA256SUMS.txt -print0 | xargs -0 shasum -a 256 > $(DIST_DIR)/SHA256SUMS.txt

package-qa: check-tools check-clean check-version-sync test-full clippy-full package-bundle build-wheel checksums
	@echo "Artefactos QA generados en $(DIST_DIR)/"

package-qa-web: check-clean check-version-sync
	@test -n "$(DB)" || (echo "Debes pasar DB=/ruta/a/base.db" && exit 1)
	$(MAKE) cargo-check-studio-web
	$(MAKE) smoke-studio-web DB="$(DB)" $(if $(BIND),BIND="$(BIND)",)
	$(MAKE) package-studio-web
	$(MAKE) checksums
	@echo "Artefacto QA Web generado en $(DIST_DIR)/ndbstudio-web-v$(STUDIO_VERSION)-$(OS)-$(ARCH).tar.gz"

package-qa-web-app: check-clean check-version-sync
	@test -n "$(DB)" || (echo "Debes pasar DB=/ruta/a/base.db" && exit 1)
	$(MAKE) cargo-check-studio-web
	$(MAKE) smoke-studio-web DB="$(DB)" $(if $(BIND),BIND="$(BIND)",)
	$(MAKE) package-studio-web-app
	$(MAKE) checksums
	@echo "App bundle QA generado en $(DIST_DIR)/NDBStudioWeb-v$(STUDIO_VERSION)-$(OS)-$(ARCH).zip"

cargo-check-studio-web:
	cargo check -p ndbstudio --features web

clean:
	rm -rf $(DIST_DIR)
