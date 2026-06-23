# NDBStudio Web Quickstart

## 1) Qué es

`NDBStudio Web` es la evolución local-first de la workbench de `ndbstudio`.

Comparte la misma lógica Rust del TUI, pero la expone en navegador con:

- editor NQL
- tabs de query persistentes por sesión/DB
- workspace con tabs (`Results`, `Graph`, `Timeline`, `Run Detail`)
- captura de conocimiento con `Findings` + `Project notebook`
- sidebar fijo derecho para `Schema + Stats`
- graph visual (`Inspect | Visual`)
- graph source switch (`Dataset | Result Focus`)
- timeline / session browser persistente
- DAG e impacto visual
- apertura de DB desde la propia UI

## 2) Ejecutar desde el repo

Desde la raíz del workspace:

```bash
cargo run -p ndbstudio --features web -- --web
```

Para abrir directo un proyecto/DB existente:

```bash
cargo run -p ndbstudio --features web -- --web ./ruta/a/tu.db
```

Ejemplo:

```bash
cargo run -p ndbstudio --features web -- --web ../NDB-QA/test_dbs/florentine_families.db
```

Con `make`:

```bash
make run-studio-web DB=../NDB-QA/test_dbs/florentine_families.db
```

Puerto/bind custom:

```bash
cargo run -p ndbstudio --features web -- --web ./ruta/a/tu.db --bind 127.0.0.1:4040
```

o:

```bash
make run-studio-web DB=./ruta/a/tu.db BIND=127.0.0.1:4040
```

## 3) Abrir en navegador

Por default:

```text
http://127.0.0.1:3737
```

Si usaste `--bind`, abre esa dirección.

## 4) Launcher inicial, abrir DB y restaurar sesión

Si arrancas con `--web` sin ruta:

- aparece el **Project Launcher**
- puedes abrir un proyecto reciente
- puedes crear un proyecto nuevo
- puedes escribir una ruta avanzada y abrir o crear ahí

Si arrancas con `--web <ruta>` y la ruta no existe:

- el servidor levanta igual
- el launcher entra en estado `pending_db_path`
- puedes decidir si crear el proyecto ahí o corregir la ruta

Además, la web permite:

- abrir otra DB desde el menú `Projects`
- volver al launcher desde el menú `Projects`
- cerrar el proyecto activo sin cerrar la app
- reabrir proyectos recientes desde el menú `Projects`
- restaurar historial de queries/timeline por DB
- conservar preferencias visuales en el navegador
- guardar notas del proyecto y findings ligados a queries/runs

Persistencia actual:

- **backend Rust**: sesiones e historial bajo `~/.ndstudio/`
- **backend Rust**: tabs, query text y último resultado por DB
- **browser**: preferencias ligeras (`tab`, filtros, modo, sidebar, etc.)

## 5) Flujo recomendado de prueba

1. Corre una query simple:

```sql
find n from (n) limit 25
```

2. Corre otra en `Explain` o `Profile`.
3. Crea una segunda tab y guarda otra query distinta.
3. Revisa `Schema`.
4. Abre el menú `Projects` en la top bar y prueba:
   - `Back to launcher`
   - `Close project`
   - `Switch project`
   - `Create project`
5. Ve a `Graph`:
   - prueba `Inspect`
   - cambia a `Visual`
   - alterna `Dataset | Result Focus`
   - ejecuta una query que devuelva nodos o un patrón con aliases (`x.name as bridge`, etc.) para ver auto-focus del grafo
   - selecciona un nodo y revisa el cuadro de detalle con tipo, id y propiedades
   - reenfoca nodos
6. Ve a `Timeline`:
   - filtra por texto o modo
   - usa `Pin`
   - usa `Load`
   - usa `Rerun`
7. En `Run Detail`, revisa:
   - `DAG`
   - `Impact`
8. En `Findings`:
   - usa `Save finding` desde `Results`
   - abre la tab `Findings`
   - prueba `Load query`, `Edit` y `Delete`
   - guarda notas libres en `Project notebook`

9. Recarga el navegador y confirma que:
   - el timeline sigue presente
   - las tabs de query siguen presentes
   - la query activa se conserva
   - el último resultado por tab se conserva
   - los findings siguen presentes
   - las notas del proyecto se mantienen
   - el sidebar mantiene su estado
   - el tab activo se restaura

## 6) Packaging para QA

El binario empaquetado de `ndbstudio` ahora debe incluir la feature `web`.

Build local:

```bash
make build-studio
```

Paquete tarball:

```bash
make package-studio
```

Tarball explícito para Web:

```bash
make package-studio-web
```

Suite QA completa:

```bash
make package-qa
```

Suite QA enfocada en Web:

```bash
make package-qa-web DB=../NDB-QA/test_dbs/florentine_families.db
```

App bundle macOS para QA:

```bash
make package-studio-web-app
```

App bundle macOS con smoke test previo:

```bash
make package-qa-web-app DB=../NDB-QA/test_dbs/florentine_families.db
```

Firma y notarización (macOS):

```bash
export CODESIGN_IDENTITY="Developer ID Application: Tu Nombre (TEAMID)"
export NOTARY_PROFILE="ndbstudio-notary"
make notarize-studio-web-app
```

Smoke test local del backend web:

```bash
make smoke-studio-web DB=../NDB-QA/test_dbs/florentine_families.db
```

## 7) Notas operativas

- `NDBStudio Web` es **local-first** en esta fase.
- No requiere Node/Vite para correr el scaffold actual.
- El frontend se sirve directamente desde el binario Rust.
- La workbench usa una paleta minimalista nueva y un icono web ligero de NopalDB.
- `Graph` no reemplaza el dataset completo al correr una query: cambia a `Result Focus` cuando el resultado es interpretable como nodos, pares `source/target`, o cuando la query describe un patrón de grafo y solo proyecta propiedades/aliases.
- En macOS puedes generar `NDBStudioWeb.app`, que abre selector de DB y lanza el navegador.
- Para distribuir fuera de tu máquina, Apple recomienda firmar con `Developer ID Application` y notarizar con `notarytool`.
- Si el binario fue compilado sin `web`, `--web` fallará con un mensaje explícito.

### Notarización rápida

Guarda primero las credenciales del perfil una sola vez:

```bash
xcrun notarytool store-credentials "ndbstudio-notary" \
  --apple-id "tu-apple-id@example.com" \
  --team-id "TEAMID" \
  --password "app-specific-password"
```

Luego:

```bash
export CODESIGN_IDENTITY="Developer ID Application: Tu Nombre (TEAMID)"
export NOTARY_PROFILE="ndbstudio-notary"
make notarize-studio-web-app
```

## 8) Troubleshooting rápido

### El puerto ya está ocupado

Usa otro bind:

```bash
make run-studio-web DB=./ruta/a/tu.db BIND=127.0.0.1:4040
```

### El binario no acepta `--web`

Recompila con la feature:

```bash
cargo build -p ndbstudio --release --features web
```

### No carga el frontend

Verifica:

1. que el proceso siga corriendo
2. que la URL sea correcta
3. que el puerto no esté bloqueado por otra app

### El launcher se queda visible o los botones parecen no responder

Haz un hard refresh del navegador (`Cmd+Shift+R` en macOS).  
El backend ahora sirve `index.html`, `app.js` y `styles.css` con `Cache-Control: no-store`, pero si el navegador ya tenía una versión vieja abierta, puede quedarse con assets desalineados.

Luego verifica:

1. si arrancaste con `--web ./ruta/existente.db`, el workbench debe abrir directo
2. si arrancaste con `--web` sin ruta, el launcher debe mostrar proyectos recientes y permitir crear/abrir
3. si una DB ya está abierta en otra instancia, puede fallar por lock de storage
4. si un proyecto estaba abierto, usa el menú `Projects` para volver al launcher o cerrarlo explícitamente
