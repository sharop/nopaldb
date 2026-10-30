// tests/nql_arithmetic_test.rs
//
// Arithmetic, `!` and `not` in NQL expressions (#269). Until 0.8.2 the
// parser kept the first operand and dropped the operator (`r.porciones * 2
// = 6` ran as `r.porciones = 6`); from 0.8.3 to 0.10.4 they were a named
// parse error, because most evaluators would have read `!(…)` as `true`
// and arithmetic as "no value". Now every evaluator shares one definition
// of the operators (`executor::value_ops`).
//
// Fictional domain: a kitchen. Recipes with servings and a region; a
// recipe uses ingredients.

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn int(i: i64) -> PropertyValue {
    PropertyValue::Int(i)
}

/// mole (3, sur), pozole (4, centro), tamal (2, sur), sopa (5, norte).
/// mole and pozole use maíz; mole uses chile.
async fn kitchen() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mut recipes = Vec::new();
    for (name, servings, region) in [("mole", 3, "sur"), ("pozole", 4, "centro"), ("tamal", 2, "sur"), ("sopa", 5, "norte")] {
        let id = g
            .add_node(
                Node::new("Receta")
                    .with_property("nombre", s(name))
                    .with_property("porciones", int(servings))
                    .with_property("region", s(region)),
            )
            .await
            .unwrap();
        recipes.push(id);
    }
    let maiz = g.add_node(Node::new("Ingrediente").with_property("nombre", s("maiz"))).await.unwrap();
    let chile = g.add_node(Node::new("Ingrediente").with_property("nombre", s("chile"))).await.unwrap();
    g.add_edge(Edge::new(recipes[0], maiz, "LLEVA").with_property("gramos", int(200))).await.unwrap();
    g.add_edge(Edge::new(recipes[1], maiz, "LLEVA").with_property("gramos", int(500))).await.unwrap();
    g.add_edge(Edge::new(recipes[0], chile, "LLEVA").with_property("gramos", int(50))).await.unwrap();
    g
}

async fn names(g: &Graph, q: &str, key: &str) -> Vec<String> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let mut out: Vec<String> = r
        .rows()
        .iter()
        .map(|row| match row.get(key) {
            Some(PropertyValue::String(s)) => s.clone(),
            other => panic!("{q}: {key} = {other:?}"),
        })
        .collect();
    out.sort();
    out
}

async fn error_of(g: &Graph, q: &str) -> String {
    match g.execute_nql(q).await {
        Ok(r) => panic!("{q}: should fail, got {} rows", r.rows().len()),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn arithmetic_and_negation_filter_by_every_path() {
    let g = kitchen().await;
    let indexed = kitchen().await;
    indexed.execute_nql("create index on Receta(region)").await.unwrap();
    let cases: &[(&str, &[&str])] = &[
        ("find r.nombre from (r:Receta) where r.porciones * 2 = 6", &["mole"]),
        ("find r.nombre from (r:Receta) where -r.porciones < -3", &["pozole", "sopa"]),
        ("find r.nombre from (r:Receta) where r.porciones + 1 > 4 and r.porciones % 2 = 0", &["pozole"]),
        ("find r.nombre from (r:Receta) where !(r.region = \"sur\")", &["pozole", "sopa"]),
        ("find r.nombre from (r:Receta) where not r.region = \"sur\"", &["pozole", "sopa"]),
        ("find r.nombre from (r:Receta) where not (r.region = \"sur\" or r.porciones > 4)", &["pozole"]),
        ("find r.nombre from (r:Receta) where r.region = \"sur\" and not r.porciones = 2", &["mole"]),
        ("find r.nombre from (r:Receta) where r.porciones / 2 = 1", &["tamal", "mole"]),
        ("find r.nombre from (r:Receta) where r.region = \"sur\" and r.porciones * 10 >= 30", &["mole"]),
    ];
    for (q, expected) in cases {
        let mut expected: Vec<String> = expected.iter().map(|x| x.to_string()).collect();
        expected.sort();
        assert_eq!(names(&g, q, "r.nombre").await, expected, "{q}");
        assert_eq!(names(&indexed, q, "r.nombre").await, expected, "{q} (index)");
    }
    // One hop and several hops.
    assert_eq!(
        names(&g, "find r.nombre from (r:Receta)-[l:LLEVA]->(i:Ingrediente) where l.gramos / 100 >= 2 and not i.nombre = \"chile\"", "r.nombre").await,
        ["mole", "pozole"]
    );
    assert_eq!(
        names(&g, "find i.nombre from (r:Receta)-[l:LLEVA]->(i:Ingrediente) where !(r.porciones - 3 = 0)", "i.nombre").await,
        ["maiz"]
    );
    // Past graph: same answer.
    let ts = g.last_commit_timestamp();
    assert_eq!(
        names(&g, &format!("find r.nombre from (r:Receta) where r.porciones * 2 = 6 at timestamp {ts}"), "r.nombre").await,
        ["mole"]
    );
}

#[tokio::test]
async fn arithmetic_in_find_is_a_column() {
    let g = kitchen().await;
    let r = g
        .execute_nql("find r.nombre, r.porciones * 2 as doble, -r.porciones from (r:Receta) where r.nombre = \"mole\"")
        .await
        .unwrap();
    assert_eq!(r.columns, ["r.nombre", "doble", "-r.porciones"]);
    assert_eq!(r.rows()[0].get("doble"), Some(&int(6)));
    assert_eq!(r.rows()[0].get("-r.porciones"), Some(&int(-3)));

    let r = g
        .execute_nql("find r.nombre, l.gramos / 10 as decenas from (r:Receta)-[l:LLEVA]->(i:Ingrediente) where i.nombre = \"chile\"")
        .await
        .unwrap();
    assert_eq!(r.rows()[0].get("decenas"), Some(&int(5)));

    // order by an arithmetic column of find.
    let r = g
        .execute_nql("find r.nombre, r.porciones * -1 as neg from (r:Receta) order by neg")
        .await
        .unwrap();
    let order: Vec<_> = r.rows().iter().map(|row| row.get("r.nombre").cloned()).collect();
    assert_eq!(order, [Some(s("sopa")), Some(s("pozole")), Some(s("mole")), Some(s("tamal"))]);
}

#[tokio::test]
async fn computed_columns_with_groups_algorithms_and_order() {
    let g = kitchen().await;
    let r = g
        .execute_nql("find r.region, count(*) * 10 as c from (r:Receta) where r.region = \"sur\" group by r.region")
        .await
        .unwrap();
    assert_eq!(r.columns, ["r.region", "c"]);
    assert_eq!(r.rows()[0].get("c"), Some(&int(20)));
    let err = error_of(&g, "find r.region, r.porciones * 2 from (r:Receta) group by r.region").await;
    assert!(err.contains("r.porciones in `r.porciones * 2` is not in group by"), "{err}");

    let r = g
        .execute_nql("find r.nombre, degree(r) * 10 as d from (r:Receta) where r.nombre = \"mole\"")
        .await
        .unwrap();
    assert_eq!(r.columns, ["r.nombre", "d"]);
    assert_eq!(r.rows()[0].get("d"), Some(&PropertyValue::Float(20.0)));

    // order by an expression that is not a column: its leaves are read and
    // dropped.
    let r = g.execute_nql("find r.nombre from (r:Receta) order by r.porciones * -1").await.unwrap();
    assert_eq!(r.columns, ["r.nombre"]);
    let order: Vec<_> = r.rows().iter().map(|row| row.get("r.nombre").cloned()).collect();
    assert_eq!(order, [Some(s("sopa")), Some(s("pozole")), Some(s("mole")), Some(s("tamal"))]);

    let err = error_of(&g, "find distinct r.porciones % 2 from (r:Receta)").await;
    assert!(err.contains("distinct with a computed column"), "{err}");
}

/// Chains of several hops and several patterns go through other evaluators.
#[tokio::test]
async fn several_hops_several_patterns_and_constants() {
    let g = kitchen().await;
    let q = "find r.nombre, o.nombre from (r:Receta)-[:LLEVA]->(i:Ingrediente)<-[:LLEVA]-(o:Receta) \
             where not r.nombre = o.nombre and o.porciones * 2 = 8";
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let pairs: Vec<_> = r.rows().iter().map(|row| (row.get("r.nombre").cloned(), row.get("o.nombre").cloned())).collect();
    assert_eq!(pairs, [(Some(s("mole")), Some(s("pozole")))], "{q}");

    let q = "find r.nombre, o.nombre from (r:Receta)-[:LLEVA]->(i:Ingrediente), (o:Receta)-[:LLEVA]->(i) \
             where r.porciones + o.porciones = 7 and !(r.porciones > o.porciones)";
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let pairs: Vec<_> = r.rows().iter().map(|row| (row.get("r.nombre").cloned(), row.get("o.nombre").cloned())).collect();
    assert_eq!(pairs, [(Some(s("mole")), Some(s("pozole")))], "{q}");

    let r = g.execute_nql("find 1 + 1 as dos from (r:Receta)").await.unwrap();
    assert_eq!(r.columns, ["dos"]);
    assert_eq!(r.rows().len(), 4);
    assert!(r.rows().iter().all(|row| row.get("dos") == Some(&int(2))));

    let err = error_of(&g, "find distinct r.region from (r:Receta) order by r.porciones * -1").await;
    assert!(err.contains("distinct with a computed column or an order by expression"), "{err}");
}

/// A literal condition is its value in writes too: until 0.10.4 anything but
/// a comparison was `false` there, so `delete … where true` deleted nothing.
#[tokio::test]
async fn a_literal_condition_in_a_write_is_its_value() {
    let g = kitchen().await;
    g.execute_nql("delete (r:Receta) where false").await.unwrap();
    assert_eq!(names(&g, "find r.nombre from (r:Receta)", "r.nombre").await.len(), 4);
    g.execute_nql("delete (r:Receta) where true").await.unwrap();
    assert!(names(&g, "find r.nombre from (r:Receta)", "r.nombre").await.is_empty());
}

#[tokio::test]
async fn having_takes_arithmetic_and_not() {
    let g = kitchen().await;
    assert_eq!(
        names(&g, "find r.region, count(*) as c from (r:Receta) group by r.region having count(*) * 2 > 3", "r.region").await,
        ["sur"]
    );
    assert_eq!(
        names(&g, "find r.region, count(*) as c from (r:Receta) group by r.region having !(count(*) > 1)", "r.region").await,
        ["centro", "norte"]
    );
}

#[tokio::test]
async fn numbers_compare_by_value() {
    let g = kitchen().await;
    let indexed = kitchen().await;
    indexed.execute_nql("create index on Receta(porciones)").await.unwrap();
    for q in [
        "find r.nombre from (r:Receta) where r.porciones = 3.0",
        "find r.nombre from (r:Receta) where r.porciones >= 3.0 and r.porciones <= 3",
        "find r.nombre from (r:Receta) where r.porciones in [3.0, 30]",
        "find r.nombre from (r:Receta) where r.porciones / 2.0 = 1.5",
    ] {
        assert_eq!(names(&g, q, "r.nombre").await, ["mole"], "{q}");
        assert_eq!(names(&indexed, q, "r.nombre").await, ["mole"], "{q} (index)");
    }
}

#[tokio::test]
async fn overflow_and_division_by_zero_are_named_errors() {
    let g = kitchen().await;
    for (q, needle) in [
        ("find r.nombre from (r:Receta) where r.porciones / 0 = 1", "division by zero"),
        ("find r.nombre from (r:Receta) where r.porciones % 0 = 1", "division by zero"),
        ("find r.nombre from (r:Receta) where r.porciones * 9223372036854775807 > 0", "overflow"),
        ("find r.nombre from (r:Receta) where r.nombre + 1 = 2", "needs two numbers"),
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains(needle), "{q}: {err}");
    }
    let err = error_of(&g, "find r.nombre from (r:Receta) where r.porciones + 1").await;
    assert!(err.contains("not a comparison"), "{err}");
}

#[tokio::test]
async fn not_is_a_keyword_but_nota_is_a_name() {
    let g = Graph::in_memory().await.unwrap();
    g.add_node(Node::new("Receta").with_property("nota", int(9))).await.unwrap();
    assert_eq!(
        g.execute_nql("find nota.nota from (nota:Receta) where not nota.nota = 1").await.unwrap().rows().len(),
        1
    );
}

#[tokio::test]
async fn set_writes_an_expression_of_the_node() {
    let g = kitchen().await;
    g.execute_nql("update (r:Receta) set r.porciones = r.porciones + 1, r.copia = r.porciones where r.region = \"sur\"")
        .await
        .unwrap();
    let r = g
        .execute_nql("find r.nombre, r.porciones, r.copia from (r:Receta) where r.region = \"sur\" order by r.nombre")
        .await
        .unwrap();
    let got: Vec<_> = r
        .rows()
        .iter()
        .map(|row| (row.get("r.porciones").cloned(), row.get("r.copia").cloned()))
        .collect();
    // Read from the node before the UPDATE: r.copia is the old value.
    assert_eq!(got, [(Some(int(4)), Some(int(3))), (Some(int(3)), Some(int(2)))]);

    // The WHERE of writes takes the operators too.
    g.execute_nql("delete (r:Receta) where not r.porciones * 2 < 10").await.unwrap();
    assert_eq!(names(&g, "find r.nombre from (r:Receta)", "r.nombre").await, ["mole", "pozole", "tamal"]);
}

#[tokio::test]
async fn set_rejects_what_it_cannot_evaluate_and_an_error_writes_nothing() {
    let g = kitchen().await;
    for (q, needle) in [
        ("update (r:Receta)-[l:LLEVA]->(i:Ingrediente) set i.de = r.nombre", "another variable"),
        ("update (r:Receta) set r.g = degree(r)", "cannot be evaluated in SET"),
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains(needle), "{q}: {err}");
    }
    g.execute_nql("update (r:Receta) set r.porciones = 9223372036854775807 where r.nombre = \"sopa\"")
        .await
        .unwrap();
    let err = error_of(&g, "update (r:Receta) set r.porciones = r.porciones + 1").await;
    assert!(err.contains("overflow"), "{err}");
    // Nothing was written: mole still has 3.
    let r = g.execute_nql("find r.porciones from (r:Receta) where r.nombre = \"mole\"").await.unwrap();
    assert_eq!(r.rows()[0].get("r.porciones"), Some(&int(3)));
}
