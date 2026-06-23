# Cookbook Queries (Synthetic Character Network + Harbor Cay)

Consultas listas para copiar/pegar y validar comportamiento en NDBStudio.

## Synthetic Character Network: validacion base

```sql
find c.name, c.house, c.title
from (c:Character)
limit 20
```

```sql
find count(*) as total_characters
from (c:Character)
```

```sql
find c.house, count(*) as total
from (c:Character)
group by c.house
order by total desc
```

## Synthetic Character Network: relaciones

Enemigos por casa:

```sql
find c.house as house, e.house as enemy_house, count(*) as total
from (c:Character)-[:ENEMY_OF]->(e:Character)
group by c.house, e.house
order by total desc
```

Aliados por casa:

```sql
find c.house as house, a.house as allied_house, count(*) as total
from (c:Character)-[:ALLIED_WITH]->(a:Character)
group by c.house, a.house
order by total desc
```

Lealtades:

```sql
find c.name as who, l.name as loyal_to, c.house as house
from (c:Character)-[:LOYAL_TO]->(l:Character)
order by c.house, who
```

## Synthetic Character Network: usar Results en modo Graph

Esta consulta esta pensada para `:results graph`:

```sql
find c.name as source, e.type as relation, t.name as target
from (c:Character)-[e]->(t:Character)
limit 200
```

## Synthetic Character Network: foco de panel Graph por nombre

1. Abre panel Graph: `:graph`
2. Sin filtro de label: `:graph label *`
3. Enfoca directo: `:graph focus name "Jon Snow"`

## Harbor Cay: smoke tests (dataset grande)

Sanidad inicial:

```sql
find * from (n) limit 20
```

Conteo global:

```sql
find count(*) as total_nodes
from (n)
```

Sample relacional:

```sql
find s.id as source, e.type as relation, t.id as target
from (s)-[e]->(t)
limit 200
```

## Diagnostico rapido de `null`

Si ves columnas `null`:

1. Verifica primero que propiedad exista:
```sql
find c.name, c.house
from (c:Character)
limit 10
```
2. Si una relacion es `Character -> Character`, no proyectes propiedades de `House` en ese mismo patron.
3. Agrega aliases para evitar confusion (`as source`, `as relation`, `as target`).

## Analisis completo recomendado

Para un caso de analisis de red end-to-end, usa el tutorial oficial:

- `docs/tutorial/acto_1_florentine/README.md`
- Dataset generator: `nopaldb/examples/florentine_families_dataset.py`
