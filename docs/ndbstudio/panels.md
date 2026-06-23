# Panels

## Layout general

- Izquierda arriba: `Query Editor`.
- Izquierda abajo: `Results`.
- Derecha (colapsable): `Schema` o `Graph`.

`Schema` y `Graph` son mutuamente excluyentes.

## Query Editor

Responsabilidades:

- Edicion multilinea de NQL.
- Navegacion basica de cursor.
- Ejecucion de query (directa o desde insert).

Operaciones clave:

- Borrar linea actual (`Ctrl+d`).
- Limpiar editor (`Ctrl+u` o `:editor clear`).

## Results

Responsabilidades:

- Mostrar resultados tabulares.
- Mostrar errores de ejecucion con cadena completa de causas.
- Mostrar tiempo de ejecucion de query en el titulo.
- Cambiar visualizacion entre:
  - `Table`
  - `JSON`
  - `Graph` (proyeccion textual desde columnas)

Estado de ejecucion:

- Al iniciar query: `Query en progreso...`.
- Al terminar: resumen y tiempo en status/header.

## Schema panel

Muestra:

- Labels de nodos y sus propiedades.
- Tipos de edge y sus propiedades.
- Estadisticas generales (`total nodes`, `total edges`, `avg degree`, `density`).

## Graph panel

Muestra:

- Snapshot de grafo en memoria (nodos/edges actuales de la DB).
- Vista de vecindad del nodo en foco.
- Vecinos navegables y profundidad de alcance.
- Filtro por label (`:graph label <Label>`).

Focus:

- Por ID: `:graph focus <id>`.
- Por nombre: `:graph focus name "..."`.
- Desde `Results`: tecla `f` (si hay ID o valores `name/title` en la fila actual).

