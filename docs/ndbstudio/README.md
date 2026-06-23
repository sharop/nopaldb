# NDBStudio Docs

Guia practica para usar y mantener `ndbstudio` (TUI de NopalDB).

## Inicio rapido

- [Quickstart](quickstart.md)
- [NDBStudio Web Quickstart](web_quickstart.md)
- [Keybindings](keybindings.md)
- [Commands](commands.md)
- [Cookbook Queries](cookbook.md)

## UI

- [Panels](panels.md)
- [Troubleshooting](troubleshooting.md)

## Contributors

- [Architecture](architecture.md)

## English

- [English Docs Index](en/README.md)

## Estrategia de documentacion

Para mantener docs utiles y actualizadas:

1. Documentar por tareas (no por archivo): "quiero hacer X".
2. Mantener paridad con UX: cualquier cambio en teclas/comandos debe incluir docs.
3. Priorizar errores reales: lock de DB, queries con `null`, salida de terminal.
4. Incluir ejemplos reales: datasets Synthetic Character Network y Harbor Cay.
5. Mantener docs cortas y enlazadas entre si.

## Alcance actual

Incluye:

- Navegacion por paneles (Editor, Results, Schema, Graph).
- Modos de resultados (`Table`, `JSON`, `Graph`).
- Comandos `:graph` (label/focus/refresh), `:results`, y utilidades de editor.
- Estado de ejecucion y tiempos de query.
- NDBStudio Web local-first con timeline, graph visual, DAG e impacto.
