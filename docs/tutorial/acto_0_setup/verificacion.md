# Acto 0 — Checklist de verificación

Ejecuta cada bloque y confirma que el output es el esperado. Si alguno falla, hay un diagnóstico debajo.

## 1. Python + binding NopalDB

```bash
python -c "import nopaldb; print(nopaldb.__version__ if hasattr(nopaldb, '__version__') else 'OK')"
```

Esperado: imprime `0.4.19` o `OK`.

**Si falla con `ModuleNotFoundError`:** el wheel no se instaló. Repite `pip install dist/wheels/nopaldb-*.whl`.

### 1.1 Verificar que el wheel trae `algorithms`

```bash
python -c "
import nopaldb
g = nopaldb.Graph.in_memory()
g.execute_nql('add (n:Test {x: 1})')
res = g.execute_nql('find pagerank(n) as pr from (n:Test)')
print(list(res))
"
```

Esperado: `[{'pr': 0.0}]` (un solo nodo aislado tiene pr=0). Si falla con
`Query execution error: pagerank() requires feature 'algorithms'` el wheel
fue construido sin features. Reconstruye:

```bash
cd /path/to/nopaldb
make build-wheel
pip install --force-reinstall dist/wheels/nopaldb-*.whl
```

`make build-wheel` usa `python-full` por defecto: wrapper Python + analytics,
reasoner OWL/Turtle y las capacidades necesarias para los actos 1-4.

## 2. Helper `shared.load_nql`

```bash
cd tutorials
python -c "from shared import load_nql, list_queries; print(list_queries('acto_1'))"
```

Esperado: lista con `['01_modelo.nql', '02_pattern_matching.nql', '03_centralidad.nql', '04_communities.nql']`.

**Si falla con `ImportError`:** estás corriendo Python desde un directorio incorrecto. `shared/` se importa relativo a `tutorials/`. Asegúrate de estar dentro de `tutorials/` o agrega el path:
```python
import sys; sys.path.insert(0, "/path/to/nopaldb/tutorials")
```

## 3. Cargo (Rust)

```bash
cargo --version
```

Esperado: `cargo 1.7x.x` o superior.

**Si no está:** instala con `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.

## 4. NDBStudio Web build

```bash
cargo check -p ndbstudio --features web
```

Esperado: compila sin errores en <60s (la primera vez puede tardar más por descarga de deps).

**Si falla con errores de OpenSSL/SDK:** consulta `docs/ndbstudio/web_quickstart.md` para troubleshooting macOS.

## 5. Smoke test de NDBStudio Web

Necesita una DB válida primero. Genera la de Florentine:

```bash
python nopaldb/examples/florentine_families_dataset.py \
  --db test_dbs/florentine_families.db --reset
```

Y luego:

```bash
make smoke-studio-web DB=test_dbs/florentine_families.db
```

Esperado: imprime `Smoke test NDBStudio Web OK` después de ~10s.

**Si falla `health check fallo`:** revisa `/tmp/ndbstudio-web-smoke.log`. Causa común: el puerto 3737 está ocupado. Pasa `BIND=127.0.0.1:3838`.

## 6. Notebook headless

```bash
cd tutorials
make smoke
```

Esperado: el notebook `00_setup_smoke_test.ipynb` se ejecuta sin errores y queda con outputs frescos.

**Si falla:** revisa que `nopaldb` esté instalado en el mismo Python que usa Jupyter. `which python` y `which jupyter` deberían apuntar al mismo entorno.

---

Si todos los pasos pasan, estás listo para [Acto 1](../acto_1_florentine/README.md).
