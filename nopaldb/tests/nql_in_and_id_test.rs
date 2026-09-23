//! NQL 0.6.8: `where n.id = "…"` se resuelve por lectura puntual (antes era
//! un scan completo de la etiqueta, dos veces), y el operador `in` / `not in`
//! con listas literales, con y sin índice. EXPLAIN reporta los caminos nuevos
//! desde la misma decisión que ejecuta.

use nopaldb::{Graph, Node, NodeId, PropertyValue};

async fn fixture() -> (Graph, Vec<NodeId>) {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::new();
    for (name, age) in [("Ana", 30), ("Beto", 25), ("Cami", 41)] {
        let n = Node::new("Person")
            .with_property("name", PropertyValue::String(name.into()))
            .with_property("age", PropertyValue::Int(age));
        ids.push(g.add_node(n).await.unwrap());
    }
    g.add_node(Node::new("Other").with_property("name", PropertyValue::String("Ana".into()))).await.unwrap();
    g.create_index("Person", "name", nopaldb::index::IndexType::Hash).await.unwrap();
    (g, ids)
}

async fn names(g: &Graph, q: &str) -> Vec<String> {
    let r = g.execute_nql(q).await.unwrap();
    let mut out: Vec<String> = r
        .rows()
        .iter()
        .filter_map(|row| row.get("p.name").map(|v| v.to_display_string()))
        .collect();
    out.sort();
    out
}

async fn explain(g: &Graph, q: &str) -> String {
    let r = g.execute_statement(&format!("explain {q}")).await.unwrap();
    match r {
        nopaldb::NqlResult::Explain(plan) => plan,
        other => panic!("expected an explain result, got {other:?}"),
    }
}

#[tokio::test]
async fn id_equality_is_a_point_read_hit_and_miss() {
    let (g, ids) = fixture().await;
    let q = format!("find p.name from (p:Person) where p.id = \"{}\"", ids[0]);
    assert_eq!(names(&g, &q).await, vec!["Ana"]);
    let plan = explain(&g, &q).await;
    assert!(plan.contains("ID LOOKUP"), "{plan}");
    assert!(!plan.contains("LABEL SCAN"), "{plan}");

    let miss = format!("find p.name from (p:Person) where p.id = \"{}\"", uuid::Uuid::new_v4());
    assert!(names(&g, &miss).await.is_empty(), "a missing id is zero rows, not a scan");
    let plan = explain(&g, &miss).await;
    assert!(plan.contains("ID LOOKUP") && plan.contains("sin scan"), "{plan}");

    let bad = "find p.name from (p:Person) where p.id = \"no-es-uuid\"";
    assert!(names(&g, bad).await.is_empty());
    assert!(explain(&g, bad).await.contains("no-UUID"), "{}", explain(&g, bad).await);

    // La etiqueta del patrón se respeta: el id de una Person no es un Other.
    let wrong_label = format!("find p.name from (p:Other) where p.id = \"{}\"", ids[0]);
    assert!(names(&g, &wrong_label).await.is_empty());
}

#[tokio::test]
async fn id_in_list_and_root_and_seed_the_lookup() {
    let (g, ids) = fixture().await;
    let q = format!("find p.name from (p:Person) where p.id in [\"{}\", \"{}\"]", ids[0], ids[2]);
    assert_eq!(names(&g, &q).await, vec!["Ana", "Cami"]);
    assert!(explain(&g, &q).await.contains("ID LOOKUP"));

    // AND raíz: el id siembra y el resto se aplica como predicado.
    let q = format!("find p.name from (p:Person) where p.age > 26 and p.id = \"{}\"", ids[0]);
    assert_eq!(names(&g, &q).await, vec!["Ana"]);
    assert!(explain(&g, &q).await.contains("ID LOOKUP"));
    let q = format!("find p.name from (p:Person) where p.id = \"{}\" and p.age > 40", ids[0]);
    assert!(names(&g, &q).await.is_empty());

    // OR raíz no siembra: sigue siendo scan (y sigue siendo correcto).
    let q = format!("find p.name from (p:Person) where p.id = \"{}\" or p.age = 41", ids[0]);
    assert_eq!(names(&g, &q).await, vec!["Ana", "Cami"]);
    assert!(explain(&g, &q).await.contains("LABEL SCAN"));
}

#[tokio::test]
async fn in_with_and_without_index_not_in_and_strict_equality() {
    let (g, _ids) = fixture().await;
    let q = "find p.name from (p:Person) where p.name in [\"Ana\", \"Beto\", \"Nadie\"]";
    assert_eq!(names(&g, q).await, vec!["Ana", "Beto"]);
    let plan = explain(&g, q).await;
    assert!(plan.contains("INDEX SEEK (IN)") && plan.contains("Person_name"), "{plan}");

    // Sin índice sobre `age`: scan de etiqueta con el predicado.
    let q = "find p.name from (p:Person) where p.age in [30, 41]";
    assert_eq!(names(&g, q).await, vec!["Ana", "Cami"]);
    assert!(explain(&g, q).await.contains("LABEL SCAN"));

    // Igualdad estricta: 25.0 no es 25.
    assert_eq!(names(&g, "find p.name from (p:Person) where p.age in [30, 25.0]").await, vec!["Ana"]);
    assert!(names(&g, "find p.name from (p:Person) where p.name in []").await.is_empty());

    assert_eq!(names(&g, "find p.name from (p:Person) where p.name not in [\"Ana\"]").await, vec!["Beto", "Cami"]);
    // `not in` sobre una propiedad ausente: la fila se excluye (no hay valor que comparar).
    assert!(names(&g, "find p.name from (p:Person) where p.missing not in [\"x\"]").await.is_empty());
    // `in` no se come identificadores que empiezan por "in".
    let g2 = Graph::in_memory().await.unwrap();
    g2.add_node(Node::new("T").with_property("index", PropertyValue::Int(1)).with_property("name", PropertyValue::String("t".into()))).await.unwrap();
    let r = g2.execute_nql("find p.name from (p:T) where p.index in [1]").await.unwrap();
    assert_eq!(r.rows().len(), 1);
}

#[tokio::test]
async fn in_works_in_update_and_delete() {
    let (g, _ids) = fixture().await;
    g.execute_statement("update (p:Person) set p.vip = true where p.name in [\"Ana\", \"Cami\"]").await.unwrap();
    let r = g.execute_nql("find p.name from (p:Person) where p.vip = true").await.unwrap();
    assert_eq!(r.rows().len(), 2);
    g.execute_statement("delete (p:Person) where p.name in [\"Beto\"]").await.unwrap();
    assert_eq!(g.get_label_count("Person").await.unwrap(), 2);
}
