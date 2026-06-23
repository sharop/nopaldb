# NopalDB MCP — Guía de configuración (Claude Desktop / Claude Code)

## Qué es esto

El servidor MCP de NopalDB permite a Claude hacer consultas sobre cualquier base de datos NopalDB usando lenguaje natural. Claude llama a las tools (`graph_query`, `schema_info`, `get_node`, etc.) de forma transparente — el usuario solo necesita preguntar.

---

## Instalación rápida

### Opción A — Desde el código fuente (recomendado)

```bash
# Desde la raíz del repositorio
cargo build -p nopaldb-mcp --release

# El binario queda en:
./target/release/nopaldb-mcp
```

### Opción B — Instalar globalmente

```bash
cargo install --path nopaldb-mcp
# Queda en ~/.cargo/bin/nopaldb-mcp
```

---

## Configurar Claude Desktop (macOS)

Edita `~/.config/claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "nopaldb": {
      "command": "/ruta/absoluta/a/nopaldb-mcp",
      "args": [
        "--db", "/ruta/absoluta/a/mi_grafo.db",
        "--readonly"
      ]
    }
  }
}
```

Reinicia Claude Desktop. Verás el icono de herramientas activo cuando la conexión sea exitosa.

### Ejemplo: Synthetic Offshore Network tutorial

```json
{
  "mcpServers": {
    "nopaldb-synthetic_offshore": {
      "command": "/Users/tu_usuario/Proyectos/nopaldb/target/release/nopaldb-mcp",
      "args": [
        "--db", "/Users/tu_usuario/Proyectos/nopaldb/test_dbs/synthetic_offshore.db",
        "--readonly"
      ]
    }
  }
}
```

### Modo escritura (permite ADD/UPDATE/DELETE)

Omite `--readonly`. Úsalo solo con DBs de desarrollo.

---

## Configurar Claude Code (CLI)

Agrega al `claude_desktop_config.json` global (mismo archivo que Desktop) o usa el flag `--mcp-config`:

```bash
# Probar sin configuración persistente
claude --mcp-server "nopaldb:/path/to/nopaldb-mcp --db /path/to/db.db --readonly"
```

O configura en `~/.config/claude/claude_desktop_config.json` como arriba.

---

## Flags disponibles

| Flag | Descripción |
|------|-------------|
| `--db <path>` | Ruta a la DB (default: `nopaldb.db` en CWD) |
| `--readonly` | Bloquea escrituras (ADD/UPDATE/DELETE/CREATE/DROP) |
| `--log-queries` | Loguea queries NQL a stderr (desactivado por default) |

---

## Ejemplos de uso con Claude

Una vez configurado, simplemente pregunta en lenguaje natural:

**Exploración inicial:**
> "¿Qué tipos de nodos y relaciones hay en esta base de datos?"

Claude llamará a `schema_info` y describirá la estructura.

**Búsqueda por nombre:**
> "Dame información sobre la entidad Atlas Fiduciary Holdings"

Claude usará `get_node` con `name = "Atlas Fiduciary Holdings"`.

**Análisis de red:**
> "¿Quiénes son las 5 entidades más influyentes en el grafo de control?"

Claude ejecutará `run_pagerank` con `top_n=5`.

**Trazado de caminos:**
> "¿Existe alguna conexión entre Alice Novak y la jurisdicción de Harbor Cay?"

Claude usará `find_path`.

**Consulta directa NQL:**
> "Muéstrame todas las entidades shell registradas en BVI con más de 3 directivos"

Claude construirá una query NQL y llamará a `graph_query`.

**Búsqueda semántica (requiere embeddings):**
> "¿Qué empresas son semánticamente similares a Atlas Fiduciary Holdings según sus descripciones?"

Claude usará `similar_nodes` con el modelo de embeddings correcto.

**Distribución de tipos de entidad:**
> "¿Cuántos nodos hay de cada tipo en este grafo?"

Claude usará `schema_by_kind` para obtener el desglose por label ordenado por frecuencia.

---

## Diagnóstico

### El servidor no aparece en Claude Desktop

1. Verifica que la ruta al binario es absoluta y el binario tiene permisos de ejecución (`chmod +x`)
2. Revisa los logs de Claude Desktop (`Help → Open Logs`)
3. Prueba el servidor manualmente: `echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | nopaldb-mcp --db mi.db`

### Error "database not found"

La ruta en `--db` debe existir. Si usas el tutorial, primero ejecuta el notebook o el generador:
```bash
python tutorials/shared/synthetic_offshore_dataset.py --db test_dbs/synthetic_offshore.db
```

### `similar_nodes` devuelve error "no embedding for model"

Los embeddings deben almacenarse antes de usar `similar_nodes`. Ejecuta el Paso 3 del notebook `02_synthetic_offshore.ipynb` para cargar los embeddings con `attach_node_embeddings`.

### Logging para debug

```bash
RUST_LOG=debug nopaldb-mcp --db mi.db --log-queries 2>mcp_debug.log
```
