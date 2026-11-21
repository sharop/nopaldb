// tests/nql_write_inline_props_test.rs
//
// `delete`/`update` de un solo nodo respetan las propiedades en línea del
// patrón, como `find` con el mismo patrón. Hasta 0.10.17 las ignoraban:
// `delete (n {k: "a"})` borraba todos los nodos y `update (n:N {k: "a"})`
// cambiaba todos los `N`.

use nopaldb::{Graph, PropertyValue, Result};

async fn graph() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    for q in [
        r#"add (a:N {k: "a", n: 1})"#,
        r#"add (b:N {k: "b", n: 2})"#,
        r#"add (c:M {k: "c", n: 1})"#,
    ] {
        g.execute_nql(q).await?;
    }
    Ok(g)
}

/// `(k, x)` of every node, sorted.
async fn state(g: &Graph) -> Result<Vec<(String, Option<PropertyValue>)>> {
    let mut out: Vec<_> = g
        .get_all_nodes()
        .await?
        .into_iter()
        .map(|n| {
            let k = match n.properties.get("k") {
                Some(PropertyValue::String(s)) => s.clone(),
                other => panic!("{other:?}"),
            };
            (k, n.properties.get("x").cloned())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn keys(state: &[(String, Option<PropertyValue>)]) -> Vec<&str> {
    state.iter().map(|(k, _)| k.as_str()).collect()
}

fn touched(state: &[(String, Option<PropertyValue>)]) -> Vec<&str> {
    state.iter().filter(|(_, x)| x.is_some()).map(|(k, _)| k.as_str()).collect()
}

#[tokio::test]
async fn delete_touches_only_the_nodes_find_returns() -> Result<()> {
    for (q, left) in [
        (r#"delete (n {k: "a"})"#, vec!["b", "c"]),
        (r#"delete (n:N {k: "a"})"#, vec!["b", "c"]),
        (r#"delete (n:M {k: "a"})"#, vec!["a", "b", "c"]),
        // Every inline property must match.
        (r#"delete (n {n: 1})"#, vec!["b"]),
        (r#"delete (n {n: 1, k: "c"})"#, vec!["a", "b"]),
        // A missing property, or a value of another type, does not match.
        (r#"delete (n {zzz: 1})"#, vec!["a", "b", "c"]),
        (r#"delete (n {n: "1"})"#, vec!["a", "b", "c"]),
        // Inline properties and `where` together.
        (r#"delete (n {n: 1}) where n.k != "a""#, vec!["a", "b"]),
    ] {
        let g = graph().await?;
        let found = g.execute_nql(&q.replacen("delete", "find n.k from", 1)).await?.len();
        g.execute_nql(q).await?;
        let s = state(&g).await?;
        assert_eq!(keys(&s), left, "{q}");
        assert_eq!(3 - left.len(), found, "{q}: delete and find disagree");
    }
    Ok(())
}

#[tokio::test]
async fn update_touches_only_the_nodes_find_returns() -> Result<()> {
    for (q, changed) in [
        (r#"update (n {k: "a"}) set n.x = 1"#, vec!["a"]),
        (r#"update (n:N {k: "a"}) set n.x = 1"#, vec!["a"]),
        (r#"update (n:N) set n.x = 1"#, vec!["a", "b"]),
        (r#"update (n {n: 1}) set n.x = 1"#, vec!["a", "c"]),
        (r#"update (n {k: "zzz"}) set n.x = 1"#, vec![]),
    ] {
        let g = graph().await?;
        g.execute_nql(q).await?;
        let s = state(&g).await?;
        assert_eq!(touched(&s), changed, "{q}");
        assert_eq!(s.len(), 3);
    }
    Ok(())
}

/// `limit` counts among the nodes the inline properties keep, not among
/// all the nodes of the label: with twenty `N`, `limit 1` touches `k7`.
#[tokio::test]
async fn limit_applies_after_the_inline_properties() -> Result<()> {
    async fn twenty() -> Result<Graph> {
        let g = Graph::in_memory().await?;
        for i in 0..20 {
            g.execute_nql(&format!(r#"add (n:N {{k: "k{i}"}})"#)).await?;
        }
        Ok(g)
    }
    let g = twenty().await?;
    g.execute_nql(r#"update (n:N {k: "k7"}) set n.x = 1 limit 1"#).await?;
    assert_eq!(touched(&state(&g).await?), ["k7"]);

    let g = twenty().await?;
    g.execute_nql(r#"delete (n:N {k: "k7"}) limit 1"#).await?;
    let s = state(&g).await?;
    assert_eq!(s.len(), 19);
    assert!(!keys(&s).contains(&"k7"));
    Ok(())
}
