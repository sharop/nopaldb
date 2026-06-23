# Tutorial NQL - Análisis de Grafos Práctico

¡Bienvenido al tutorial de **NQL (NopalDB Query Language)**! Esta guía está diseñada para que **cualquier profesional de datos** —sea científico, sociólogo o desarrollador— aprenda a extraer valor de las conexiones.

---

## Tabla de Contenidos

1. [Conceptos Esenciales](#capítulo-1-conceptos-esenciales)
2. [Consultas Básicas](#capítulo-2-consultas-básicas)
3. [Descubriendo Conexiones (Relaciones)](#capítulo-3-descubriendo-conexiones)
4. [Análisis Cuantitativo](#capítulo-4-análisis-cuantitativo)
5. [Ciencia de Datos e Integración](#capítulo-5-ciencia-de-datos-e-integración)

---

## Capítulo 1: Conceptos Esenciales

Imagina tus datos como una red, no como una hoja de cálculo.

- **Nodos**: Puntos en la red. Pueden ser Personas, Empresas, Artículos Científicos.
  - Ejemplo: `(p:Persona {nombre: "Ana"})`
- **Relaciones**: Líneas que conectan los puntos. Tienen dirección.
  - Ejemplo: `Ana -> [:CONOCE] -> Beto`

### Configuración Rápida (Python)

Si usas Python, así se ve la carga de datos:

```python
import nopaldb

# Iniciar base de datos
graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Crear nodos
ana = tx.add_node("Persona", {"nombre": "Ana", "edad": 28, "ciudad": "CDMX"})
beto = tx.add_node("Persona", {"nombre": "Beto", "edad": 35, "ciudad": "GDL"})
carla = tx.add_node("Persona", {"nombre": "Carla", "edad": 22, "ciudad": "MTY"})

# Conectar (Ana conoce a Beto, Beto conoce a Carla)
tx.add_edge(ana, beto, "CONOCE")
tx.add_edge(beto, carla, "CONOCE")

tx.commit()
```

---

## Capítulo 2: Consultas Básicas

¿Cómo recuperamos esa información? La estructura es simple: `FIND` (qué quieres) `FROM` (dónde buscas).

### Tu Primera Consulta
```nql
find p.nombre, p.edad
from (p:Persona)
```

**Resultado:**
```json
{"p.nombre": "Ana", "p.edad": 28}
{"p.nombre": "Beto", "p.edad": 35}
...
```

### Filtrando Datos (Sociología/Marketing)
Digamos que buscas un segmento específico: personas jóvenes en CDMX.

```nql
find p.nombre
from (p:Persona)
where p.edad < 30 and p.ciudad = "CDMX"
```

**Explicación:**
- `(p:Persona)`: Busca en todos los nodos etiquetados como Persona.
- `where`: Aplica el filtro lógico.

---

## Capítulo 3: Descubriendo Conexiones

Aquí es donde NQL brilla. En lugar de complicados "JOINs" de SQL, simplemente dibujas la flecha.

### ¿Quién conoce a quién?
```nql
find a.nombre, b.nombre
from (a:Persona) -> [:CONOCE] -> (b:Persona)
where a.nombre = "Ana"
```
Esto lee: "Encuentra personas `b` tal que `a` (Ana) tiene una relación `CONOCE` hacia `b`".

### Análisis de Influencia (Cadena de 2 pasos)
Imagina que analizas la propagación de una idea. Si Ana influye en Beto, y Beto influye en Carla, ¿a quién influye Ana indirectamente?

```nql
find indirecto.nombre
from (ana:Persona {nombre: "Ana"}) -> [:CONOCE] -> (intermedio) -> [:CONOCE] -> (indirecto)
```

**Visualización Mental:**
`Ana -> Intermedio -> Indirecto`

---

## Capítulo 4: Análisis Cuantitativo

Para analistas que necesitan métricas agregadas.

### Contar Resultados
¿Cuántas personas viven en CDMX?
```nql
find count(*)
from (p:Persona {ciudad: "CDMX"})
```

### Estadísticas Básicas
Calcular la edad promedio de los usuarios activos.
```nql
find avg(u.edad), min(u.edad), max(u.edad)
from (u:Usuario)
where u.activo = true
```

---

## Capítulo 5: Ciencia de Datos e Integración

NopalDB está diseñado para integrarse en tu pipeline de Machine Learning o Análisis de Datos.

### Exportación Directa (DataFrames)
La forma más eficiente de mover datos para ML no es iterar fila por fila, sino exportar en bloque.

**Consulta NQL:**
```nql
find u.edad, u.intereses
from (u:Usuario)
export csv with path="social_graph.csv", header=true
```

**Python (Pandas):**
```python
import pandas as pd

# Leer el archivo generado por NopalDB
df = pd.read_csv("social_graph.csv")

# Análisis rápido
print(df.describe())

# Entrenar modelo
from sklearn.cluster import KMeans
kmeans = KMeans(n_clusters=3).fit(df[['u.edad']])
```

### Ejemplo Avanzado: Detección de Fraude
Identificar cuentas que comparten el mismo dispositivo (IP) a través de múltiples transacciones.

```nql
find cuenta1.id, cuenta2.id
from (cuenta1:Cuenta) -> [:USO_IP] -> (ip:DireccionIP) <- [:USO_IP] <- (cuenta2:Cuenta)
where cuenta1.id != cuenta2.id
```
*Patrón: Dos cuentas distintas apuntando a la misma IP.*

---

## Resumen de Comandos Clave

| Comando | Uso |
|---------|-----|
| `FIND` | Selecciona propiedades (`p.nombre`) o agregaciones (`count(*)`). |
| `FROM` | Define el patrón (`(a)->(b)`). |
| `WHERE` | Filtra (`edad > 18`). |
| `LIMIT` | Restringe resultados (útil para exploración). |
| `EXPORT` | Exporta resultados a CSV/JSON (ruta opcional). |

---

## Ejemplos de EXPORT

```nql
find p.nombre, p.edad
from (p:Persona)
order by p.edad
export csv with path="usuarios.csv", header=true
```

```nql
find p.nombre, p.edad
from (p:Persona)
limit 100
export json with jsonl=true
```

Nota:
- La sintaxis recomendada pone `export` al final.
- La forma con prefijo (`export ...` antes de `find`) no está soportada.

---

## Comentarios

```nql
// Comentario de una línea
/* Comentario en bloque */
```

## ¡Siguientes Pasos!

- **Practica:** Intenta modelar tu propio problema (ej. autores y papers, clientes y productos).
- **Referencia Técnica:** Si necesitas detalles profundos de sintaxis, revisa `NQL_REFERENCIA.md`.

**NopalDB** - *Empoderando descubrimientos.* 🚀
