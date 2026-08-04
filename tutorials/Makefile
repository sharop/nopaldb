SHELL := /bin/bash
PYTHON ?= python3
NOTEBOOKS_DIR := notebooks
NBEXEC_OUT ?= /tmp/tutorial-nbexec
REPO_ROOT := $(shell cd .. && pwd)

.PHONY: help install install-min install-nopaldb smoke jupyter notebook nbexec-all clean \
	nbexec-ci

help:
	@echo "Targets:"
	@echo "  make install          - instala todas las deps (incluye embeddings)"
	@echo "  make install-min      - instala solo deps base (sin sentence-transformers)"
	@echo "  make install-nopaldb  - compila e instala el paquete Python nopaldb (maturin, requiere Rust)"
	@echo "  make jupyter          - abre Jupyter Lab/Notebook en notebooks/"
	@echo "  make smoke            - ejecuta 00_setup_smoke_test.ipynb headless"
	@echo "  make nbexec-all       - ejecuta los 5 notebooks headless (CI gate)"
	@echo "  make nbexec-ci        - ejecuta actos 1-4 headless fuera del checkout"
	@echo ""
	@echo "  make clean            - limpia precomputed/ y *.ipynb_checkpoints"

install:
	$(PYTHON) -m pip install -r requirements.txt

install-min:
	awk '!/sentence-transformers/ && !/^torch/ && !/^[[:space:]]*#/ && !/^[[:space:]]*$$/' requirements.txt > /tmp/nopaldb-tutorial-requirements.txt
	$(PYTHON) -m pip install -r /tmp/nopaldb-tutorial-requirements.txt

# Compila el crate nopaldb con maturin (lee features de nopaldb/pyproject.toml: python-full)
# e instala el wheel en el entorno Python activo. Requiere toolchain de Rust.
install-nopaldb:
	$(PYTHON) -m pip install $(REPO_ROOT)/nopaldb

jupyter:
	cd $(NOTEBOOKS_DIR) && $(PYTHON) -m jupyter notebook

smoke:
	$(PYTHON) -m jupyter nbconvert --to notebook --execute --inplace \
		$(NOTEBOOKS_DIR)/00_setup_smoke_test.ipynb

nbexec-all:
	@for nb in 00_setup_smoke_test 01_florentine_families 02_synthetic_offshore 03_biomedical_owl 04_synthetic_fraud; do \
		if [ -f "$(NOTEBOOKS_DIR)/$$nb.ipynb" ]; then \
			echo "Ejecutando $$nb.ipynb"; \
			$(PYTHON) -m jupyter nbconvert --to notebook --execute --inplace \
				$(NOTEBOOKS_DIR)/$$nb.ipynb || exit 1; \
		fi; \
	done
	@echo "Todos los notebooks ejecutaron correctamente."

nbexec-ci:
	@mkdir -p "$(NBEXEC_OUT)"
	@for nb in 01_florentine_families 02_synthetic_offshore 03_biomedical_owl 04_synthetic_fraud; do \
		if [ -f "$(NOTEBOOKS_DIR)/$$nb.ipynb" ]; then \
			echo "Ejecutando $$nb.ipynb -> $(NBEXEC_OUT)"; \
			$(PYTHON) -m jupyter nbconvert --to notebook --execute \
				--output-dir "$(NBEXEC_OUT)" \
				$(NOTEBOOKS_DIR)/$$nb.ipynb || exit 1; \
		fi; \
	done
	@echo "Notebooks CI ejecutados correctamente en $(NBEXEC_OUT)."

clean:
	rm -rf precomputed/*.npy precomputed/*.json
	find $(NOTEBOOKS_DIR) -type d -name .ipynb_checkpoints -exec rm -rf {} + 2>/dev/null || true
