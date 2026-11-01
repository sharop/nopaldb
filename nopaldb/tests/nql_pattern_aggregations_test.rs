// tests/nql_pattern_aggregations_test.rs
//
// Aggregations over several patterns in `from` and over a multi-hop pattern
// (#362). Until 0.10.9 both were a named error ("Pattern aggregations for
// multi-pattern / multi-hop queries are not supported yet"). The rows they
// aggregate are those of the join (#349); the semantics are those of the
// one-hop aggregation, which are SQL's: `count(*)` counts rows, `count(x)`
// the values present, `sum`/`avg` the numbers (`avg` of none is null),
// `min`/`max` order as `order by` does and are null with no values.
//
// Fictional domain: a kitchen. Recipes with a region and servings,
// ingredients with a region and grams (one has none), zones where they grow.

use std::collections::BTreeMap;

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

const RECIPES: [(&str, &str, i64); 4] = [("pozole", "centro", 4), ("mole", "sur", 6), ("sopa", "centro", 2), ("tamal", "norte", 3)];
const INGREDIENTS: [(&str, &str, Option<i64>); 4] =
    [("maiz", "centro", Some(100)), ("chile", "sur", Some(20)), ("frijol", "centro", None), ("trigo", "norte", Some(50))];
/// recipe → ingredients it takes.
const TAKES: [(&str, &[&str]); 4] =
    [("pozole", &["maiz", "chile"]), ("mole", &["chile", "maiz", "frijol"]), ("sopa", &["frijol"]), ("tamal", &["maiz"])];
/// ingredient → zone where it grows.
const GROWS: [(&str, &str); 4] = [("maiz", "valle"), ("chile", "sierra"), ("frijol", "valle"), ("trigo", "llano")];

async fn kitchen() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = BTreeMap::new();
    for (name, region, servings) in RECIPES {
        let n = Node::new("Receta")
            .with_property("nombre", s(name))
            .with_property("region", s(region))
            .with_property("porciones", PropertyValue::Int(servings));
        ids.insert(name, g.add_node(n).await.unwrap());
    }
    for (name, region, grams) in INGREDIENTS {
        let mut n = Node::new("Ingrediente").with_property("nombre", s(name)).with_property("region", s(region));
        if let Some(gr) = grams {
            n = n.with_property("gramos", PropertyValue::Int(gr));
        }
        ids.insert(name, g.add_node(n).await.unwrap());
    }
    for zone in ["valle", "sierra", "llano"] {
        ids.insert(zone, g.add_node(Node::new("Zona").with_property("nombre", s(zone))).await.unwrap());
    }
    for (recipe, list) in TAKES {
        for ingredient in list {
            g.add_edge(Edge::new(ids[recipe], ids[ingredient], "LLEVA")).await.unwrap();
        }
    }
    for (ingredient, zone) in GROWS {
        g.add_edge(Edge::new(ids[ingredient], ids[zone], "CRECE_EN")).await.unwrap();
    }
    g
}

fn grams(ingredient: &str) -> Option<i64> {
    INGREDIENTS.iter().find(|i| i.0 == ingredient).unwrap().2
}

/// The rows as `column → value`, keyed by the first column.
async fn by_first(g: &Graph, q: &str) -> BTreeMap<String, Vec<PropertyValue>> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let cols = r.columns.clone();
    r.rows()
        .iter()
        .map(|row| {
            let key = match row.get(&cols[0]) {
                Some(PropertyValue::String(v)) => v.clone(),
                other => format!("{other:?}"),
            };
            (key, cols[1..].iter().map(|c| row.get(c).cloned().unwrap_or(PropertyValue::Null)).collect())
        })
        .collect()
}

async fn single(g: &Graph, q: &str) -> Vec<PropertyValue> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    assert_eq!(r.rows().len(), 1, "{q}");
    r.columns.iter().map(|c| r.rows()[0].get(c).cloned().unwrap_or(PropertyValue::Null)).collect()
}

/// Recipe × ingredient pairs of the same region: (recipe, servings, ingredient grams).
fn same_region_pairs() -> Vec<(&'static str, &'static str, i64, Option<i64>)> {
    let mut out = Vec::new();
    for (r, region, servings) in RECIPES {
        for (_, iregion, gr) in INGREDIENTS {
            if region == iregion {
                out.push((r, region, servings, gr));
            }
        }
    }
    out
}

#[tokio::test]
async fn aggregates_over_several_patterns_match_a_count_by_hand() {
    let g = kitchen().await;
    let pairs = same_region_pairs();

    let total = single(&g, "find count(*) from (r:Receta), (i:Ingrediente) where r.region = i.region").await;
    assert_eq!(total, [PropertyValue::Int(pairs.len() as i64)]);

    let q = "find r.region, count(*), count(i.gramos), sum(r.porciones * i.gramos), avg(i.gramos), \
             min(r.porciones), max(r.porciones) \
             from (r:Receta), (i:Ingrediente) where r.region = i.region group by r.region";
    let got = by_first(&g, q).await;
    let mut expected = BTreeMap::new();
    for region in ["centro", "sur", "norte"] {
        let rows: Vec<_> = pairs.iter().filter(|p| p.1 == region).collect();
        let present: Vec<i64> = rows.iter().filter_map(|p| p.3).collect();
        let sum: i64 = rows.iter().filter_map(|p| p.3.map(|gr| gr * p.2)).sum();
        let avg = if present.is_empty() {
            PropertyValue::Null
        } else {
            PropertyValue::Float(present.iter().sum::<i64>() as f64 / present.len() as f64)
        };
        expected.insert(
            region.to_string(),
            vec![
                PropertyValue::Int(rows.len() as i64),
                PropertyValue::Int(present.len() as i64),
                PropertyValue::Float(sum as f64),
                avg,
                PropertyValue::Int(rows.iter().map(|p| p.2).min().unwrap()),
                PropertyValue::Int(rows.iter().map(|p| p.2).max().unwrap()),
            ],
        );
    }
    assert_eq!(got, expected);

    // Without `group by`, the properties of `find` group (as in the other
    // paths).
    let implicit = by_first(&g, "find r.region, count(*) from (r:Receta), (i:Ingrediente) where r.region = i.region").await;
    let explicit = by_first(
        &g,
        "find r.region, count(*) from (r:Receta), (i:Ingrediente) where r.region = i.region group by r.region",
    )
    .await;
    assert_eq!(implicit, explicit);
}

#[tokio::test]
async fn aggregates_over_a_multi_hop_pattern_match_a_count_by_hand() {
    let g = kitchen().await;
    // Ingredients of each recipe by the zone where they grow.
    let q = "find z.nombre, count(*), count(i.gramos), sum(i.gramos), max(r.porciones) \
             from (r:Receta)-[:LLEVA]->(i:Ingrediente)-[:CRECE_EN]->(z:Zona) group by z.nombre";
    let mut expected: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for (recipe, list) in TAKES {
        let servings = RECIPES.iter().find(|r| r.0 == recipe).unwrap().2;
        for ingredient in list {
            let zone = GROWS.iter().find(|g| g.0 == *ingredient).unwrap().1;
            let e = expected.entry(zone.to_string()).or_default();
            e.0 += 1;
            if let Some(gr) = grams(ingredient) {
                e.1 += 1;
                e.2 += gr;
            }
            e.3 = e.3.max(servings);
        }
    }
    let expected: BTreeMap<String, Vec<PropertyValue>> = expected
        .into_iter()
        .map(|(z, (n, c, sum, max))| {
            (z, vec![PropertyValue::Int(n), PropertyValue::Int(c), PropertyValue::Float(sum as f64), PropertyValue::Int(max)])
        })
        .collect();
    assert_eq!(by_first(&g, q).await, expected);
}

#[tokio::test]
async fn having_order_by_and_limit_apply_to_the_groups() {
    let g = kitchen().await;
    let base = "find r.region, count(*) as n from (r:Receta), (i:Ingrediente) where r.region = i.region group by r.region";
    // centro: 2 recipes × 2 ingredients = 4; sur and norte: 1.
    let got = by_first(&g, &format!("{base} having count(*) > 1")).await;
    assert_eq!(got.keys().collect::<Vec<_>>(), ["centro"]);

    let r = g.execute_nql(&format!("{base} order by n desc, r.region limit 2")).await.unwrap();
    let order: Vec<_> = r.rows().iter().map(|row| row.get("r.region").cloned()).collect();
    assert_eq!(order, [Some(s("centro")), Some(s("norte"))]);

    // `limit` counts groups, not the rows of the join: the join must not
    // stop at the first row.
    let total = single(&g, "find count(*) from (r:Receta), (i:Ingrediente) limit 1").await;
    assert_eq!(total, [PropertyValue::Int(16)]);

    // An aggregate in an expression of `find` (#269).
    let doubled = by_first(
        &g,
        "find r.region, count(*) * 2 as doble from (r:Receta), (i:Ingrediente) where r.region = i.region group by r.region",
    )
    .await;
    assert_eq!(doubled["centro"], [PropertyValue::Int(8)]);
}

#[tokio::test]
async fn no_rows_give_a_zero_count_or_no_groups() {
    let g = kitchen().await;
    let none = "from (r:Receta), (i:Ingrediente) where r.porciones > 100";
    assert_eq!(single(&g, &format!("find count(*), sum(r.porciones), avg(r.porciones), max(r.porciones) {none}")).await, [
        PropertyValue::Int(0),
        PropertyValue::Float(0.0),
        PropertyValue::Null,
        PropertyValue::Null
    ]);
    let r = g.execute_nql(&format!("find r.region, count(*) {none} group by r.region")).await.unwrap();
    assert!(r.rows().is_empty());
}

#[tokio::test]
async fn the_same_answers_with_an_index_reordered_and_in_the_past() {
    let g = kitchen().await;
    let q = "find z.nombre, count(*) from (r:Receta), (z:Zona), (i:Ingrediente) \
             where r.region = i.region and z.nombre = 'valle' group by z.nombre";
    // In `q`, `z` relates to nothing before it: the join goes r, i, z (#349).
    let before = by_first(&g, q).await;
    let plain = by_first(&g, "find z.nombre, count(*) from (r:Receta), (i:Ingrediente), (z:Zona) \
                              where r.region = i.region and z.nombre = 'valle' group by z.nombre")
    .await;
    assert_eq!(before, plain);

    g.execute_nql("create index on Receta(region)").await.unwrap();
    g.execute_nql("create index on Zona(nombre)").await.unwrap();
    assert_eq!(by_first(&g, q).await, before);

    let ts = g.last_commit_timestamp();
    g.add_node(
        Node::new("Receta")
            .with_property("nombre", s("atole"))
            .with_property("region", s("centro"))
            .with_property("porciones", PropertyValue::Int(1)),
    )
    .await
    .unwrap();
    assert_ne!(by_first(&g, q).await, before);
    assert_eq!(by_first(&g, &format!("{q} at timestamp {ts}")).await, before);
}

#[tokio::test]
async fn what_is_not_supported_is_a_named_error() {
    let g = kitchen().await;
    // A column that is neither a group key nor an aggregate (#335).
    let err = g
        .execute_nql("find r.region, i.nombre, count(*) from (r:Receta), (i:Ingrediente) group by r.region")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("i.nombre is not in group by"), "{err}");
    // A per-node algorithm inside an aggregate.
    let err = g.execute_nql("find avg(degree(r)) from (r:Receta), (i:Ingrediente)").await.unwrap_err();
    assert!(err.to_string().contains("degree(r) is not supported in aggregations"), "{err}");
    // The join still has its row limit: the aggregate needs every row.
    g.set_max_join_rows(10);
    let err = g.execute_nql("find count(*) from (r:Receta), (i:Ingrediente)").await.unwrap_err();
    assert!(err.to_string().contains("max_join_rows"), "{err}");
}
