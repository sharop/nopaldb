// #190 (a): Leiden ponderado y acotado.
//
// `leiden(n, labels = [...], edge_types = [...], weight = "w", gamma = g)` en
// NQL y `LeidenConfig { labels, edge_types, weight_property, .. }` en Rust:
// la partición se calcula solo sobre los nodos y aristas pedidos, y con el
// peso de las aristas. Antes: todo el grafo y peso fijo 1.

use std::collections::HashMap;

use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig};
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn add(g: &Graph, label: &str, name: &str) -> NodeId {
    g.add_node(Node::new(label).with_property("name", s(name))).await.unwrap()
}

async fn edge(g: &Graph, a: NodeId, b: NodeId, t: &str, w: Option<f64>) {
    let mut e = Edge::new(a, b, t);
    if let Some(w) = w {
        e = e.with_property("w", PropertyValue::Float(w));
    }
    g.add_edge(e).await.unwrap();
}

/// `name → comunidad` de una consulta NQL con `leiden(n, …) as c`.
async fn nql_parts(g: &Graph, nql: &str) -> HashMap<String, Option<i64>> {
    g.execute_nql(nql)
        .await
        .unwrap_or_else(|e| panic!("{nql}\n  {e}"))
        .rows()
        .iter()
        .map(|r| {
            let name = r.get("n.name").unwrap().as_str().unwrap().to_string();
            // Por la ruta de agregación (n.name agrupa) la comunidad sale como
            // Float; por la de patrones, como Int.
            let c = match r.get("c") {
                Some(PropertyValue::Int(c)) => Some(*c),
                Some(PropertyValue::Float(c)) => Some(*c as i64),
                _ => None,
            };
            (name, c)
        })
        .collect()
}

/// Dos pares fuertes, {A,B} y {C,D} (peso 10), y un nodo X con dos aristas
/// débiles hacia A y B (0.1) y una fuerte hacia C (5). Leiden plano mueve
/// nodos de uno en uno: sin peso X cuenta aristas y se va con {A,B} (2 contra
/// 1); con peso se va con {C,D} (5 contra 0.2).
async fn weighted_fixture(g: &Graph) -> [NodeId; 5] {
    let ids = [
        add(g, "Entity", "A").await,
        add(g, "Entity", "B").await,
        add(g, "Entity", "C").await,
        add(g, "Entity", "D").await,
        add(g, "Entity", "X").await,
    ];
    edge(g, ids[0], ids[1], "RELATED", Some(10.0)).await;
    edge(g, ids[2], ids[3], "RELATED", Some(10.0)).await;
    edge(g, ids[4], ids[0], "RELATED", Some(0.1)).await;
    edge(g, ids[4], ids[1], "RELATED", Some(0.1)).await;
    edge(g, ids[4], ids[2], "RELATED", Some(5.0)).await;
    ids
}

#[tokio::test]
async fn weights_change_the_partition() {
    let g = Graph::in_memory().await.unwrap();
    weighted_fixture(&g).await;
    let plain = nql_parts(&g, "find n.name, leiden(n) as c from (n:Entity)").await;
    assert!(plain.values().all(Option::is_some), "toda entidad tiene comunidad: {plain:?}");
    assert_eq!(plain["A"], plain["B"], "{plain:?}");
    assert_eq!(plain["C"], plain["D"], "{plain:?}");
    assert_eq!(plain["X"], plain["A"], "sin peso X cuenta aristas: va con A y B: {plain:?}");

    let weighted = nql_parts(&g, r#"find n.name, leiden(n, weight = "w") as c from (n:Entity)"#).await;
    assert_eq!(weighted["A"], weighted["B"], "{weighted:?}");
    assert_eq!(weighted["C"], weighted["D"], "{weighted:?}");
    assert_eq!(weighted["X"], weighted["C"], "con peso X va con C y D: {weighted:?}");

    // Consultas seguidas con opciones distintas no se devuelven la partición
    // de la otra (la caché lleva la configuración en la clave).
    let plain_again = nql_parts(&g, "find n.name, leiden(n) as c from (n:Entity)").await;
    assert_eq!(plain_again, plain);
}

/// Con `labels` y `edge_types` los nodos de texto y sus aristas no cuentan.
/// Dos grupos de entidades unidos solo a través de un chunk que menciona a
/// las dos: sin acotar, el chunk los une; acotado a `Entity`/`RELATED`, no.
#[tokio::test]
async fn labels_and_edge_types_scope_the_partition() {
    let g = Graph::in_memory().await.unwrap();
    let a: Vec<NodeId> = vec![add(&g, "Entity", "a1").await, add(&g, "Entity", "a2").await, add(&g, "Entity", "a3").await];
    let b: Vec<NodeId> = vec![add(&g, "Entity", "b1").await, add(&g, "Entity", "b2").await, add(&g, "Entity", "b3").await];
    for group in [&a, &b] {
        edge(&g, group[0], group[1], "RELATED", None).await;
        edge(&g, group[1], group[2], "RELATED", None).await;
        edge(&g, group[2], group[0], "RELATED", None).await;
    }
    let chunk = add(&g, "Chunk", "c1").await;
    for e in a.iter().chain(&b) {
        edge(&g, chunk, *e, "MENTIONS", None).await;
    }

    let config = LeidenConfig { labels: Some(vec!["Entity".into()]), edge_types: Some(vec!["RELATED".into()]), ..Default::default() };
    let parts = LeidenCommunity::new(config).detect(&g).await.unwrap();
    assert!(!parts.contains_key(&chunk), "el chunk no participa");
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[&a[0]], parts[&a[2]]);
    assert_eq!(parts[&b[0]], parts[&b[2]]);
    assert_ne!(parts[&a[0]], parts[&b[0]], "sin el chunk, los grupos quedan separados");

    let nql = nql_parts(&g, r#"find n.name, leiden(n, labels = ["Entity"], edge_types = ["RELATED"]) as c from (n)"#).await;
    assert!(nql["a1"].is_some() && nql["b1"].is_some(), "{nql:?}");
    assert_eq!(nql["c1"], None, "un nodo fuera del alcance no tiene comunidad");
    assert_ne!(nql["a1"], nql["b1"]);
}

/// La misma configuración en dos lugares de la consulta es válida; dos
/// configuraciones distintas, no.
#[tokio::test]
async fn one_leiden_configuration_per_query() {
    let g = Graph::in_memory().await.unwrap();
    weighted_fixture(&g).await;
    let ok = g
        .execute_nql(r#"find n.name, leiden(n, weight = "w") as c from (n:Entity) where leiden(n, weight = "w") >= 0"#)
        .await;
    assert!(ok.is_ok(), "{ok:?}");
    let err = g
        .execute_nql(r#"find n.name, leiden(n, weight = "w") as c from (n:Entity) where leiden(n) >= 0"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("must use the same options"), "{err}");
}

#[tokio::test]
async fn invalid_options_and_weights_are_named_errors() {
    let g = Graph::in_memory().await.unwrap();
    let ids = weighted_fixture(&g).await;
    for (opts, needle) in [
        ("rrf_k = 3", "unknown option `rrf_k`"),
        (r#"labels = "Entity""#, "option `labels`"),
        ("labels = []", "option `labels`"),
        ("gamma = 0", "option `gamma`"),
        ("weight = 3", "option `weight`"),
    ] {
        let err = g.execute_nql(&format!("find leiden(n, {opts}) as c from (n:Entity)")).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{opts}: {err}");
    }
    let err = g.execute_nql("find leiden(n, n) as c from (n:Entity)").await.unwrap_err().to_string();
    assert!(err.contains("exactly one positional"), "{err}");

    edge(&g, ids[0], ids[2], "RELATED", Some(-1.0)).await;
    let err = LeidenCommunity::new(LeidenConfig { weight_property: Some("w".into()), ..Default::default() })
        .detect(&g)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("must be finite and >= 0"), "{err}");
}

/// Con pesos reales la partición no depende del orden en que se insertaron
/// las aristas (los vecinos se suman en orden de `NodeId`).
#[tokio::test]
async fn the_weighted_partition_does_not_depend_on_insertion_order() {
    async fn build(reverse: bool) -> (Graph, Vec<NodeId>) {
        let g = Graph::in_memory().await.unwrap();
        let mut ids = Vec::new();
        for i in 0..24 {
            ids.push(add(&g, "Entity", &format!("e{i:02}")).await);
        }
        let mut edges: Vec<(usize, usize, f64)> = (0..24)
            .flat_map(|i| [(i, (i + 1) % 24, 1.0 + (i % 5) as f64 * 0.37), (i, (i * 7 + 3) % 24, 0.11 * ((i % 3) as f64 + 1.0))])
            .filter(|(a, b, _)| a != b)
            .collect();
        if reverse {
            edges.reverse();
        }
        for (a, b, w) in edges {
            edge(&g, ids[a], ids[b], "RELATED", Some(w)).await;
        }
        (g, ids)
    }
    let config = || LeidenConfig { weight_property: Some("w".into()), gamma: 0.3, ..Default::default() };
    let (g1, ids1) = build(false).await;
    let (g2, ids2) = build(true).await;
    let p1 = LeidenCommunity::new(config()).detect(&g1).await.unwrap();
    let p2 = LeidenCommunity::new(config()).detect(&g2).await.unwrap();
    // Misma agrupación (los números de comunidad pueden diferir).
    for i in 0..24 {
        for j in 0..24 {
            assert_eq!(p1[&ids1[i]] == p1[&ids1[j]], p2[&ids2[i]] == p2[&ids2[j]], "e{i:02} y e{j:02}");
        }
    }
}
