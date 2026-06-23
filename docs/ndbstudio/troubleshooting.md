# Troubleshooting

## 1) Error de lock al abrir DB

Mensaje tipico:

`could not acquire lock ... Resource temporarily unavailable`

Causa comun:

- Otra instancia del proceso tiene abierta la misma base.

Que hacer:

1. Cierra la otra instancia de `ndbstudio` o proceso que use esa DB.
2. Reintenta abrir la base.
3. Si persiste, revisa procesos colgados y cierralos.

## 2) Resultados con columnas en `null`

Causa comun:

- Query proyecta propiedades/variables que no existen para ese patron.

Ejemplo frecuente en Synthetic Character Network:

- `ENEMY_OF` y `ALLIED_WITH` suelen estar entre `Character -> Character`, no `House -> House`.

Que hacer:

1. Valida primero con `find c.name, c.house from (c:Character) limit 10`.
2. Ajusta el patron de relaciones al label correcto.
3. Usa aliases claros en `find ... as ...`.

## 3) No puedo hacer scroll en Results

Verifica:

1. Que el foco este en `Results` (`Tab` o tecla `2`).
2. Usa `Up/Down`, `PageUp/PageDown`, `Home/End` o `j/k`.

## 4) Query tarda y parece congelado

Comportamiento esperado:

- Debe aparecer `Query en progreso...` en `Results`.

Si no ves movimiento:

1. Espera a que termine en datasets grandes.
2. Prueba con `LIMIT` pequeno para validar.
3. Verifica lock de DB (seccion 1).

## 5) Terminal queda desalineada al salir

Esto ya se mitiga con limpieza explicita de terminal en `main`.

Si alguna sesion queda rara:

```bash
reset
```

## 6) Grafo no refleja cambios recientes

Que hacer:

1. Abre panel de grafo (`x` o `:graph`).
2. Refresca snapshot (`r` o `:graph refresh`).
3. Si estas filtrando label, prueba `:graph label *`.

