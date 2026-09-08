# SHACL validation

NopalDB validates a graph against [SHACL](https://www.w3.org/TR/shacl/) shapes
written in standard Turtle, and tells you two things a validator usually keeps
to itself: **which constraint failed on which value**, and **which parts of your
shapes it did not check**. Feature `shacl` (included in the `semantic` and
`full` tiers). The graph is never modified.

> Español: [docs/es/SHACL.md](es/SHACL.md).

## Five minutes

```rust
use nopaldb::Graph;

let graph = Graph::in_memory().await?;
graph.import_owl_file("recetario.ttl").await?;          // the data, as Turtle

let (report, shapes) = graph.validate_shapes_file("recetario_shapes.ttl").await?;

assert!(shapes.ignored.is_empty());   // every sh:* term in the file is checked
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

A shapes file, for a fictional cookbook:

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

## What is checked

| Term | Meaning here |
|---|---|
| `sh:NodeShape`, `sh:name` | A shape; the name appears in every violation. Any subject with `sh:targetClass` or `sh:targetNode` is a shape too. |
| `sh:targetClass :C` | Focus nodes = the individuals labelled `C` (the class node itself is not one). Subclasses and IRIs: [#99](https://github.com/sharop/nopaldb/issues/99). |
| `sh:targetNode <iri>` | The node whose `iri` property is that IRI. An IRI that matches nothing is reported in `report.notes`, not ignored. |
| `sh:property [ sh:path :p ; … ]` | Constraints on the values of `p`: the property `p` of the node (a list counts one value per element) **and** the targets of its edges of type `p`. Turtle does not say which one a predicate became in NopalDB, so both are the path. |
| `sh:minCount`, `sh:maxCount` | Number of values. |
| `sh:datatype xsd:*` | Read with the importer's own table: the integer family → integer, `decimal`/`double`/`float` → float, `boolean`, everything else (`xsd:string`, `xsd:date`, …) → text, with a warning when a datatype is checked as text. A node value never satisfies a datatype. |
| `sh:minInclusive` … `sh:maxExclusive` | Numeric ranges; a non-number does not satisfy them. |
| `sh:minLength`, `sh:maxLength` | On strings; other values are not judged. |
| `sh:pattern` | A regex (Rust `regex` syntax) on strings; a non-string does not match. An invalid pattern is a `Warning` per value (an error at load time is [#100](https://github.com/sharop/nopaldb/issues/100)). |
| `sh:in ( … )` | The value is one of the listed literals. IRIs in the list are reported and skipped. |
| `sh:hasValue` | Some value equals the literal, or some node value has that IRI. |
| `sh:class :C` | Every node value is an instance of `C`: directly or through the class hierarchy when the graph has one (`C` by label, `prefix:Local` or IRI), by label otherwise. A literal never is. |
| `sh:nodeKind sh:IRI` / `sh:BlankNode` / `sh:Literal` and combinations | A node value is an IRI (or a blank node when its `iri` starts with `_:`); a property value is a literal. |

Node-level constraints (`sh:class`, `sh:nodeKind` written directly on the
shape) are evaluated on the focus node itself with the same code.

## What is not checked, and how you know

Everything else in the `sh:` vocabulary is **reported, never ignored in
silence**: each unsupported or malformed term is one line in
`ShapesReport.ignored` with the reason, and the rest of the shape still loads.
Today that covers `sh:and`/`sh:or`/`sh:not`/`sh:xone`/`sh:node`,
`sh:severity`/`sh:message`/`sh:deactivated` ([#100](https://github.com/sharop/nopaldb/issues/100)),
composite paths (sequences, inverse, alternatives, closures;
[#101](https://github.com/sharop/nopaldb/issues/101)), `sh:closed`,
`sh:qualifiedValueShape`, property comparisons (`sh:equals`, `sh:lessThan`, …),
`sh:languageIn`/`sh:uniqueLang` (language tags are not kept by the import),
`sh:flags`, `sh:targetSubjectsOf`/`sh:targetObjectsOf` and SHACL-SPARQL. A
property shape without `sh:path`, or with a composite one, is dropped with a
line saying so.

`ShapesReport` also carries `shapes`, `property_shapes`, `constraints` counts
and the Turtle parser's `warnings` (a missing `@prefix :`, a relative IRI
without `@base`). Malformed Turtle is an error with line and column, and
nothing is validated.

## The report

`ValidationReport { conforms, violations, notes }`. `conforms` is `true` when
no violation has severity `Violation`. Each `ConstraintViolation`:

| Field | |
|---|---|
| `focus_node` | The node that failed (`NodeId`). |
| `shape_name`, `shape_id` | Which shape. |
| `constraint` | The SHACL component, e.g. `sh:MinCountConstraintComponent`. |
| `path` | The predicate of the property shape; `None` for node-level constraints. |
| `value` | The offending value: the literal, or the `iri` of the node (its id when it has none). `None` for cardinality. |
| `message`, `severity` | Human text and `Violation` / `Warning` / `Info`. |

`notes` lists what the validator could not do at all (a `sh:targetNode` that
matches nothing). Both types derive `Serialize`.

## Shapes from Rust

The programmatic API is unchanged and shares the evaluator:

```rust
use nopaldb::shacl::{ConstraintType, PathSpec, PropertyShape, ShaclValidator, Shape, Target};

let shape = Shape::new("PersonShape")
    .with_target(Target::Class("Person".into()))
    .with_property_shape(PropertyShape::new(PathSpec::Property("age".into()), vec![ConstraintType::MinCount(1)]));
let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await?;
```

`PathSpec::Property` reads only the property, `PathSpec::Edge` only the edges,
`PathSpec::Predicate` both (what `sh:path` loads as).

## Feature

`shacl = ["regex", "owl-import"]`: shapes are Turtle, so the feature brings the
bridge's parser, and `sh:class` resolves through the taxonomy, which lives
behind `owl-import` (and its `reasoner`). `cargo build --features shacl` or any
tier from `semantic` up.
