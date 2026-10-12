// Filtro selectivo sobre la rama de texto (#213): el único documento
// permitido que contiene el término debe salir aunque haya más de 1000
// documentos ajenos que también lo contengan (reporte externo, 0.6.9 y
// 0.6.12).
//
// La rama de texto pedía a tantivy el top-N GLOBAL (tope 1000) y filtraba
// después; con un filtro que deja 1 de 1024 el permitido quedaba fuera del
// tope y la búsqueda devolvía 0. La rama vectorial ya evitaba este diseño
// (ver `search_knn_filtered_explained`: "k × 4 globales y filtrar después").
// Ahora la rama elige camino por el tamaño del conjunto permitido
// (`TextPath`) y es completa a cualquier cardinalidad.

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery, TextPath};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// `allowed` nodos del tenant B, cada uno con un cuerpo MÁS LARGO que los
/// `noise` del tenant A ⇒ BM25 deja a todos los permitidos detrás del ruido.
async fn fixture(allowed: usize, noise: usize) -> (Graph, tempfile::TempDir, Vec<NodeId>) {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let mut allowed_ids = Vec::with_capacity(allowed);
    for i in 0..allowed {
        let body = format!("fragmento unico {}", "relleno ".repeat(i % 7 + 1));
        let id = graph
            .add_node(
                Node::new("Fragment")
                    .with_property("key", s(&format!("allowed-{i}")))
                    .with_property("tenant_id", s("B"))
                    .with_property("case_id", s("b"))
                    .with_property("body", s(&body)),
            )
            .await
            .unwrap();
        allowed_ids.push(id);
    }
    for i in 0..noise {
        graph
            .add_node(
                Node::new("Fragment")
                    .with_property("key", s(&format!("noise-{i}")))
                    .with_property("tenant_id", s("A"))
                    .with_property("case_id", s("a"))
                    .with_property("body", s("fragmento")),
            )
            .await
            .unwrap();
    }
    graph.create_index("Fragment", "body", IndexType::FullText).await.unwrap();
    (graph, dir, allowed_ids)
}

fn tenant_b(k: usize) -> HybridQuery {
    let mut q = HybridQuery::new();
    q.text = Some("fragmento".into());
    q.k = k;
    q.filter = Some(HybridFilter {
        label: Some("Fragment".into()),
        props: vec![("tenant_id".into(), s("B")), ("case_id".into(), s("b"))],
    });
    q
}

/// El caso del reporte: 1 permitido entre 1024 que contienen el término.
#[tokio::test]
async fn selective_filter_finds_the_only_allowed_text_hit_past_the_global_cap() {
    let (graph, _dir, allowed) = fixture(1, 1023).await;

    let explain = graph.search_hybrid_explain(tenant_b(8)).await.unwrap();
    assert_eq!(explain.allowed_set_size, Some(1), "el filtro deja exactamente uno");
    assert_eq!(explain.text_path, Some(TextPath::TermSetOverAllowed));
    let text = explain.text.expect("rama de texto");
    assert_eq!(text.returned, 1, "la rama de texto devuelve el permitido");
    assert!(text.underfilled(), "y underfilled ahora significa 'no hay más permitidos'");

    let hits = graph.search_hybrid(tenant_b(8)).await.unwrap();
    assert_eq!(hits.len(), 1, "un solo hit, el del ámbito B/b");
    assert_eq!(hits[0].node_id, allowed[0]);
    assert_eq!(hits[0].text_rank, Some(0));
}

/// Control: por debajo del tope global el mismo caso ya funcionaba y sigue
/// devolviendo lo mismo.
#[tokio::test]
async fn selective_filter_below_the_global_cap_still_works() {
    let (graph, _dir, allowed) = fixture(1, 100).await;
    let hits = graph.search_hybrid(tenant_b(8)).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, allowed[0]);
}

/// Conjunto permitido por encima de `TEXT_EXACT_THRESHOLD`, todo detrás del
/// tope global: el escalado se agota corto y la pasada final sobre el
/// conjunto completa los candidatos. Nada fuera del conjunto entra.
#[tokio::test]
async fn large_allowed_set_past_the_cap_falls_back_to_the_term_set() {
    let allowed_n = nopaldb::graph::hybrid::TEXT_EXACT_THRESHOLD + 200;
    let (graph, _dir, allowed) = fixture(allowed_n, 1100).await;
    let allowed_set: std::collections::HashSet<NodeId> = allowed.iter().copied().collect();

    let q = tenant_b(8);
    let candidates = q.k * q.overfetch;
    let explain = graph.search_hybrid_explain(q.clone()).await.unwrap();
    assert_eq!(explain.allowed_set_size, Some(allowed_n));
    assert_eq!(explain.text_path, Some(TextPath::GlobalThenTermSet));
    let text = explain.text.unwrap();
    assert_eq!(text.returned, candidates, "la rama llena sus candidatos");
    assert!(!text.underfilled());

    let hits = graph.search_hybrid(q).await.unwrap();
    assert_eq!(hits.len(), 8);
    assert!(hits.iter().all(|h| allowed_set.contains(&h.node_id)), "nada fuera del conjunto");
    // Mejor primero: los permitidos más cortos (menos relleno) puntúan más.
    let scores: Vec<f32> = explain.hits.iter().map(|h| h.text_score.unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "orden BM25 descendente: {scores:?}");
}

/// Con filtro, cada hit trae el MISMO score BM25 que sin filtro, y el orden
/// es el global restringido al conjunto: el term set no altera el ranking,
/// solo lo acota (el `TermSetQuery` va con boost 0).
#[tokio::test]
async fn filtered_scores_and_order_equal_the_global_ranking_restricted() {
    // Pocos documentos: sin filtro el top-N global alcanza a todos.
    let (graph, _dir, allowed) = fixture(20, 30).await;
    let allowed_set: std::collections::HashSet<NodeId> = allowed.iter().copied().collect();

    let mut global = HybridQuery::new();
    global.text = Some("fragmento".into());
    global.k = 50;
    global.overfetch = 1;
    let global = graph.search_hybrid_explain(global).await.unwrap();
    assert_eq!(global.text_path, Some(TextPath::Global));
    assert_eq!(global.hits.len(), 50);
    let expected: Vec<(NodeId, f32)> = global
        .hits
        .iter()
        .filter(|h| allowed_set.contains(&h.node_id))
        .map(|h| (h.node_id, h.text_score.unwrap()))
        .collect();
    assert_eq!(expected.len(), 20);

    let mut filtered = tenant_b(20);
    filtered.overfetch = 1;
    let filtered = graph.search_hybrid_explain(filtered).await.unwrap();
    assert_eq!(filtered.text_path, Some(TextPath::TermSetOverAllowed));
    let got: Vec<(NodeId, f32)> = filtered.hits.iter().map(|h| (h.node_id, h.text_score.unwrap())).collect();
    assert_eq!(got.len(), expected.len());
    for ((gid, gscore), (eid, escore)) in got.iter().zip(&expected) {
        assert_eq!(gid, eid, "mismo orden que el ranking global restringido");
        assert!((gscore - escore).abs() < 1e-6, "mismo score BM25: {gscore} vs {escore}");
    }
}

/// El filtro de solo etiqueta y la consulta sin filtro siguen por el camino
/// global, y `explain` lo dice.
#[tokio::test]
async fn label_only_and_unfiltered_keep_the_global_path() {
    let (graph, _dir, _allowed) = fixture(1, 50).await;

    let mut q = HybridQuery::new();
    q.text = Some("fragmento".into());
    q.k = 5;
    let e = graph.search_hybrid_explain(q.clone()).await.unwrap();
    assert_eq!(e.text_path, Some(TextPath::Global));
    assert_eq!(e.hits.len(), 5);

    q.filter = Some(HybridFilter { label: Some("Fragment".into()), props: vec![] });
    let e = graph.search_hybrid_explain(q).await.unwrap();
    assert_eq!(e.text_path, Some(TextPath::GlobalLabelChecked));
    assert_eq!(e.allowed_set_size, None, "la etiqueta se comprueba sin construir el conjunto");
    assert_eq!(e.hits.len(), 5);
}

/// Texto + vector con filtro selectivo: el hit entra con evidencia de las
/// DOS ramas, no solo de la vectorial.
#[tokio::test]
async fn hybrid_text_and_vector_both_rank_the_allowed_hit() {
    let (graph, _dir, allowed) = fixture(1, 1023).await;
    graph.add_node_embedding(allowed[0], vec![1.0, 0.0, 0.0], "m").await.unwrap();
    let mut q = tenant_b(8);
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    let hits = graph.search_hybrid(q).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text_rank, Some(0), "rango de texto");
    assert_eq!(hits[0].vector_rank, Some(0), "rango vectorial");
}
