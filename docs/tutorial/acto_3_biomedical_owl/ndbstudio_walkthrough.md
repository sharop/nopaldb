# Acto 3 — Walkthrough NDBStudio Web

Visualización de la ontología biomédica. NDBStudio te permite *ver* la estructura OWL
como grafo y ejecutar queries NQL directamente — incluyendo `instanceOf` semántico.

## Levantar

```bash
# Generar la DB primero (si no existe):
cd tutorials && jupyter notebook notebooks/03_biomedical_owl.ipynb
# (ejecuta el Paso 0.5 que genera test_dbs/biomedical_owl.db)

# Levantar NDBStudio Web:
make run-studio-web DB=tutorials/test_dbs/biomedical_owl.db
```

Abre `http://127.0.0.1:3737`.

---

## Paso 1 — Explorar las 7 clases OWL

Pega [`queries/01_classes.nql`](queries/01_classes.nql) en el editor y ejecuta.

En la vista **Schema** (panel derecho) verás los 7 labels:
`Disease`, `Infection`, `ViralInfection`, `BacterialInfection`, `Treatment`, `Antiviral`, `Antibiotic`.

**Nota:** el importer crea un *nodo Class* por cada clase y nodos *Individual* para cada instancia.
Todos comparten el mismo `label` (el nombre de su clase directa). Por eso el conteo de
`ViralInfection` es 4 (1 class node + 3 individuos: Covid19, Dengue, Influenza).

---

## Paso 2 — Contar por label

Pega [`queries/02_instances_check.nql`](queries/02_instances_check.nql).

| label | total | Explicación |
|-------|-------|-------------|
| ViralInfection | 4 | 1 class + Covid19, Dengue, Influenza |
| BacterialInfection | 3 | 1 class + Tuberculosis, StrepThroat |
| Antibiotic | 3 | 1 class + Amoxicillin, Rifampicin |
| Antiviral | 3 | 1 class + Remdesivir, Oseltamivir |
| Disease | 1 | sólo el class node (clase abstracta — sin individuos directos) |
| Infection | 1 | igual |
| Treatment | 1 | igual |

Las clases abstractas (Disease, Infection, Treatment) tienen `total = 1` porque nadie
declara `rdf:type :Disease` directamente en el TTL — sólo subclases concretas.

---

## Paso 3 — Visualizar la jerarquía OWL

Pega [`queries/03_subclassof_edges.nql`](queries/03_subclassof_edges.nql) y cambia a vista **Graph**.

Verás dos sub-árboles paralelos:
```
Disease ← Infection ← ViralInfection
                    └─ BacterialInfection

Treatment ← Antiviral
          └─ Antibiotic
```

Esta es la estructura *asertada* (lo que estaba escrito en el TTL). La estructura
*inferida* (ViralInfection ⊑ Disease, por transitivity) no aparece como edge — el
reasoner la deriva en memoria, no la persiste como arista adicional.

---

## Paso 4 — instanceOf semántico (el contraste clave)

Este paso muestra la diferencia entre un grafo de propiedades y un knowledge graph.

**Query naive** — pega directamente en el editor:
```sql
find n.name as nombre
from (n)
where n.label = "Disease"
```
Resultado: **1 fila** (el class node Disease, sin individuos — porque ningún individuo
tiene `label = "Disease"` explícitamente).

**Query semántica** — pega [`queries/04_instanceof_nql.nql`](queries/04_instanceof_nql.nql):
```sql
find n.label as clase, n.name as nombre
from (n)
where instanceOf(n, "Disease")
order by clase, nombre
```
Resultado: **5 filas** — Covid19, Dengue, Influenza, StrepThroat, Tuberculosis.

El predicado `instanceOf` consulta el TaxonomyIndex del grafo (reconstruido al abrir la DB).
Para cada nodo Individual, comprueba si su `label` (la clase directa del individuo) es
subclase de "Disease" en la jerarquía transitiva. No necesita que el nodo tenga
`label = "Disease"` — basta con que su clase directa esté en la cadena de herencia.

---

## Lo que NDBStudio NO muestra (pero sí el notebook)

NDBStudio visualiza la **estructura asertada**: lo que está almacenado como nodos y aristas.
Las **inferencias derivadas** (ViralInfection ⊑ Disease por CR1, Infection ⊑ InfectiousDisease
por CR2, Treatment ⊑ TreatableEntity por CR3) viven en el TaxonomyIndex en memoria.

Para ver el razonador en acción con trazas paso a paso, ejecuta el notebook `03_biomedical_owl.ipynb`.

---

## Verificación

Cubierta en el [README → gates](README.md#verificación-cruzada-gates-del-acto-3).
