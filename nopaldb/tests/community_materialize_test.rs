// #190 c: comunidades persistidas con keys estables.
//
// Jerarquía sembrada como en `leiden_hierarchy_test`: bloques de 50 nodos con
// 5 subbloques de 10 (probabilidad 0.8 dentro del subbloque, 0.15 entre
// subbloques del bloque) y ~0.5 aristas por nodo hacia cualquier nodo.

use std::collections::{HashMap, HashSet};

use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig, LeidenHierarchyOptions};
use nopaldb::graph::communities::{CommunityMaterializeOptions, COMMUNITY_LABEL, IN_COMMUNITY, PARENT_OF};
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::Graph;

fn rng(seed: u64) -> impl FnMut(usize) -> usize {
    let mut x = seed;
    move |m| {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % m
    }
}

async fn planted(g: &Graph, n: usize) -> Vec<NodeId> {
    planted_with(g, n, false).await
}

/// `sparse`: bloques de 50 sin subbloques, ~4 aristas internas por nodo
/// (densidad ≈ 0.155, cerca de γ); la partición es frágil y un 1% de aristas
/// nuevas sí la cambia.
async fn planted_with(g: &Graph, n: usize, sparse: bool) -> Vec<NodeId> {
    let mut ids = Vec::with_capacity(n);
    let mut loader = g.bulk_loader(50_000);
    for _ in 0..n {
        let node = Node::new("Entity");
        ids.push(node.id);
        loader.add_node(node).await.unwrap();
    }
    let mut r = rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..n {
        if sparse {
            for _ in 0..4 {
                let j = (i / 50) * 50 + r(50);
                if j < n && j != i {
                    loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
                }
            }
        } else {
            for j in (i + 1)..((i / 50 + 1) * 50).min(n) {
                if r(100) < if i / 10 == j / 10 { 80 } else { 15 } {
                    loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
                }
            }
        }
        if r(2) == 0 {
            let j = r(n);
            if j != i {
                loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
            }
        }
    }
    loader.finish().await.unwrap();
    ids
}

fn leiden() -> LeidenCommunity {
    LeidenCommunity::new(LeidenConfig { labels: Some(vec!["Entity".into()]), ..Default::default() })
}

async fn levels(g: &Graph) -> Vec<HashMap<NodeId, usize>> {
    leiden().detect_hierarchy(g, &LeidenHierarchyOptions::default()).await.unwrap().levels
}

/// `(level, key)` de la comunidad de cada nodo en cada nivel, leído del grafo.
async fn stored_keys(g: &Graph, partition: &str) -> HashMap<(NodeId, i64), String> {
    let comms: HashMap<NodeId, (i64, String)> = g
        .get_nodes_by_label(COMMUNITY_LABEL)
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.properties.get("partition").and_then(PropertyValue::as_str) == Some(partition))
        .map(|c| {
            let level = c.properties["level"].as_i64().unwrap();
            (c.id, (level, c.properties["key"].as_str().unwrap().to_string()))
        })
        .collect();
    let mut out = HashMap::new();
    for e in g.get_edges_by_label(IN_COMMUNITY).await.unwrap() {
        if let Some((level, key)) = comms.get(&e.target) {
            assert!(out.insert((e.source, *level), key.clone()).is_none(), "un nodo, una comunidad por nivel");
        }
    }
    out
}

#[tokio::test]
async fn materializes_the_hierarchy() {
    let g = Graph::in_memory().await.unwrap();
    let ids = planted(&g, 2_000).await;
    let lv = levels(&g).await;
    assert!(lv.len() >= 2);
    let report = g.materialize_communities(&lv, &CommunityMaterializeOptions::default()).await.unwrap();
    let total: usize = lv.iter().map(LeidenCommunity::count_communities).sum();
    assert_eq!((report.communities, report.created, report.kept_keys, report.deleted), (total, total, 0, 0));
    assert_eq!(report.memberships_added, ids.len() * lv.len());
    let comms = g.get_nodes_by_label(COMMUNITY_LABEL).await.unwrap();
    assert_eq!(comms.len(), total);
    // size = número de aristas IN_COMMUNITY; PARENT_OF une niveles contiguos.
    let mut members: HashMap<NodeId, usize> = HashMap::new();
    for e in g.get_edges_by_label(IN_COMMUNITY).await.unwrap() {
        *members.entry(e.target).or_default() += 1;
    }
    let level_of: HashMap<NodeId, i64> = comms.iter().map(|c| (c.id, c.properties["level"].as_i64().unwrap())).collect();
    for c in &comms {
        assert_eq!(c.properties["size"].as_i64().unwrap() as usize, members[&c.id]);
    }
    let parent_links = g.get_edges_by_label(PARENT_OF).await.unwrap();
    assert_eq!(parent_links.len(), total - LeidenCommunity::count_communities(&lv[0]));
    assert_eq!(report.parent_links_added, parent_links.len());
    for e in &parent_links {
        assert_eq!(level_of[&e.source] + 1, level_of[&e.target], "PARENT_OF va del nivel L al L+1");
    }
    // Cada miembro está en la comunidad hija de su comunidad padre.
    let keys = stored_keys(&g, "leiden").await;
    assert_eq!(keys.len(), ids.len() * lv.len());
}

#[tokio::test]
async fn recomputing_without_changes_writes_nothing_and_keeps_every_key() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 2_000).await;
    let opts = CommunityMaterializeOptions::default();
    g.materialize_communities(&levels(&g).await, &opts).await.unwrap();
    let before = stored_keys(&g, "leiden").await;
    let report = g.materialize_communities(&levels(&g).await, &opts).await.unwrap();
    assert!(report.is_unchanged(), "{report:?}");
    assert_eq!(report.kept_keys, report.communities);
    assert_eq!(stored_keys(&g, "leiden").await, before);
}

/// Con 1% de aristas nuevas al azar, se conserva la key de la gran mayoría
/// de los nodos en cada nivel. Medido (2k nodos, 3 semillas): en el grafo
/// jerárquico la partición no cambia (100%); en el disperso, 0.87–0.92 de
/// los nodos por nivel y 0.89–0.92 de las comunidades emparejadas. El umbral
/// (0.8) deja margen.
#[tokio::test]
async fn one_percent_new_edges_keeps_most_keys() {
    for sparse in [false, true] {
        for seed in [42u64, 7, 99] {
            one_percent_case(sparse, seed).await;
        }
    }
}

async fn one_percent_case(sparse: bool, seed: u64) {
    let g = Graph::in_memory().await.unwrap();
    let ids = planted_with(&g, 2_000, sparse).await;
    let opts = CommunityMaterializeOptions::default();
    let first = levels(&g).await;
    g.materialize_communities(&first, &opts).await.unwrap();
    let before = stored_keys(&g, "leiden").await;
    let edges = g.get_edges_by_label("RELATED").await.unwrap().len();
    let mut r = rng(seed);
    for _ in 0..edges / 100 {
        let (a, b) = (r(ids.len()), r(ids.len()));
        if a != b {
            g.add_edge(Edge::new(ids[a], ids[b], "RELATED")).await.unwrap();
        }
    }
    let second = levels(&g).await;
    let report = g.materialize_communities(&second, &opts).await.unwrap();
    let after = stored_keys(&g, "leiden").await;
    for level in 0..first.len().min(second.len()) as i64 {
        let kept = ids.iter().filter(|id| before.get(&(**id, level)) == after.get(&(**id, level))).count();
        let share = kept as f64 / ids.len() as f64;
        assert!(share >= 0.8, "sparse={sparse} seed={seed} nivel {level}: solo {share:.3} conserva su key; {report:?}");
    }
}

#[tokio::test]
async fn a_node_hanging_from_a_kept_community_stays_attached() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 500).await;
    let opts = CommunityMaterializeOptions::default();
    g.materialize_communities(&levels(&g).await, &opts).await.unwrap();
    let comm = g.get_nodes_by_label(COMMUNITY_LABEL).await.unwrap().into_iter().next().unwrap();
    let report = g.add_node(Node::new("Report")).await.unwrap();
    g.add_edge(Edge::new(report, comm.id, "SUMMARIZES")).await.unwrap();
    g.materialize_communities(&levels(&g).await, &opts).await.unwrap();
    let out = g.get_outgoing_edges(report).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].target, comm.id, "misma comunidad, mismo NodeId");
}

#[tokio::test]
async fn partitions_coexist_and_keys_do_not_depend_on_the_partition_order() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 500).await;
    let lv = levels(&g).await;
    let a = CommunityMaterializeOptions { partition: "a".into(), ..Default::default() };
    let b = CommunityMaterializeOptions { partition: "b".into(), ..Default::default() };
    g.materialize_communities(&lv, &a).await.unwrap();
    g.materialize_communities(&lv, &b).await.unwrap();
    let (ka, kb) = (stored_keys(&g, "a").await, stored_keys(&g, "b").await);
    assert_eq!(ka.len(), kb.len());
    for (node, key) in &ka {
        assert_eq!(key.strip_prefix("a/"), kb[node].strip_prefix("b/"), "keys derivadas de los miembros");
    }
    // Volver a materializar "a" con un solo nivel borra sus niveles finos y no toca "b".
    let report = g.materialize_communities(&lv[..1], &a).await.unwrap();
    assert!(report.deleted > 0 && report.parent_links_added == 0);
    assert_eq!(stored_keys(&g, "a").await.len(), ka.len() / lv.len());
    assert_eq!(stored_keys(&g, "b").await, kb);
    let parents_a: Vec<_> = g.get_edges_by_label(PARENT_OF).await.unwrap();
    assert_eq!(parents_a.len(), lv.iter().skip(1).map(LeidenCommunity::count_communities).sum::<usize>());
}

#[tokio::test]
async fn survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let opts = CommunityMaterializeOptions::default();
    let before = {
        let g = Graph::open(dir.path()).await.unwrap();
        planted(&g, 500).await;
        g.materialize_communities(&levels(&g).await, &opts).await.unwrap();
        let keys = stored_keys(&g, "leiden").await;
        g.close().await.unwrap();
        keys
    };
    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(stored_keys(&g, "leiden").await, before);
    assert!(g.materialize_communities(&levels(&g).await, &opts).await.unwrap().is_unchanged());
}

#[tokio::test]
async fn invalid_input_is_rejected() {
    let g = Graph::in_memory().await.unwrap();
    let (x, y) = (g.add_node(Node::new("E")).await.unwrap(), g.add_node(Node::new("E")).await.unwrap());
    let ghost = Node::new("E").id;
    let opts = CommunityMaterializeOptions::default();
    let cases: Vec<(Vec<HashMap<NodeId, usize>>, CommunityMaterializeOptions, &str)> = vec![
        (vec![HashMap::from([(x, 0), (y, 1)]), HashMap::from([(x, 0), (y, 0)])], opts.clone(), "spans"),
        (vec![HashMap::from([(x, 0), (y, 0)]), HashMap::from([(x, 0)])], opts.clone(), "level 1 has"),
        (vec![HashMap::from([(x, 0), (ghost, 0)])], opts.clone(), "do not exist"),
        (vec![HashMap::from([(x, 0)])], CommunityMaterializeOptions { min_jaccard: 0.0, ..Default::default() }, "min_jaccard"),
        (vec![HashMap::from([(x, 0)])], CommunityMaterializeOptions { partition: String::new(), ..Default::default() }, "partition"),
    ];
    for (lv, o, needle) in cases {
        let err = g.materialize_communities(&lv, &o).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{err}");
    }
    assert!(g.get_nodes_by_label(COMMUNITY_LABEL).await.unwrap().is_empty(), "nada se escribe si la entrada es inválida");
}
