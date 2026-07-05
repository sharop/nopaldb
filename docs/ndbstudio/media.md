# Media Guide (Screenshots y GIFs)

Guia para capturar material visual consistente de NDBStudio.

## Objetivo

Mostrar:

1. Flujo base (abrir DB -> query -> resultados).
2. Result modes (`Table`, `JSON`, `Graph`).
3. Panel lateral (`Schema` y `Graph`).
4. Estados de feedback (`Query en progreso...`, errores explicitos).

## Convenciones

- Terminal con ancho minimo: 140 columnas.
- Tema oscuro (default).
- Dataset recomendado para demos: el generador sintético de `examples/benchmarks.rs`.
- Evitar datos sensibles en capturas.

## Checklist de capturas

1. `01-open-loading.png`: pantalla de loading.
2. `02-editor-query.png`: query lista en editor.
3. `03-results-table.png`: resultados tabulares.
4. `04-results-json.png`: resultados JSON.
5. `05-results-graph.png`: resultados Graph mode.
6. `06-schema-panel.png`: panel Schema visible.
7. `07-graph-panel-focus.png`: panel Graph con nodo enfocado.
8. `08-error-chain.png`: error con detalles (caused by...).

Ruta sugerida:

- `docs/ndbstudio/assets/`

## GIF recomendado

`ndbstudio-flow.gif` con esta secuencia:

1. Ejecutar query.
2. Cambiar `t` entre Table/JSON/Graph.
3. Abrir `:graph` y enfocar `:graph focus name "Jon Snow"`.
4. Mostrar `:results graph`.

## Insercion en markdown

```md
![Results Table](assets/03-results-table.png)
```

```md
![NDBStudio Flow](assets/ndbstudio-flow.gif)
```

