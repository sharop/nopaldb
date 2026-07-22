# Tutoriales NopalDB

Tutorial avanzado en varios actos (Florentine Families, Synthetic Offshore Network,
Biomedical/OWL, Synthetic Fraud) en forma de notebooks de Jupyter, con datasets
deterministas y ejemplos en Rust y Python.

## Requisitos previos

- **Python ≥ 3.10**
- **Toolchain de Rust** — el paquete Python `nopaldb` se compila desde el código fuente
  con [maturin]; **no está publicado en PyPI**. La versión queda fijada por
  [`rust-toolchain.toml`](../rust-toolchain.toml) en la raíz del repo.

## Instalación

Desde este directorio (`tutorials/`):

```bash
# 1. Dependencias Python de los notebooks
make install-min      # base, sin embeddings (sentence-transformers / torch)
# o, con embeddings reales (Actos 2 y 4):
make install

# 2. Compilar e instalar el paquete Python nopaldb (maturin, requiere Rust)
make install-nopaldb
```

`make install-nopaldb` ejecuta `pip install ../nopaldb`, que usa maturin (declarado como
build-backend en [`../nopaldb/pyproject.toml`](../nopaldb/pyproject.toml)) para compilar
el crate con la feature `python-full` e instalar el wheel en tu entorno Python activo.

Verifica la instalación:

```bash
python3 -c "import nopaldb; print(nopaldb.__version__)"
```

> **Nota:** `nopaldb` ya **no** aparece en `requirements.txt` ni en `pyproject.toml`
> porque no se instala desde PyPI sino desde el código fuente. Por eso es un paso aparte
> (`make install-nopaldb`).

## Ejecutar los notebooks

```bash
make jupyter          # abre Jupyter en notebooks/
make smoke            # ejecuta 00_setup_smoke_test.ipynb headless
make nbexec-all       # ejecuta todos los notebooks headless (gate de CI)
```

Consulta todos los targets con `make help`.

## Ejemplo: second brain en ~20 líneas

Un ejemplo mínimo y autónomo (no notebook) que ingesta una carpeta de markdown
estilo Obsidian a NopalDB y la consulta con búsqueda híbrida + el grafo de
wikilinks. Corre offline (embeddings deterministas de fallback):

```bash
python second_brain/ingest.py --db ../test_dbs/second_brain.db --reset
python second_brain/query.py  --db ../test_dbs/second_brain.db
```

Ver [`second_brain/README.md`](second_brain/README.md).