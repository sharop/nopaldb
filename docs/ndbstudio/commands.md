# NDBStudio Commands

Comandos disponibles en command mode (`:`).

## Generales

- `:q` / `:quit`: salir.
- `:help`: abre Help Modal (comandos + atajos).
- `:history`: abrir vista de historial.
- `:session`: resumen de estado de sesion (v2).
- `:run`: ejecuta el query actual del editor.
- `:explain`: ejecuta `EXPLAIN` sobre el query actual.
- `:profile`: ejecuta query + vista tabular de metricas de perfil.

## Paneles

- `:schema`: abrir/cerrar schema panel.
- `:graph`: abrir/cerrar graph panel.
- `:graph refresh`: refrescar snapshot de nodos/relaciones.

## Results

- `:results table`
- `:results json`
- `:results graph`
- `:results plan` (vista dedicada para `EXPLAIN` / `PROFILE`)

En `Plan`, hay navegacion interactiva:

- `j/k`: mover seleccion entre operadores detectados.
- `z`: colapsar/expandir seccion activa.
- `Plan Detail` muestra `Cost Score` estimado (0-100) con leyenda `LOW/MEDIUM/HIGH`.

## Graph filter/focus

- `:graph labels`: listar labels disponibles en `Results`.
- `:graph label <Label>`: filtrar panel de grafo por label.
- `:graph label *`: remover filtro.
- `:graph focus <node_id>`: enfocar nodo por ID.
- `:graph focus name "Jon Snow"`: enfocar nodo por propiedad `name`/`title`.

Notas:

- `:graph focus name` intenta match exacto primero, luego parcial.
- Si hay multiples matches, enfoca el primero y lo reporta.

## Editor

- `:clear` o `:editor clear`: limpiar editor.
- `:editor delete-line` (alias `:editor delline`): borrar linea actual.

## Session v2: timeline, tabs, snippets

Requiere activar:

```bash
NDBSTUDIO_SESSION_V2=1 cargo run -p ndbstudio -- <db_path>
```

Timeline:

- `:timeline` (mostrar/ocultar panel)
- `:browser` (abre Session Browser interactivo)
- `:browser filter <text>`
- `:browser filter clear`
- `:browser mode <run|explain|profile|all>` (shortcut de filtro por modo)
- `:timeline rerun last`
- `:timeline rerun <n>` (indice del panel timeline)
- `:timeline rerun <n> --as <run|explain|profile>` (override del modo al re-ejecutar)
- `:timeline rerun dependents <n>` (re-ejecuta los runs que dependen de la entrada `<n>`)
- `:timeline rerun impacted <n> [--threshold N]` (re-ejecuta dependientes impactados por score)
- `:timeline lineage <n>` (resumen de dependencias/dependientes de `<n>`)
- `:timeline dag <n>` (proyección del vecindario de linaje en Results Graph)
- `:timeline impact <n> [--threshold N]` (tabla priorizada de impacto)
- `:timeline pin <n>` (pin/unpin de entrada de timeline)
- `:cache stats` (hits/misses/capacidad/revisiones)
- `:cache stats session` (hit-rate de la sesión)
- `:cache stats tab` (hit-rate del tab activo)
- `:cache recent [session|tab] [N]` (hit-rate en ventana reciente, default 20)
- `:cache clear` (limpia cache en memoria)
- `:params` (listar parametros activos)
- `:param set <name> <value>` (define parametro de sesion, soporta `$name`)
- `:param unset <name>`
- `:params clear`

Tabs:

- `:tab new [titulo]`
- `:tab next`
- `:tab prev`
- `:tab close`
- `:tabs` (lista tabs en Results)

Snippets:

- `:save <name>` (guarda query actual del editor)
- `:snippets` (lista snippets)
- `:snippet run <name>` (carga snippet al editor)

## Export (placeholder)

- `:export csv`
- `:export json`
- `:export arrow`

Estado actual: comando visible en UI, export de archivo aun marcado como pendiente en codigo.
- Timeline/Session Browser ahora muestra badge de cache por ejecución: `HIT` / `MISS`.
- Timeline/Session Browser muestra `dN` para cantidad de dependencias del run.
- `:timeline rerun dependents` prioriza por impacto semántico (labels/edges/properties compartidos).
