# Acto 3 — Biomedical OWL + ELReasoner

**Tiempo estimado:** 40 min
**Dataset:** `tutorials/data/biomedical.ttl` — ontología biomédica compacta (7 clases, 9 instancias).
**DB generada por:** `tutorials/notebooks/03_biomedical_owl.ipynb` (Paso 0.5) o el ejemplo Rust.

---

## ¿Por qué este acto?

Los actos 1 y 2 trabajan con **grafos de propiedades**: nodos con labels y propiedades, edges tipados.
Eso basta para modelar *quién se relaciona con quién*. Pero no basta para responder:

> "¿Es Covid19 una enfermedad?" — sin saber que ViralInfection ⊑ Infection ⊑ Disease.

Aquí entra el territorio de los **knowledge graphs semánticos**: el grafo no sólo almacena datos,
sino que *razona* sobre ellos. Una ontología OWL declara axiomas y el reasoner deriva
conocimiento nuevo que no estaba escrito explícitamente en los datos.

---

## Conceptos clave antes de empezar

### ¿Qué es OWL y para qué sirve?

**OWL** (Web Ontology Language) es un lenguaje para declarar *vocabularios estructurados* con semántica
formal. En lugar de decir "Covid19 tiene label ViralInfection" (afirmación de dato),
dices "Covid19 es instancia de ViralInfection" (afirmación de clase) y además declaras que
"ViralInfection es subclase de Infection que es subclase de Disease" (axioma de jerarquía).

La diferencia práctica: con una base de datos de propiedades, una query `n.label = "Disease"`
devuelve sólo los nodos donde literal-mente está escrito "Disease". Con OWL + reasoner,
`instanceOf(n, "Disease")` devuelve **todos los individuos** cuya clase, directa o por herencia,
es subclase de Disease — aunque sus labels digan "ViralInfection" o "BacterialInfection".

**Usos reales:**
- **SNOMED CT** — 400,000+ conceptos médicos con jerarquía que los sistemas de historia clínica
  usan para responder "¿cuáles pacientes tienen una enfermedad respiratoria?" con un solo código.
- **Gene Ontology (GO)** — anotación de funciones de genes. "¿Qué genes están relacionados con
  el metabolismo lipídico?" — la respuesta incluye subontologías 3 niveles abajo.
- **CHEBI** — ontología de entidades químicas biológicamente interesantes. Usada en drug discovery.

Los tres usan el perfil **EL** o **EL++** de OWL — el mismo que implementa NopalDB.

---

### ¿Qué es la Lógica Descriptiva EL?

**EL** (del inglés *Existential Language*) es un *fragmento* de OWL que equilibra expresividad y
eficiencia computacional. Full OWL 2 DL es EXPTIME-completo (exponencial en el peor caso).
EL es **PTIME** — tiempo polinomial. Para ontologías con millones de conceptos como SNOMED,
esa diferencia es la razón de que el razonador termine en segundos y no en horas.

EL permite tres operaciones sobre conceptos (clases):

| Operación | Notación | Significado |
|-----------|----------|-------------|
| Nombre de clase | `Disease` | Una clase atómica |
| Intersección | `A ⊓ B` | "cosa que es A Y B" |
| Restricción existencial | `∃r.A` | "cosa que tiene relación r con algún A" |

Con estos tres bloques básicos se construyen todos los axiomas relevantes para ontologías biomédicas.
No se permite negación, disyunción, ni cuantificación universal — eso lo haría NP-hard o peor.

**¿Por qué NopalDB implementa EL y no full OWL?**
Porque el sweet spot de los knowledge graphs en producción es el razonamiento tractable: taxonomías
profundas, clasificación de instancias, herencia transitiva. Full OWL añade complejidad que muy pocos
sistemas realmente necesitan.

---

### Las Reglas CR — cómo razona el ELReasoner

El ELReasoner de NopalDB implementa tres **Reglas de Completación** (Completion Rules) del algoritmo
estándar de Baader et al. (2005). Cada regla dispara cuando cierta precondición es verdadera
y produce una nueva inferencia:

---

#### CR1 — Transitividad de subclases

**Regla formal:**
```
Si:  A ⊑ B   y   B ⊑ C
→    A ⊑ C        (derivado)
```

**En palabras:** si A es subclase de B y B es subclase de C, entonces A es subclase de C.
Es la regla más intuitiva — herencia transitiva.

**Traza en el dataset biomédico:**

```
TTL declara:
  ViralInfection  rdfs:subClassOf  Infection     → ViralInfection ⊑ Infection
  Infection       rdfs:subClassOf  Disease       → Infection ⊑ Disease

CR1 dispara (paso 1):
  ViralInfection ⊑ Disease  ← INFERIDO

CR1 dispara (encadenado):
  BacterialInfection ⊑ Disease  ← INFERIDO
  Antiviral ⊑ Treatment  ← INFERIDO  (ya estaba declarado)
  Antibiotic ⊑ Treatment  ← INFERIDO  (ya estaba declarado)
```

**Resultado observable:** `instanceOf(Covid19, "Disease")` → `true`, aunque Covid19 tiene
`rdf:type ViralInfection` en el TTL y nunca aparece la palabra "Disease" junto a Covid19.

---

#### CR2 — Conjunción (intersección de clases)

**Regla formal:**
```
Si:  A ⊑ B   y   A ⊑ C   y   existe axioma B ⊓ C ⊑ D
→    A ⊑ D        (derivado)
```

**En palabras:** si sabemos que X pertenece a la categoría A y a la categoría B, y hay una
regla que dice "cualquier cosa que sea A-y-B es también D", entonces X es D.

**Traza en el dataset:**

```
Axioma:
  Disease ⊓ Infection ⊑ InfectiousDisease

Hechos conocidos sobre Infection:
  Infection ⊑ Disease   (por CR1, transitivo)
  Infection ⊑ Infection (trivialmente)

CR2 dispara:
  Infection ⊑ InfectiousDisease  ← INFERIDO
  ViralInfection ⊑ InfectiousDisease  ← INFERIDO (porque ViralInf ⊑ Infection ⊑ Disease)
  BacterialInfection ⊑ InfectiousDisease  ← INFERIDO
```

**Cuándo se usa en la práctica:** en SNOMED CT, "Disorder of musculoskeletal system" se define
como `Disorder ⊓ FindingSiteOf(Musculoskeletal)` — exactamente CR2. Sin él, no puedes
clasificar dinámicamente condiciones nuevas en categorías compuestas.

---

#### CR3 — Restricción existencial

**Regla formal:**
```
Si:  A ⊑ ∃r.B   y   ∃r.B ⊑ C
→    A ⊑ C        (derivado)
```

**En palabras:** si algo de tipo A tiene una relación `r` que apunta a algo de tipo B,
y cualquier cosa con esa relación-apuntando-a-B se clasifica como C, entonces A es C.

**Traza en el dataset:**

```
Axioma 1: Treatment ⊑ ∃treats.Disease
  "Todo tratamiento trata alguna enfermedad"

Axioma 2: ∃treats.Disease ⊑ TreatableEntity
  "Cualquier cosa que trate alguna enfermedad es una TreatableEntity"

CR3 dispara:
  Treatment ⊑ TreatableEntity  ← INFERIDO
```

**Por qué CR3 importa:** permite modelar relaciones funcionales sin enumerar todos los casos.
En farmacología: "∃hasTarget.KinaseProtein ⊑ KinaseInhibitor" clasifica
automáticamente cualquier droga que interactúe con una kinasa como inhibidora de kinasas,
sin tener que enumerar cada droga individualmente.

---

## El contraste clave: grafo de propiedades vs knowledge graph

Esta es la diferencia más importante del Acto 3, vale detenerse:

```sql
-- Query A: búsqueda naive (grafo de propiedades)
-- Encuentra: 1 nodo (sólo el class node "Disease")
find n.name as nombre
from (n)
where n.label = "Disease"
```

```sql
-- Query B: búsqueda semántica (knowledge graph)
-- Encuentra: 5 individuos (Covid19, Dengue, Influenza, Tuberculosis, StrepThroat)
find n.name as nombre, n.label as tipo_directo
from (n)
where instanceOf(n, "Disease")
order by tipo_directo, nombre
```

**¿Por qué la diferencia?**

Query A filtra por la propiedad `label` del nodo. Covid19 tiene `label = "ViralInfection"`,
no "Disease". La query no lo encuentra.

Query B usa el TaxonomyIndex del grafo que almacena la cadena de herencia reconstruida:

```
Covid19.label = "ViralInfection"
             ↓ TaxonomyIndex.is_subclass_of("ViralInfection", Disease_id)?
ViralInfection ⊑ Infection ⊑ Disease  → SÍ
             ↓
Covid19 es instancia de Disease  ✓
```

El reasoner hace este trabajo por ti, para cualquier profundidad de jerarquía y sin que
tengas que conocer la estructura de la ontología de antemano.

---

## Jerarquía del dataset

```
Thing
├── Disease
│   └── Infection
│       ├── ViralInfection
│       │   ├── Covid19
│       │   ├── Dengue
│       │   └── Influenza
│       └── BacterialInfection
│           ├── Tuberculosis
│           └── StrepThroat
└── Treatment
    ├── Antiviral
    │   ├── Remdesivir
    │   └── Oseltamivir
    └── Antibiotic
        ├── Amoxicillin
        └── Rifampicin

(clases en MAYÚSCULA, individuos en minúscula/CamelCase)
```

**Clases abstractas** (sin individuos directos): Disease, Infection, Treatment.
Ningún nodo tiene `label = "Disease"` directamente — todos los "enfermos" son ViralInfection
o BacterialInfection. Eso es exactamente el caso donde el reasoner brilla.

---

## Setup

```bash
# Opción A — Notebook Python (genera la DB + demuestra el reasoner):
cd tutorials && jupyter notebook notebooks/03_biomedical_owl.ipynb

# Opción B — Ejemplo Rust (mismo resultado):
cargo run --example tutorial_acto_3_biomedical \
  --no-default-features \
  --features storage-sled,reasoner,owl-import,algorithms,analytics,hypergraph,ml \
  -- test_dbs/biomedical_owl.db tutorials/data/biomedical.ttl
```

La DB generada queda en `tutorials/test_dbs/biomedical_owl.db`.

---

## Paso 1 — Importar el TTL al grafo

`Graph::import_turtle()` parsea el Turtle en tres pasadas:

| Pasada | Triple procesado | Qué crea en el grafo |
|--------|-----------------|----------------------|
| Pass 1 | `:X rdf:type owl:Class` | Nodo Class con `label = X` |
| Pass 2 | `:X rdfs:subClassOf :Y` | Edge `X → Y` de tipo `"subClassOf"` + registro en TaxonomyIndex |
| Pass 3 | `:x rdf:type :SomeClass` | Nodo Individual con `label = SomeClass`, propiedades del sujeto |

**Gate del import:**
```
classes_added:        7
subclass_edges_added: 5
instances_added:      9
triples_skipped:      18    (data properties: :name, :agent, :route — no son ontológicas)
```

Al abrir la DB en una sesión nueva (`Graph::open()`), el método `rebuild_taxonomy_from_graph()`
recorre los nodos Class y las aristas `subClassOf` y reconstruye la TaxonomyIndex en memoria.
Eso es lo que permite que `instanceOf` NQL funcione en sesiones nuevas sin
haber llamado a `import_turtle` de nuevo.

---

## Paso 2 — ELReasoner standalone (notebook Python)

El notebook demuestra el reasoner **completamente en memoria**, sin DB. Útil para entender
las reglas de forma aislada, sin la complejidad del grafo persistido.

```python
reasoner = nopaldb.ELReasoner()
reasoner.register_class(ids["Disease"], "Disease")
# ... registrar los 9 conceptos

# CR1 — Transitividad
reasoner.assert_subclass(ids["ViralInfection"], ids["Infection"])
reasoner.assert_subclass(ids["Infection"],      ids["Disease"])

# CR2 — Conjunción: Disease ⊓ Infection ⊑ InfectiousDisease
reasoner.assert_conjunction(ids["Disease"], ids["Infection"], ids["InfectiousDisease"])

# CR3 — Existencial: Treatment ⊑ ∃treats.Disease  y  ∃treats.Disease ⊑ TreatableEntity
reasoner.assert_existential(ids["Treatment"], "treats", ids["Disease"])
reasoner.assert_existential_domain("treats", ids["Disease"], ids["TreatableEntity"])

reasoner.classify_all()  # dispara todas las reglas hasta punto fijo
```

`classify_all()` itera hasta alcanzar el **punto fijo**: el estado donde aplicar todas las
reglas otra vez no produce ninguna inferencia nueva. Para EL, se garantiza que este proceso
termina en tiempo polinomial.

**Inferencias que el reasoner deriva (no estaban en el TTL):**

| Inferencia | Regla | ¿Estaba declarada? |
|------------|-------|-------------------|
| `ViralInfection ⊑ Disease` | CR1 | No |
| `BacterialInfection ⊑ Disease` | CR1 | No |
| `Infection ⊑ InfectiousDisease` | CR2 | No |
| `ViralInfection ⊑ InfectiousDisease` | CR2 | No |
| `BacterialInfection ⊑ InfectiousDisease` | CR2 | No |
| `Treatment ⊑ TreatableEntity` | CR3 | No |

---

## Paso 3 — NQL con instanceOf (grafo persistido)

<!-- source: queries/04_instanceof_nql.nql -->
```sql
find n.label as clase, n.name as nombre
from (n)
where instanceOf(n, "Disease")
order by clase, nombre
```

Resultado esperado (5 filas):
```
BacterialInfection  StrepThroat
BacterialInfection  Tuberculosis
ViralInfection      Covid19
ViralInfection      Dengue
ViralInfection      Influenza
```

**Nota:** ninguno de estos nodos tiene `label = "Disease"`. El predicado `instanceOf`
navega la TaxonomyIndex transitivamente — la misma lógica que CR1 en el reasoner.

Para la query de contraste (que devuelve 0 individuos):
<!-- source: queries/05_naive_vs_semantic.nql -->
```sql
find n.name from (n) where n.label = "Disease"
-- Resultado: sólo el class node Disease (sin individuos)
```

---

## Verificación cruzada (gates del Acto 3)

**Import (Rust o notebook):**
- `import_turtle` reporta 7 classes / 5 subclass_edges / 9 instances ✓

**ELReasoner (notebook Python):**

| Aserción | Regla | Resultado |
|----------|-------|-----------|
| `is_subclass_of(ViralInfection, Disease)` | CR1 (transitivo) | `True` |
| `is_subclass_of(Infection, InfectiousDisease)` | CR2 | `True` |
| `is_subclass_of(Treatment, TreatableEntity)` | CR3 | `True` |
| `is_subclass_of(Antibiotic, Disease)` | negativo | `False` |

**NQL + TaxonomyIndex:**
- `instanceOf(n, "Disease")` → 5 individuos ✓
- `n.label = "Disease"` → 0 individuos (clase abstracta) ✓

## NDBStudio Web

Ver [`ndbstudio_walkthrough.md`](ndbstudio_walkthrough.md) para la visualización del árbol OWL.

**TL;DR:** NDBStudio muestra la estructura *asertada* (lo que está en el TTL). Las *inferencias*
(lo que derivó el reasoner) son visibles sólo via NQL con `instanceOf` o via el notebook.

---

## Para profundizar

| Recurso | Tipo | Por qué |
|---------|------|---------|
| Baader et al., ["A Description Logic Primer"](https://arxiv.org/abs/1201.4089) | Paper libre (arxiv) | La referencia más accesible sobre DL, EL, y las reglas CR. 30 páginas. |
| ["The Description Logic Handbook"](https://www.cambridge.org/core/books/description-logic-handbook/B3AEB1D7A95A6D7571D41E3E1F25B5AE), Cambridge | Libro | Referencia completa. Capítulo 6 cubre EL. |
| [W3C OWL 2 EL Profile](https://www.w3.org/TR/owl2-profiles/#OWL_2_EL) | Estándar | Especificación formal del perfil EL, con ejemplos. |
| [SNOMED CT Technical Reference Guide](https://confluence.ihtsdotools.org/display/DOCTRG) | Guía técnica | Cómo la ontología clínica más grande del mundo usa EL++ en producción. |
| [Feature Tiers](../../FEATURE_TIERS.md) | Guia NopalDB | Features necesarias para compilar reasoner, OWL, SHACL y embeddings. |
| [Embeddings](../../EMBEDDINGS.md) | Guia NopalDB | Busqueda semantica y vectores dentro de NopalDB. |

**Diferencia entre el reasoner (Acto 3) y SHACL (hands-on):**
- *Reasoner*: "¿qué se puede inferir?" → deriva conocimiento nuevo.
- *SHACL*: "¿qué está mal en los datos?" → detecta violaciones de constraints.
Son complementarios: el reasoner enriquece los datos, SHACL valida que cumplan las reglas de negocio.

---

## Siguiente

[Acto 4 — Synthetic Fraud (Final Boss)](../acto_4_synthetic_fraud/README.md): combina TODAS
las features (MVCC time-travel, community detection, embeddings, reasoner) sobre un dataset
sintético con ~5K nodos.
