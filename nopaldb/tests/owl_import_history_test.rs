// tests/owl_import_history_test.rs
//
// La importación Turtle escribe sus nodos con `add_node` directo, que hasta
// 0.10.0 no versionaba: las clases importadas no tenían historia, y
// `get_class_nodes_at` (la reconstrucción histórica del razonador) las
// incluía en CUALQUIER timestamp, también antes de importarlas. Desde
// 0.10.1 toda escritura versiona. Dominio ficticio: un recetario.
//
// Required features: owl-import

use nopaldb::graph::Graph;

const RECETARIO: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix : <http://example.org/recetario#> .

:Receta  rdf:type owl:Class .
:Postre  rdf:type owl:Class .
:Postre  rdfs:subClassOf :Receta .
:flan    rdf:type :Postre .
"#;

#[tokio::test]
async fn los_nodos_importados_tienen_historia() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let antes = graph.last_commit_timestamp();

    let report = graph.import_turtle(RECETARIO).await.unwrap();
    assert_eq!(report.classes_added, 2);
    let despues = graph.last_commit_timestamp();
    assert!(despues > antes, "la importación cuenta para last_commit_timestamp");

    let nodos = graph.get_all_nodes().await.unwrap();
    assert!(nodos.len() >= 3, "dos clases y una instancia");
    for nodo in &nodos {
        let historia = graph.history(nodo.id).await.unwrap();
        assert!(!historia.is_empty(), "el nodo importado {} ({}) no tiene historia", nodo.label, nodo.id);
        assert!(historia[0].valid_from > antes);
        assert!(graph.get_node_at(nodo.id, despues).await.is_ok());
    }

    // La reconstrucción histórica ve las clases desde su importación, y no
    // antes (en 0.10.0 aparecían en cualquier timestamp).
    let ahora = graph.get_class_nodes_at(despues).await.unwrap();
    assert_eq!(ahora.len(), 2);
    let antes_de_importar = graph.get_class_nodes_at(antes).await.unwrap();
    assert!(
        antes_de_importar.is_empty(),
        "clases visibles antes de importarlas: {:?}",
        antes_de_importar.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
}
