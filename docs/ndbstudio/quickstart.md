# NDBStudio Quickstart

> Para la workbench web local, ver [NDBStudio Web Quickstart](web_quickstart.md).

## 1) Ejecutar

Desde la raiz del repo:

```bash
cargo run -p ndbstudio -- ./ruta/a/tu.db
```

Ejemplo:

```bash
cargo run -p ndbstudio -- ../synthetic_offshore/data/synthetic_offshore.db
```

Si quieres desactivar la pantalla de loading:

```bash
NDBSTUDIO_NO_LOADING=1 cargo run -p ndbstudio -- ./ruta/a/tu.db
```

## 2) Flujo basico

1. Escribe query en `Editor` (panel superior izquierdo).
2. Ejecuta con `Enter` en modo `NORMAL`, o `Ctrl+Enter` en `INSERT`.
3. Revisa `Results` (panel inferior izquierdo).
4. Cambia visualizacion de resultados con `t` o `:results <mode>`.

## 3) Navegacion minima

- `Tab` cambia foco entre `Editor` y `Results`.
- `2` mueve foco a `Results`.
- `s` abre/cierra `Schema`.
- `x` abre/cierra `Graph`.
- `Ctrl+h` / `Ctrl+l` cambia ancho de panel lateral.

## 4) Primeros queries utiles

Validar datos:

```sql
find c.name, c.house
from (c:Character)
limit 20
```

Casas enemigas (Synthetic Character Network):

```sql
find c.house as house, e.house as enemy_house, count(*) as total
from (c:Character)-[:ENEMY_OF]->(e:Character)
group by c.house, e.house
order by total desc
```

Casas aliadas (Synthetic Character Network):

```sql
find c.house as house, a.house as allied_house, count(*) as total
from (c:Character)-[:ALLIED_WITH]->(a:Character)
group by c.house, a.house
order by total desc
```

## 5) Salir

- `:q` o `:quit`.
