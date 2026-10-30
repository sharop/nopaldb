// tests/nql_disconnected_patterns_test.rs
//
// Several patterns in `from` that share no variable give the cross product,
// as `MATCH (a), (b)` in openCypher and `FROM a, b` in SQL (#345). The join
// already did it when a pattern had an edge; but when every pattern was a
// single node the query took the single-node path, which reads only the
// first pattern. Until 0.10.4 `from (a:Receta), (b:Receta)` gave the rows
// of `a` with `b` empty, and `count(*)` counted only `a`.
//
// Fictional domain: a kitchen. Recipes with servings; ingredients.

use nopaldb::types::{Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// pozole (4), sopa (5), mole (3); ingredients maiz and chile.
async fn kitchen() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    for (name, servings) in [("pozole", 4), ("sopa", 5), ("mole", 3)] {
        g.add_node(
            Node::new("Receta")
                .with_property("nombre", s(name))
                .with_property("porciones", PropertyValue::Int(servings)),
        )
        .await
        .unwrap();
    }
    for name in ["maiz", "chile"] {
        g.add_node(Node::new("Ingrediente").with_property("nombre", s(name))).await.unwrap();
    }
    g
}

/// The rows as sorted `(a, b)` pairs of the given columns.
async fn pairs(g: &Graph, q: &str, left: &str, right: &str) -> Vec<(String, String)> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let text = |row: &nopaldb::query::nql::executor::Row, key: &str| match row.get(key) {
        Some(PropertyValue::String(v)) => v.clone(),
        other => panic!("{q}: {key} = {other:?}"),
    };
    let mut out: Vec<(String, String)> = r.rows().iter().map(|row| (text(row, left), text(row, right))).collect();
    out.sort();
    out
}

fn expected(list: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    out.sort();
    out
}

const RECIPES: [&str; 3] = ["pozole", "sopa", "mole"];

#[tokio::test]
async fn node_patterns_without_a_shared_variable_give_the_cross_product() {
    let g = kitchen().await;
    let all: Vec<(&str, &str)> = RECIPES.iter().flat_map(|a| RECIPES.iter().map(move |b| (*a, *b))).collect();
    assert_eq!(
        pairs(&g, "find a.nombre, b.nombre from (a:Receta), (b:Receta)", "a.nombre", "b.nombre").await,
        expected(&all)
    );
    // Two labels: every recipe with every ingredient.
    let mixed: Vec<(&str, &str)> = RECIPES.iter().flat_map(|a| ["maiz", "chile"].map(move |b| (*a, b))).collect();
    assert_eq!(
        pairs(&g, "find a.nombre, i.nombre from (a:Receta), (i:Ingrediente)", "a.nombre", "i.nombre").await,
        expected(&mixed)
    );
}

#[tokio::test]
async fn where_relates_the_patterns() {
    let g = kitchen().await;
    let indexed = kitchen().await;
    indexed.execute_nql("create index on Receta(porciones)").await.unwrap();
    for graph in [&g, &indexed] {
        assert_eq!(
            pairs(
                graph,
                "find a.nombre, b.nombre from (a:Receta), (b:Receta) where a.porciones = 4 and b.porciones = 5",
                "a.nombre",
                "b.nombre"
            )
            .await,
            expected(&[("pozole", "sopa")])
        );
        assert_eq!(
            pairs(
                graph,
                "find a.nombre, b.nombre from (a:Receta), (b:Receta) where a.porciones < b.porciones",
                "a.nombre",
                "b.nombre"
            )
            .await,
            expected(&[("mole", "pozole"), ("mole", "sopa"), ("pozole", "sopa")])
        );
    }
}

#[tokio::test]
async fn anonymous_patterns_and_inline_properties_count() {
    let g = kitchen().await;
    // An anonymous pattern still multiplies: one row per recipe and ingredient.
    let r = g.execute_nql("find a.nombre from (a:Receta), (:Ingrediente)").await.unwrap();
    assert_eq!(r.rows().len(), 6);
    assert_eq!(
        pairs(&g, "find a.nombre, b.nombre from (a:Receta {porciones: 4}), (b:Receta)", "a.nombre", "b.nombre").await,
        expected(&[("pozole", "mole"), ("pozole", "pozole"), ("pozole", "sopa")])
    );
}

#[tokio::test]
async fn order_by_and_limit_apply_to_the_product() {
    let g = kitchen().await;
    let r = g
        .execute_nql("find a.nombre, b.nombre from (a:Receta), (b:Receta) order by b.nombre, a.nombre limit 2")
        .await
        .unwrap();
    let rows: Vec<(String, String)> = r
        .rows()
        .iter()
        .map(|row| match (row.get("a.nombre"), row.get("b.nombre")) {
            (Some(PropertyValue::String(a)), Some(PropertyValue::String(b))) => (a.clone(), b.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(rows, expected(&[("mole", "mole"), ("pozole", "mole")]));
}

#[tokio::test]
async fn at_timestamp_gives_the_product_of_the_past() {
    let g = kitchen().await;
    let ts = g.last_commit_timestamp();
    g.add_node(
        Node::new("Receta")
            .with_property("nombre", s("tamal"))
            .with_property("porciones", PropertyValue::Int(2)),
    )
    .await
    .unwrap();
    assert_eq!(
        pairs(
            &g,
            &format!("find a.nombre, b.nombre from (a:Receta), (b:Receta) where a.porciones < b.porciones at timestamp {ts}"),
            "a.nombre",
            "b.nombre"
        )
        .await,
        expected(&[("mole", "pozole"), ("mole", "sopa"), ("pozole", "sopa")])
    );
}

#[tokio::test]
async fn an_aggregate_over_several_patterns_is_a_named_error() {
    // Until 0.10.4 this counted the first pattern only (3). Aggregations over
    // several patterns are not supported yet: an error, not a wrong number.
    let g = kitchen().await;
    let err = g.execute_nql("find count(*) from (a:Receta), (b:Receta)").await.unwrap_err();
    assert!(err.to_string().contains("multi-pattern"), "{err}");
}

#[tokio::test]
async fn explain_says_why_the_index_is_not_used() {
    let g = kitchen().await;
    g.execute_nql("create index on Receta(porciones)").await.unwrap();
    let r = g
        .execute_nql("explain find a.nombre from (a:Receta), (b:Receta) where a.porciones = 4")
        .await
        .unwrap();
    let plan = format!("{:?}", r.rows());
    assert!(plan.contains("varios patrones"), "{plan}");
}
