# Architecture

Vista de alto nivel de `ndbstudio`.

## Objetivo

Proveer un cliente TUI para explorar y consultar NopalDB, con:

- Editor de queries NQL.
- Ejecucion no bloqueante.
- Resultados en multiples formatos.
- Panel lateral de schema y grafo.

## Componentes principales

- `src/main.rs`
  - Bootstrap de terminal.
  - Loading screen durante apertura de DB.
  - Event loop principal.
  - Guard de limpieza de terminal al salir.

- `src/app.rs`
  - Estado global de la aplicacion (`App`).
  - Manejo de modos (`NORMAL`, `INSERT`, `COMMAND`, `HISTORY`).
  - Keybindings y comandos `:`.
  - Orquestacion de queries async en worker thread.
  - Sincronizacion de schema/graph view despues de writes.

- `src/ui/*.rs`
  - `editor.rs`: edicion multilinea NQL.
  - `results.rs`: render table/json/graph con scroll.
  - `schema.rs`: schema browser lateral.
  - `graph.rs`: neighborhood view, focus y filtros.
  - `mod.rs`: layout general, header y status bar.

- `src/engine/mapper.rs`
  - Conversion de `NqlResult` a estructura tabular usada por Results.

## Flujo de ejecucion de query

1. Usuario ejecuta query desde `Editor`.
2. `App` crea job y lanza worker thread.
3. Worker crea runtime local Tokio y llama `graph.execute_statement(...)`.
4. Resultado se transforma con `mapper` a headers/rows.
5. Main loop hace `poll_pending_query()` y actualiza UI.
6. Si hubo cambio de schema por write, se dispara `rebuild_schema()`.

## Persistencia de estado UI

Archivo:

- `~/.ndstudio/ui_state.json`

Actualmente persiste:

- Panel lateral activo (`none`, `schema`, `graph`).
- Ancho del panel lateral.

## Decisions clave

- Queries en background para mantener UI responsiva.
- Grafo como snapshot on-demand (`refresh`) para control de costo.
- Mensajes de error con cadena completa de causas para debug rapido.
- Result modes (`table/json/graph`) para diferentes necesidades de inspeccion.

## Limites actuales

- Export por comando `:export` esta en placeholder (pendiente de escritura real a archivo).
- Visualizacion de grafo en `Results` es proyeccion textual, no canvas interactivo.
- Algunos atajos dependen del modo/foco activo.

