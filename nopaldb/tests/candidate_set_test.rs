// CandidateSet (#214, fase 1): el universo explícito del caller en
// `search_hybrid`. Dos invariantes bajo prueba:
//
//   1. cambia el universo puntuable, nunca el score: `top-k(Q, C)` con los
//      mismos BM25 y distancias que sin conjunto;
//   2. nunca se amplía: `{label, props}` solo mantiene o reduce el conjunto
//      del caller, y todo resultado pertenece a él.
//
// El caso guía es el de un integrador: 65 fragmentos del mismo tenant/caso,
// el único del documento seleccionado detrás de 64 distractores más
// cercanos. Pedir 32 dentro del filtro y quedarse con el documento después
// devolvía 0; con el documento como conjunto de candidatos devuelve 1, con
// rango en las dos ramas.

use std::collections::HashSet;
use std::sync::OnceLock;

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{CandidateSet, CandidateSource, Graph, HybridFilter, HybridQuery, TextPath, VectorPath};
use proptest::prelude::*;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn add(graph: &Graph, tenant: &str, doc: &str, body: &str, vector: Vec<f32>) -> NodeId {
    let id = graph
        .add_node(
            Node::new("Fragment")
                .with_property("tenant_id", s(tenant))
                .with_property("case_id", s(&format!("case-{tenant}")))
                .with_property("doc_id", s(doc))
                .with_property("body", s(body)),
        )
        .await
        .unwrap();
    graph.add_node_embedding(id, vector, "m").await.unwrap();
    id
}

/// 65 fragmentos del tenant B: 64 distractores (cuerpo corto, vector casi
/// igual a la query) y 1 objetivo (cuerpo largo ⇒ último en BM25; vector
/// más lejano que todos los distractores). Más 10 del tenant A.
async fn fixture() -> (Graph, NodeId, Vec<NodeId>, Vec<NodeId>) {
    let graph = Graph::in_memory().await.unwrap();
    let mut distractors = Vec::new();
    for i in 0..64u32 {
        let v = vec![1.0, 0.001 * i as f32, 0.0];
        distractors.push(add(&graph, "B", &format!("doc-{}", i % 8), "fragmento", v).await);
    }
    let target = add(
        &graph,
        "B",
        "doc-target",
        &format!("fragmento {}", "relleno ".repeat(20)),
        vec![0.6, 0.8, 0.0],
    )
    .await;
    let mut others = Vec::new();
    for i in 0..10u32 {
        others.push(add(&graph, "A", "doc-a", "fragmento", vec![1.0, 0.0, 0.0005 * i as f32]).await);
    }
    graph.create_index("Fragment", "body", IndexType::FullText).await.unwrap();
    (graph, target, distractors, others)
}

fn tenant_b() -> HybridFilter {
    HybridFilter {
        label: Some("Fragment".into()),
        props: vec![("tenant_id".into(), s("B"))],
    }
}

fn both(k: usize) -> HybridQuery {
    let mut q = HybridQuery::new();
    q.text = Some("fragmento".into());
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.k = k;
    q
}

/// El caso del integrador: filtrar el documento DESPUÉS de los 32 candidatos
/// pierde el fragmento; darlo como conjunto de candidatos lo encuentra con
/// rango en ambas ramas.
#[tokio::test]
async fn one_target_behind_64_distractors_is_found_with_the_document_as_candidate_set() {
    let (graph, target, _, _) = fixture().await;

    // Como lo hacía el integrador: top-32 dentro de tenant ∩ caso, filtrar
    // el documento después ⇒ el objetivo no está.
    let mut q = both(8);
    q.filter = Some(tenant_b());
    let after = graph.search_hybrid_explain(q).await.unwrap();
    assert_eq!(after.candidates, 32);
    assert!(
        !after.hits.iter().any(|h| h.node_id == target),
        "detrás de 64 distractores en las dos ramas, el objetivo queda fuera del top-32"
    );

    // Con el documento como universo.
    let mut q = both(8);
    q.filter = Some(tenant_b());
    q.candidates = CandidateSet::ids([target]);
    let explain = graph.search_hybrid_explain(q.clone()).await.unwrap();
    let ids: Vec<NodeId> = explain.hits.iter().map(|h| h.node_id).collect();
    assert_eq!(ids, vec![target]);
    let hit = &explain.hits[0];
    assert_eq!(hit.text_rank, Some(0), "rango de texto dentro del conjunto");
    assert_eq!(hit.vector_rank, Some(0), "rango vectorial dentro del conjunto");
    assert!(hit.text_score.is_some() && hit.vector_distance.is_some());

    assert_eq!(explain.candidate_set.source, CandidateSource::IdsAndFilter);
    assert_eq!(explain.candidate_set.caller_size, Some(1));
    assert_eq!(explain.candidate_set.size, Some(1));
    assert_eq!(explain.allowed_set_size, Some(1), "compatibilidad: mismo valor");
    assert_eq!(explain.text_path, Some(TextPath::TermSetOverAllowed));
    assert_eq!(explain.vector_path, Some(VectorPath::ExactIndex), "índice de 75 vectores ⇒ exacto");
    assert_eq!(explain.vector_exact, Some(true));
    let text = explain.text.unwrap();
    assert_eq!((text.returned, text.underfilled()), (1, true), "no hay más en el universo");

    let hits = graph.search_hybrid(q).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, target);
}

/// Invariante 2: el filtro solo reduce el conjunto del caller. Ids de otro
/// tenant y un id inexistente salen al construir el conjunto, y explain
/// deja ver que fue ahí (size < caller_size), no en el ranking.
#[tokio::test]
async fn a_filter_only_narrows_the_callers_set() {
    let (graph, target, distractors, others) = fixture().await;
    let mut given: Vec<NodeId> = others.clone();
    given.push(target);
    given.extend_from_slice(&distractors[..4]);
    given.push(NodeId::new_v4());

    let mut q = both(20);
    q.filter = Some(tenant_b());
    q.candidates = CandidateSet::ids(given.iter().copied());
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    let allowed: HashSet<NodeId> = [target].into_iter().chain(distractors[..4].iter().copied()).collect();
    let got: HashSet<NodeId> = explain.hits.iter().map(|h| h.node_id).collect();
    assert_eq!(got, allowed, "exactamente el conjunto del caller ∩ tenant B");
    assert_eq!(explain.candidate_set.caller_size, Some(16));
    assert_eq!(explain.candidate_set.size, Some(5), "10 del tenant A y 1 inexistente fuera al construir");
    assert_eq!(explain.candidate_set.source, CandidateSource::IdsAndFilter);
}

/// Sin filtro los ids se toman tal cual, sin leerlos: un id inexistente
/// cuenta en el tamaño y simplemente no puntúa.
#[tokio::test]
async fn ids_without_a_filter_are_taken_as_given() {
    let (graph, target, distractors, _) = fixture().await;
    let ghost = NodeId::new_v4();
    let mut q = both(10);
    q.candidates = CandidateSet::ids([target, distractors[0], distractors[1], ghost]);
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    let got: HashSet<NodeId> = explain.hits.iter().map(|h| h.node_id).collect();
    assert_eq!(got, [target, distractors[0], distractors[1]].into_iter().collect());
    assert_eq!(explain.candidate_set.source, CandidateSource::Ids);
    assert_eq!(explain.candidate_set.caller_size, Some(4));
    assert_eq!(explain.candidate_set.size, Some(4), "no se hidrata: el fantasma sigue contando");
    assert_eq!(explain.text_path, Some(TextPath::TermSetOverAllowed));
}

/// Un conjunto vacío no puntúa nada, y lo dice.
#[tokio::test]
async fn an_empty_candidate_set_scores_nothing() {
    let (graph, _, _, _) = fixture().await;
    let mut q = both(5);
    q.candidates = CandidateSet::ids([]);
    let explain = graph.search_hybrid_explain(q).await.unwrap();
    assert!(explain.hits.is_empty());
    assert_eq!(explain.candidate_set.size, Some(0));
    assert_eq!(explain.text.unwrap().returned, 0);
    assert_eq!(explain.vector.unwrap().returned, 0);
}

/// Invariante 1: el conjunto no toca el score. Un distractor que sí entra en
/// el top-k global lleva el mismo BM25 y la misma distancia cuando es el
/// único candidato.
#[tokio::test]
async fn scores_inside_a_candidate_set_equal_the_unrestricted_ones() {
    let (graph, _, distractors, _) = fixture().await;
    let pick = distractors[3];

    let global = graph.search_hybrid_explain(both(80)).await.unwrap();
    let g = global.hits.iter().find(|h| h.node_id == pick).expect("en el top global");

    let mut q = both(5);
    q.candidates = CandidateSet::ids([pick]);
    let within = graph.search_hybrid_explain(q).await.unwrap();
    let w = &within.hits[0];
    assert_eq!(w.node_id, pick);
    assert_eq!(w.text_score, g.text_score, "mismo BM25 (IDF de todo el índice en ambos caminos)");
    let (a, b) = (w.vector_distance.unwrap(), g.vector_distance.unwrap());
    assert!((a - b).abs() < 1e-6, "misma distancia: {a} vs {b}");
}

/// Con el conjunto del caller, un filtro de solo etiqueta no toma el atajo
/// (hay conjunto): la etiqueta se confirma sobre los ids.
#[tokio::test]
async fn label_only_filter_with_candidates_confirms_the_label_on_the_set() {
    let (graph, target, _, _) = fixture().await;
    let other = graph.add_node(Node::new("Note").with_property("body", s("fragmento"))).await.unwrap();
    graph.add_node_embedding(other, vec![1.0, 0.0, 0.0], "m").await.unwrap();

    let mut q = both(5);
    q.filter = Some(HybridFilter { label: Some("Fragment".into()), props: vec![] });
    q.candidates = CandidateSet::ids([target, other]);
    let explain = graph.search_hybrid_explain(q).await.unwrap();
    let got: Vec<NodeId> = explain.hits.iter().map(|h| h.node_id).collect();
    assert_eq!(got, vec![target], "la Note sale al confirmar la etiqueta");
    assert_eq!(explain.candidate_set.size, Some(1));
    assert_ne!(explain.vector_path, Some(VectorPath::LabelChecked), "sin atajo: hay conjunto");
}

// ---- propiedades --------------------------------------------------------

struct Corpus {
    rt: tokio::runtime::Runtime,
    graph: Graph,
    ids: Vec<NodeId>,
}

/// 60 Doc que todos contienen el término y todos tienen vector: cualquier
/// subconjunto C tiene exactamente |C| coincidencias en cada rama.
fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (graph, ids) = rt.block_on(async {
            let graph = Graph::in_memory().await.unwrap();
            let mut ids = Vec::new();
            for i in 0..60u32 {
                let angle = i as f32 * 0.1;
                let body = format!("comun {}", "extra ".repeat((i % 5) as usize));
                let id = graph
                    .add_node(Node::new("Doc").with_property("body", s(&body)))
                    .await
                    .unwrap();
                graph.add_node_embedding(id, vec![angle.cos(), angle.sin(), 0.1], "m").await.unwrap();
                ids.push(id);
            }
            graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();
            (graph, ids)
        });
        Corpus { rt, graph, ids }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// ∀ C, k, modo: todo resultado ∈ C, y |resultados| = min(k, |C|) porque
    /// todo el corpus coincide en las dos ramas.
    #[test]
    fn results_lie_in_the_set_and_fill_k_when_the_set_allows(
        mask in proptest::collection::vec(any::<bool>(), 60),
        k in 1usize..=12,
        mode in 0u8..3,
    ) {
        let c = corpus();
        let chosen: HashSet<NodeId> = c.ids.iter().zip(&mask).filter(|(_, m)| **m).map(|(id, _)| *id).collect();
        let mut q = HybridQuery::new();
        if mode != 1 { q.text = Some("comun".into()); }
        if mode != 0 { q.vector = Some((vec![1.0, 0.0, 0.0], "m".into())); }
        q.k = k;
        q.candidates = CandidateSet::ids(chosen.iter().copied());
        let hits = c.rt.block_on(c.graph.search_hybrid(q)).unwrap();
        prop_assert!(hits.iter().all(|h| chosen.contains(&h.node_id)), "resultado fuera del conjunto");
        prop_assert_eq!(hits.len(), k.min(chosen.len()));
        let distinct: HashSet<NodeId> = hits.iter().map(|h| h.node_id).collect();
        prop_assert_eq!(distinct.len(), hits.len(), "sin duplicados");
    }
}
