# NDBStudio Keybindings

## Global (modo NORMAL)

- `?`: abrir/cerrar Help Modal.
- `Ctrl+k`: abrir Command Palette.
- `Tab`: alterna foco `Editor` / `Results`.
- `1`: foco a `Editor`.
- `2`: foco a `Results`.
- `s`: abrir/cerrar panel `Schema`.
- `x`: abrir/cerrar panel `Graph`.
- `Ctrl+h` / `Ctrl+l`: ajustar ancho del panel lateral.
- `:`: entrar a command mode.
- `i`: entrar a insert mode.
- `Enter`: ejecutar query.
- `t`: cambiar modo de `Results` (`Table` -> `JSON` -> `Graph`).
- `t`: cambiar modo de `Results` (`Table` -> `JSON` -> `Graph` -> `Plan`).
- `g` / `G`: top / bottom en `Results`.
- `q` (solo en pantalla de historial): salir de historial.
- `y`: mostrar/ocultar Timeline (Session v2).
- `R`: re-ejecutar ultimo query del Timeline (Session v2).
- `[` / `]`: tab anterior/siguiente (Session v2).
- `Ctrl+t`: crear tab nuevo (Session v2).
- `Ctrl+w`: cerrar tab activo (Session v2).
- `b`: abrir Session Browser (Session v2).

## Navegacion de Results (con foco en Results)

- `j` / `k`: scroll abajo/arriba.
- `Up` / `Down`: scroll abajo/arriba.
- `PageDown` / `PageUp`: scroll rapido.
- `Home` / `End`: inicio/fin.
- En modo `Plan`: `j/k` navega operadores del plan y `z` colapsa/expande seccion.

## Editor

### En modo INSERT

- `Esc`: volver a `NORMAL`.
- `Ctrl+Enter`: ejecutar query.
- `Enter`: nueva linea.
- `Backspace`: borrar caracter.
- `Up` / `Down` / `Left` / `Right`: mover cursor.
- `Ctrl+d`: borrar linea actual.
- `Ctrl+u`: limpiar editor completo.

### En modo NORMAL (con foco en Editor)

- `j` / `k`: mover linea abajo/arriba.
- `Up` / `Down`: mover linea abajo/arriba.
- `PageDown` / `PageUp`: salto vertical rapido.
- `Ctrl+d`: borrar linea actual.
- `Ctrl+u`: limpiar editor completo.

## Panel Schema

- `Shift+J` / `Shift+K`: scroll del panel.
- `s`: ocultar panel.

## Panel Graph

- `Shift+J` / `Shift+K`: seleccionar vecino.
- `o`: enfocar vecino seleccionado.
- `Shift+Enter`: enfocar vecino seleccionado.
- `+` / `-`: profundidad (`1..3`).
- `f`: intentar enfocar nodo desde fila actual de `Results`.
- `r`: refrescar snapshot del grafo.
- `x`: ocultar panel.

## Historial

- `:history`: abrir historial.
- `Esc` o `q`: cerrar historial.
- `Ctrl+p` / `Ctrl+n`: navegar queries previas/siguientes.

## Session Browser (Session v2)

- `Tab`: cambiar panel (`Timeline` / `Snippets` / `Tabs`).
- `j` / `k` o `Down` / `Up`: mover seleccion.
- `Enter` o `r`: ejecutar query seleccionada (o activar tab seleccionado).
- `l`: cargar query seleccionada al editor (o activar tab).
- `p`: pin/unpin entrada seleccionada en `Timeline`.
- `g` (en Timeline): abrir DAG de linaje de la entrada seleccionada.
- `d` (en Timeline): abrir tabla de impacto (score) de la entrada seleccionada.
- `/`: entrar a edicion de filtro rapido.
- Filtro rapido soporta `mode:run`, `mode:explain`, `mode:profile`.
- En filtro: `Enter` aplicar, `Backspace` borrar, `Ctrl+u` limpiar, `Esc` salir de edicion.
- `q` o `Esc`: volver a modo normal.

## Help Modal

- `?`, `q` o `Esc`: cerrar.
- `j/k` o `Up/Down`: scroll.
- `PageUp/PageDown`, `Home/End`: navegación rápida.

## Command Palette

- `Ctrl+k`: abrir.
- Escribe para filtrar acciones/comandos/snippets/tabs/timeline.
- Incluye acciones directas: `Run Query`, `Explain Query`, `Profile Query`.
- `j/k` o `Up/Down`: mover selección.
- `Enter`: ejecutar acción seleccionada.
- `Esc` o `q`: cerrar.
