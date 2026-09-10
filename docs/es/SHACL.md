# Validación SHACL

NopalDB valida un grafo contra shapes [SHACL](https://www.w3.org/TR/shacl/)
escritas en Turtle estándar, y dice dos cosas que un validador suele callar:
**qué constraint falló sobre qué valor**, y **qué partes de tus shapes no
comprobó**. Feature `shacl` (incluida en los tiers `semantic` y `full`). El
grafo nunca se modifica.

> English: [docs/SHACL.md](../SHACL.md).

## En cinco minutos

```rust
use nopaldb::Graph;

let graph = Graph::in_memory().await?;
graph.import_owl_file("recetario.ttl").await?;          // los datos, en Turtle

let (report, shapes) = graph.validate_shapes_file("recetario_shapes.ttl").await?;

assert!(shapes.ignored.is_empty());   // todo término sh:* del archivo se comprueba
if !report.conforms {
    for v in &report.violations {
        println!("{} · {} · {:?} · {:?}: {}", v.shape_name, v.constraint, v.path, v.value, v.message);
    }
}
```

Python:

```python
result = graph.validate_shapes(open("recetario_shapes.ttl").read())
result["conforms"], result["ignored"]
for v in result["violations"]:
    print(v["shape"], v["constraint"], v["path"], v["value"], v["message"])
```

Un archivo de shapes, para un recetario ficticio:

```turtle
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .

:RecetaShape a sh:NodeShape ;
    sh:name "Receta bien descrita" ;
    sh:targetClass :Receta ;
    sh:property [ sh:path :nombre ;     sh:minCount 1 ; sh:maxCount 1 ; sh:datatype xsd:string ] ,
                [ sh:path :tiempoMin ;  sh:datatype xsd:integer ; sh:minInclusive 1 ] ,
                [ sh:path :dificultad ; sh:in ( "fácil" "media" "difícil" ) ] ,
                [ sh:path :usa ;        sh:minCount 2 ; sh:class :Ingrediente ] .
```

## Qué se comprueba

| Término | Significado aquí |
|---|---|
| `sh:NodeShape`, `sh:name` | Una shape; el nombre aparece en cada violación. Todo sujeto con `sh:targetClass` o `sh:targetNode` también es shape. Una shape sin target no valida nada por sí sola: existe para que `sh:node` o un combinador la refieran (como dice SHACL; solo la API programática conserva "sin targets = todos los nodos"). |
| `sh:targetClass :C` | Focus nodes = las instancias de `C` **y de sus subclases**, por la jerarquía de clases del grafo, por cualquiera de sus tipos declarados; `C` puede ser label, `prefijo:Local` o IRI completo (la misma resolución que `instanceOf` en NQL). El nodo clase nunca lo es. Sin jerarquía (grafo a mano) son los individuos con label `C`. |
| `sh:targetNode <iri>` | El nodo cuya propiedad `iri` es ese IRI. Un IRI que no existe se reporta en `report.notes`, no se ignora. |
| `sh:property [ sh:path :p ; … ]` | Constraints sobre los valores de `p`: la propiedad `p` del nodo (una lista cuenta un valor por elemento) **y** los destinos de sus aristas de tipo `p`. Turtle no dice en cuál de las dos aterrizó un predicado en NopalDB, así que el path son ambas. |
| `sh:path ( :a :b … )` | Una secuencia: los valores de un salto son los focus del siguiente, solo a través de nodos (un literal termina su rama); el resultado es la unión de todas las ramas sin duplicados. Las violaciones muestran el path como `a/b`. |
| `sh:minCount`, `sh:maxCount` | Número de valores. |
| `sh:datatype xsd:*` | Con la tabla del propio importer: la familia integer → entero, `decimal`/`double`/`float` → flotante, `boolean`, y todo lo demás (`xsd:string`, `xsd:date`, …) → texto, con aviso cuando un datatype se comprueba como texto. Un valor-nodo nunca cumple un datatype. |
| `sh:minInclusive` … `sh:maxExclusive` | Rangos numéricos; un no-número no los cumple. |
| `sh:minLength`, `sh:maxLength` | Sobre cadenas; otros valores no se juzgan. |
| `sh:pattern` | Una regex (sintaxis del crate `regex` de Rust) sobre cadenas, compilada una vez al cargar la shape; un no-string no coincide. Un patrón inválido es error de carga con el nombre de la shape y el patrón. |
| `sh:in ( … )` | El valor es uno de los literales de la lista. Los IRIs de la lista se reportan y se saltan. |
| `sh:hasValue` | Algún valor es igual al literal, o algún valor-nodo tiene ese IRI. |
| `sh:class :C` | Cada valor-nodo es instancia de `C`: directa o por la jerarquía de clases cuando el grafo la tiene (`C` por label, `prefijo:Local` o IRI), por label si no. Un literal nunca lo es. |
| `sh:nodeKind sh:IRI` / `sh:BlankNode` / `sh:Literal` y combinaciones | Un valor-nodo es IRI (o blank node cuando su `iri` empieza por `_:`); un valor de propiedad es literal. |

| `sh:and ( s1 s2 … )`, `sh:or ( … )`, `sh:xone ( … )`, `sh:not s` | Cada valor debe conformar con todas / al menos una / exactamente una / ninguna de las shapes miembro (anónimas `[ … ]` o con nombre). La violación lleva las ramas fallidas en `nested`, así un `or` dice por qué falló cada rama. |
| `sh:node :OtraShape` | Cada valor debe conformar con otra shape del mismo documento (por IRI o nombre), property shapes incluidas: el validador carga los paths del propio valor. La violación anida las de la otra shape. Una referencia inexistente es violación. |
| `sh:severity sh:Violation` / `sh:Warning` / `sh:Info`, `sh:message "…"` | En una shape o en una property shape. `conforms` solo mira `Violation`; el mensaje del autor sustituye al generado. |
| `sh:deactivated true` | La shape se carga y no valida nada. |

Las constraints de nodo (`sh:class`, `sh:nodeKind`, los combinadores escritos
directamente en la shape) se evalúan sobre el propio focus node con el mismo
código que las property shapes: hay un solo evaluador, `evaluate_shape`.

## Qué no se comprueba, y cómo lo sabes

Todo lo demás del vocabulario `sh:` se **reporta, nunca se ignora en
silencio**: cada término no soportado o mal formado es una línea en
`ShapesReport.ignored` con la razón, y el resto de la shape se carga igual. Hoy
eso cubre las otras formas de path (`sh:inversePath`, `sh:alternativePath`,
`sh:zeroOrMorePath`, `sh:oneOrMorePath`, `sh:zeroOrOnePath`: cada una cambia
lo que significa la cardinalidad sobre un cierre y merece su propio diseño con
la taxonomía), `sh:closed`,
`sh:qualifiedValueShape`, comparaciones entre propiedades (`sh:equals`,
`sh:lessThan`, …), `sh:languageIn`/`sh:uniqueLang` (el import no conserva lang
tags), `sh:flags`, `sh:targetSubjectsOf`/`sh:targetObjectsOf` y SHACL-SPARQL.
Una property shape sin `sh:path`, o con una de esas formas de path, se
descarta con una línea que lo dice.

`ShapesReport` trae además los conteos `shapes`, `property_shapes`,
`constraints` y los `warnings` del parser Turtle (falta `@prefix :`, IRI
relativo sin `@base`). Un Turtle malformado es error con línea y columna, y no
se valida nada.

## El reporte

`ValidationReport { conforms, violations, notes }`. `conforms` es `true` cuando
ninguna violación tiene severidad `Violation`. Cada `ConstraintViolation`:

| Campo | |
|---|---|
| `focus_node` | El nodo que falló (`NodeId`). |
| `shape_name`, `shape_id` | Qué shape. |
| `constraint` | El componente SHACL, p. ej. `sh:MinCountConstraintComponent`. |
| `path` | El predicado de la property shape; `None` en constraints de nodo. |
| `value` | El valor culpable: el literal, o el `iri` del nodo (su id si no tiene). `None` en cardinalidad. |
| `message`, `severity` | Texto legible (el `sh:message` del autor cuando lo hay) y `Violation` / `Warning` / `Info`. |
| `nested` | En combinadores y `sh:node`: las violaciones de las ramas / de la shape referida. |

`notes` lista lo que el validador no pudo hacer en absoluto (un `sh:targetNode`
que no existe). Ambos tipos derivan `Serialize`.

## Shapes desde Rust

La API programática no cambia y comparte el evaluador:

```rust
use nopaldb::shacl::{ConstraintType, PathSpec, PropertyShape, ShaclValidator, Shape, Target};

let shape = Shape::new("PersonShape")
    .with_target(Target::Class("Person".into()))
    .with_property_shape(PropertyShape::new(PathSpec::Property("age".into()), vec![ConstraintType::MinCount(1)]));
let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await?;
```

`PathSpec::Property` lee solo la propiedad, `PathSpec::Edge` solo las aristas,
`PathSpec::Predicate` ambas (como carga `sh:path`).

### Modo de target

`sh:targetClass` sigue la taxonomía por defecto. Si dependes del comportamiento
anterior a 0.5.15 (individuos cuyo `label` es exactamente el texto, sin
subclases ni IRIs):

```rust
let validator = ShaclValidator::from_shapes(shapes).with_target_mode(TargetMode::ExactLabel);
```

## Feature

`shacl = ["regex", "owl-import"]`: las shapes son Turtle, así que la feature trae
el parser del puente, y `sh:class` se resuelve por la taxonomía, que vive tras
`owl-import` (y su `reasoner`). `cargo build --features shacl` o cualquier tier
desde `semantic`.
