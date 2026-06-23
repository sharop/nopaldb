# Acto 0 — Setup

**Tiempo estimado:** 20 minutos
**Pre-requisitos:** Rust toolchain, Python ≥3.10, `cargo`, `pip`.

Antes de empezar el tutorial, montamos los tres componentes que vamos a usar:

1. **NopalDB** — la base de datos en sí, vía wheel de Python (binding PyO3) o el ejemplo Rust.
2. **Jupyter** + helpers `shared/` (en `tutorials/`).
3. **NDBStudio Web** — la UI interactiva con visualización de grafo.

---

## 1. Construir e instalar NopalDB para Python

NopalDB se distribuye como wheel binario que incluye el motor en Rust. Lo construimos desde el repo (en local; el tutorial está pinneado a v0.4.19):

```bash
cd /path/to/nopaldb
make build-wheel                          # usa python-full por defecto
pip install dist/wheels/nopaldb-*.whl
```

> `make build-wheel` compila el wrapper Python con `maturin` y la feature `python-full`. `cargo build --features full` solo compila la librería Rust.

Verificación:

```bash
python -c "import nopaldb; g = nopaldb.Graph.in_memory(); print(g.execute_nql('add (n:Test {x: 1})').summary)"
```

Debe imprimir algo como `Wrote 1 node(s)`.

---

## 2. Instalar las deps del tutorial

```bash
cd tutorials
pip install -r requirements.txt
```

Esto trae `jupyter`, `pandas`, `pyarrow`, `matplotlib`, `networkx`, `numpy`, y `sentence-transformers` + `torch` (para los embeddings de Actos 2 y 4).

> Si quieres saltarte sentence-transformers (ej. solo te interesa Acto 1), corre `make install-min` desde `tutorials/` — instala todo menos los modelos de embeddings.

---

## 3. NDBStudio Web

NDBStudio es la UI: editor NQL + visualización de grafo + timeline + schema panel. Lo construimos una vez con el feature `web` y lo levantamos por DB:

```bash
# Desde la raíz del repo
make run-studio-web DB=test_dbs/florentine_families.db
```

Por defecto bindea en `http://127.0.0.1:3737`. Puedes pasar `BIND=0.0.0.0:8080` para cambiar.

Cada acto del tutorial trae un `Makefile target` que lanza la UI con la DB correspondiente:

```bash
cd tutorials
make studio-florentine    # Acto 1
make studio-offshore        # Acto 2 (después de generarla)
make studio-fraud         # Acto 4 (después de generarla)
```

---

## 4. Verificación end-to-end

Corre el smoke test del notebook 0:

```bash
cd tutorials
make smoke      # ejecuta 00_setup_smoke_test.ipynb headless
```

Si pasa, sigue con [Acto 1 — Florentine Families](../acto_1_florentine/README.md).

Si falla, revisa [verificacion.md](verificacion.md) para diagnóstico rápido.

---

## ¿Qué pasa si no tengo Rust toolchain?

El binding Python ya viene compilado en el wheel — el toolchain de Rust solo se necesita para:
- **Construir el wheel** (`make build-wheel`)
- **Levantar NDBStudio Web** (`make run-studio-web`)
- **Correr los ejemplos Rust** del tutorial

Si solo te interesan los notebooks Python (3 de los 4 medios), basta con que alguien te pase un wheel pre-construido. Pero perdés:
- la visualización interactiva de NDBStudio,
- el cross-check del ejemplo Rust (gate de drift).

Recomendado: instalar Rust con [rustup](https://rustup.rs/). Toma <5 min.
