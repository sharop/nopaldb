// #191: reportes de comunidad y `stale_reports`.
//
// Jerarquía sembrada (bloques de 50 con subbloques de 10, como en
// `leiden_hierarchy_test`), materializada con `materialize_communities`.

use std::collections::{BTreeSet, HashMap, HashSet};

use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig, LeidenHierarchyOptions};
use nopaldb::graph::communities::{
    CommunityMaterializeOptions, CommunityReport, ReportStatus, StaleReport, COMMUNITY_LABEL, IN_COMMUNITY,
    PARENT_OF, REPORT_LABEL, SUMMARIZES,
};
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::Graph;

async fn planted(g: &Graph, n: usize) -> Vec<NodeId> {
    let mut ids = Vec::with_capacity(n);
    let mut loader = g.bulk_loader(50_000);
    for i in 0..n {
        let node = Node::new("Entity").with_property("name", format!("e{i}"));
        ids.push(node.id);
        loader.add_node(node).await.unwrap();
    }
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut r = |m: usize| {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % m
    };
    for i in 0..n {
        for j in (i + 1)..((i / 50 + 1) * 50).min(n) {
            if r(100) < if i / 10 == j / 10 { 80 } else { 15 } {
                loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
            }
        }
    }
    loader.finish().await.unwrap();
    ids
}

async fn materialize(g: &Graph, levels: Option<usize>) {
    let leiden = LeidenCommunity::new(LeidenConfig { labels: Some(vec!["Entity".into()]), ..Default::default() });
    let opts = LeidenHierarchyOptions { resolution_factor: 3.0, ..Default::default() };
    let mut lv = leiden.detect_hierarchy(g, &opts).await.unwrap().levels;
    if let Some(keep) = levels {
        lv.truncate(keep);
    }
    g.materialize_communities(&lv, &CommunityMaterializeOptions::default()).await.unwrap();
}

/// Community node → (key, level).
async fn communities(g: &Graph) -> HashMap<NodeId, (String, i64)> {
    g.get_nodes_by_label(COMMUNITY_LABEL)
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.id, (c.properties["key"].as_str().unwrap().to_string(), c.properties["level"].as_i64().unwrap())))
        .collect()
}

/// Un "LLM" falso: resume con los nombres de los miembros.
async fn write_all_reports(g: &Graph) {
    for (id, (key, level)) in communities(g).await {
        let members = g.get_incoming_edges(id).await.unwrap().into_iter().filter(|e| e.edge_type == IN_COMMUNITY).count();
        let report = CommunityReport {
            title: format!("community {key}"),
            summary: format!("level {level}, {members} members"),
            rating: Some(5.0),
            embedding: None,
        };
        g.upsert_community_report(&key, report).await.unwrap();
    }
}

fn keys(stale: &[StaleReport], status: ReportStatus) -> BTreeSet<String> {
    stale.iter().filter(|s| s.status == status).map(|s| s.community_key.clone()).collect()
}

#[tokio::test]
async fn reports_follow_the_convention_and_are_fresh_after_writing() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 500).await;
    materialize(&g, None).await;
    let comms = communities(&g).await;
    let all: BTreeSet<String> = comms.values().map(|(k, _)| k.clone()).collect();
    let stale = g.stale_reports("leiden", None).await.unwrap();
    assert_eq!(keys(&stale, ReportStatus::Missing), all, "sin reportes, todas faltan");
    write_all_reports(&g).await;
    assert!(g.stale_reports("leiden", None).await.unwrap().is_empty());
    // Convención: un Report por comunidad, con SUMMARIZES hacia ella.
    let reports = g.get_nodes_by_label(REPORT_LABEL).await.unwrap();
    assert_eq!(reports.len(), comms.len());
    for r in &reports {
        let out = g.get_outgoing_edges(r.id).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].edge_type, SUMMARIZES);
        let (key, level) = &comms[&out[0].target];
        assert_eq!(r.properties["community_key"].as_str(), Some(key.as_str()));
        assert_eq!(r.properties["level"].as_i64(), Some(*level));
        assert_eq!(r.properties["partition"].as_str(), Some("leiden"));
        assert_eq!(
            r.properties["source_version"].as_str().unwrap(),
            g.community_fingerprint(key).await.unwrap()
        );
        assert!(r.properties["generated_at"].as_i64().unwrap() > 0);
    }
    // Reescribir no duplica.
    write_all_reports(&g).await;
    assert_eq!(g.get_nodes_by_label(REPORT_LABEL).await.unwrap().len(), comms.len());
}

#[tokio::test]
async fn an_internal_edge_marks_only_its_community_and_its_ancestors() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 500).await;
    materialize(&g, None).await;
    write_all_reports(&g).await;
    let comms = communities(&g).await;
    let deepest = comms.values().map(|(_, l)| *l).max().unwrap();
    assert!(deepest >= 1, "hace falta al menos un nivel fino");
    // Una comunidad del nivel más fino con ≥ 2 miembros.
    let (target, members) = {
        let mut found = None;
        for (id, (_, level)) in &comms {
            if *level != deepest {
                continue;
            }
            let m: Vec<NodeId> = g.get_incoming_edges(*id).await.unwrap().into_iter()
                .filter(|e| e.edge_type == IN_COMMUNITY).map(|e| e.source).collect();
            if m.len() >= 2 {
                found = Some((*id, m));
                break;
            }
        }
        found.unwrap()
    };
    // Ancestros por PARENT_OF (padre → hijo).
    let parent_of: HashMap<NodeId, NodeId> =
        g.get_edges_by_label(PARENT_OF).await.unwrap().into_iter().map(|e| (e.target, e.source)).collect();
    let mut expected = BTreeSet::from([comms[&target].0.clone()]);
    let mut cur = target;
    while let Some(p) = parent_of.get(&cur) {
        expected.insert(comms[p].0.clone());
        cur = *p;
    }
    assert_eq!(expected.len() as i64, deepest + 1, "una comunidad por nivel");
    // Arista nueva dentro de la comunidad, sin recalcular la partición.
    g.add_edge(Edge::new(members[0], members[1], "RELATED").with_property("note", "new")).await.unwrap();
    let stale = g.stale_reports("leiden", None).await.unwrap();
    assert_eq!(keys(&stale, ReportStatus::Stale), expected);
    assert!(keys(&stale, ReportStatus::Missing).is_empty() && keys(&stale, ReportStatus::Orphan).is_empty());
    // Filtrar por nivel.
    let only0 = g.stale_reports("leiden", Some(0)).await.unwrap();
    assert_eq!(only0.len(), 1);
    assert_eq!(only0[0].level, 0);
    // Reescribir el reporte de esas comunidades las deja al día.
    for key in &expected {
        g.upsert_community_report(key, CommunityReport { title: "t".into(), summary: "s".into(), ..Default::default() })
            .await.unwrap();
    }
    assert!(g.stale_reports("leiden", None).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_member_property_change_marks_one_community_per_level() {
    let g = Graph::in_memory().await.unwrap();
    let ids = planted(&g, 500).await;
    materialize(&g, None).await;
    write_all_reports(&g).await;
    let depth = communities(&g).await.values().map(|(_, l)| *l).max().unwrap() + 1;
    let mut node = g.get_node(ids[0]).await.unwrap();
    node.properties.insert("name".into(), PropertyValue::String("renamed".into()));
    g.add_node(node).await.unwrap();
    let stale = g.stale_reports("leiden", None).await.unwrap();
    assert_eq!(stale.len() as i64, depth);
    let levels: HashSet<usize> = stale.iter().map(|s| s.level).collect();
    assert_eq!(levels.len() as i64, depth, "una por nivel");
}

#[tokio::test]
async fn reports_of_deleted_communities_are_orphans() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 500).await;
    materialize(&g, None).await;
    write_all_reports(&g).await;
    let comms = communities(&g).await;
    let finer: BTreeSet<String> = comms.values().filter(|(_, l)| *l > 0).map(|(k, _)| k.clone()).collect();
    assert!(!finer.is_empty());
    materialize(&g, Some(1)).await; // borra los niveles finos
    let stale = g.stale_reports("leiden", None).await.unwrap();
    assert_eq!(keys(&stale, ReportStatus::Orphan), finer);
    assert!(stale.iter().filter(|s| s.status == ReportStatus::Orphan).all(|s| s.community.is_none() && s.report.is_some()));
    assert!(keys(&stale, ReportStatus::Stale).is_empty());
}

#[tokio::test]
async fn unknown_community_and_other_partitions() {
    let g = Graph::in_memory().await.unwrap();
    planted(&g, 200).await;
    materialize(&g, None).await;
    let err = g
        .upsert_community_report("leiden/L0/nope", CommunityReport::default())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not found"), "{err}");
    assert!(g.community_fingerprint("nope").await.is_err());
    assert!(g.stale_reports("other", None).await.unwrap().is_empty(), "otra partición, nada");
}
