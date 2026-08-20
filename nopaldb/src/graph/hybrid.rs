// Hybrid search (issue M1-5): Reciprocal Rank Fusion of the full-text (tantivy)
// and vector (HNSW) retrieval paths, with an optional label/property filter.
//
// Both paths already exist; this fuses them. RRF is rank-based, which fits the
// full-text index (it returns node ids ordered by score, no raw scores) and the
// HNSW index (ordered by distance) uniformly:
//     score(d) = Σ_path 1 / (rrf_k + rank_path(d))
//
// The property/label filter is applied as a precomputed allowed-set (the HNSW
// filtered search takes a sync closure, so the set is built up front from the
// label scan + property index), then intersected into both paths.

use std::collections::{HashMap, HashSet};

use crate::error::{NopalError, Result};
use crate::index::{IndexQuery, IndexType};
use crate::types::{NodeId, PropertyValue};

use super::Graph;

/// HNSW `ef_search` default when the query does not override it — the
/// embeddings index default itself, so both entry points cannot drift.
#[cfg(feature = "hybrid")]
use crate::embeddings::index::DEFAULT_EF_SEARCH;
/// Exact-path pieces shared with the embeddings index: the same cosine
/// distance and tie-break, so both branches of `vector_path` rank identically.
#[cfg(feature = "hybrid")]
use crate::embeddings::{rank_exact, EXACT_SEARCH_THRESHOLD};

/// Equality-conjunction filter over a node's label and properties.
#[derive(Debug, Clone, Default)]
pub struct HybridFilter {
    pub label: Option<String>,
    pub props: Vec<(String, PropertyValue)>,
}

impl HybridFilter {
    pub fn is_empty(&self) -> bool {
        self.label.is_none() && self.props.is_empty()
    }
}

/// A hybrid search request. At least one of `text` / `vector` must be set.
#[derive(Debug, Clone)]
pub struct HybridQuery {
    /// Full-text query string (tantivy path).
    pub text: Option<String>,
    /// Full-text index name; auto-discovered when `None`.
    pub text_index: Option<String>,
    /// `(vector, model)` for the HNSW path.
    pub vector: Option<(Vec<f32>, String)>,
    /// Number of fused results to return.
    pub k: usize,
    /// HNSW `ef_search`; the index default is used when `None`.
    pub ef_search: Option<usize>,
    /// RRF constant (higher = flatter contribution from tail ranks).
    pub rrf_k: f32,
    /// Candidates fetched per path = `k * overfetch`.
    pub overfetch: usize,
    pub filter: Option<HybridFilter>,
}

impl HybridQuery {
    /// A query with sensible defaults (k=10, rrf_k=60, overfetch=4).
    pub fn new() -> Self {
        Self {
            text: None,
            text_index: None,
            vector: None,
            k: 10,
            ef_search: None,
            rrf_k: 60.0,
            overfetch: 4,
            filter: None,
        }
    }
}

impl Default for HybridQuery {
    fn default() -> Self {
        Self::new()
    }
}

/// Which branch resolved the vector path of a hybrid search.
///
/// Internal for now: it exists so the planner's choice is testable instead of
/// merely plausible — a filtered search that returns the right nodes says
/// nothing about *how* it got them, and the exact branch would rot unnoticed.
/// Surfacing it to callers belongs to the retrieval-explain work.
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorPath {
    /// No filter: plain KNN over the whole index.
    Unfiltered,
    /// Small allowed set over a large index: exact cosine over just those
    /// vectors, read from storage. Recall = 1 by construction.
    ExactOverAllowed,
    /// HNSW walk with the predicate applied natively during traversal.
    HnswFiltered,
}

/// One fused result. `score` is the RRF score (ordering only, not a probability).
/// `text_rank` / `vector_rank` are the 0-based positions in each path (None if the
/// node did not appear in that path).
#[derive(Debug, Clone)]
pub struct HybridHit {
    pub node_id: NodeId,
    pub score: f32,
    pub text_rank: Option<usize>,
    pub vector_rank: Option<usize>,
}

impl Graph {
    /// Fuse full-text and vector retrieval with Reciprocal Rank Fusion.
    #[cfg(feature = "hybrid")]
    pub async fn search_hybrid(&self, q: HybridQuery) -> Result<Vec<HybridHit>> {
        if q.text.is_none() && q.vector.is_none() {
            return Err(NopalError::Custom(
                "search_hybrid: provide at least one of `text` or `vector`".into(),
            ));
        }
        let candidates = q.k.saturating_mul(q.overfetch).max(q.k);

        // 1. Precompute the allowed-set from the filter (None = no restriction).
        let allowed = self.hybrid_allowed_set(q.filter.as_ref()).await?;

        // 2. Full-text path → node ids ranked by relevance.
        let mut text_ranks: HashMap<NodeId, usize> = HashMap::new();
        if let Some(text) = &q.text {
            let index_name = self.resolve_fulltext_index(q.text_index.as_deref(), q.filter.as_ref()).await?;
            let ids = self
                .index_manager
                .query(&index_name, &IndexQuery::FullText(text.clone()))
                .await?;
            for id in ids
                .into_iter()
                .filter(|id| allowed.as_ref().is_none_or(|s| s.contains(id)))
                .take(candidates)
            {
                let n = text_ranks.len();
                text_ranks.entry(id).or_insert(n);
            }
        }

        // 3. Vector path → node ids ranked by distance.
        let mut vector_ranks: HashMap<NodeId, usize> = HashMap::new();
        if let Some((vector, model)) = &q.vector {
            let index = self.get_or_build_embedding_index(model).await?;
            let ef = q.ef_search.unwrap_or(DEFAULT_EF_SEARCH);
            let (hits, _path) = self
                .vector_path(&index, vector, model, allowed.as_ref(), candidates, ef)
                .await?;
            for (rank, (id, _dist)) in hits.into_iter().enumerate() {
                vector_ranks.entry(id).or_insert(rank);
            }
        }

        // 4. RRF fusion over the union of both paths.
        let mut ids: HashSet<NodeId> = HashSet::new();
        ids.extend(text_ranks.keys().copied());
        ids.extend(vector_ranks.keys().copied());

        let mut hits: Vec<HybridHit> = ids
            .into_iter()
            .map(|id| {
                let tr = text_ranks.get(&id).copied();
                let vr = vector_ranks.get(&id).copied();
                let mut score = 0.0f32;
                if let Some(r) = tr {
                    score += 1.0 / (q.rrf_k + r as f32);
                }
                if let Some(r) = vr {
                    score += 1.0 / (q.rrf_k + r as f32);
                }
                HybridHit { node_id: id, score, text_rank: tr, vector_rank: vr }
            })
            .collect();

        // Sort by score desc; break ties by node id for determinism.
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        hits.truncate(q.k);
        Ok(hits)
    }

    /// Run the vector path, choosing between an exact scan over the allowed set
    /// and the filtered HNSW search.
    ///
    /// The choice is the point of this function. HNSW's filtered search applies
    /// the predicate while walking the graph, which is far better than fetching
    /// global neighbours and filtering after — but it is still approximate, and
    /// a selective predicate over a large index can walk a long way for a
    /// handful of allowed hits. When the allowed set is small enough to score
    /// directly, an exact scan over just those vectors is both *exact* and
    /// cheaper: at most `EXACT_SEARCH_THRESHOLD` point reads, no graph walk.
    ///
    /// Only the caller knows the allowed cardinality — the index sees an opaque
    /// closure — so the decision lives here, not in `HnswIndex`.
    ///
    /// The exact branch is skipped when the index itself is under the threshold:
    /// there `search_knn_filtered` is already exact over every point, without
    /// touching storage.
    #[cfg(feature = "hybrid")]
    pub(crate) async fn vector_path(
        &self,
        index: &crate::embeddings::HnswIndex,
        vector: &[f32],
        model: &str,
        allowed: Option<&HashSet<NodeId>>,
        candidates: usize,
        ef: usize,
    ) -> Result<(Vec<(NodeId, f32)>, VectorPath)> {
        let Some(set) = allowed else {
            let hits = index.search_knn_with_ef(vector, candidates, ef)?;
            return Ok((hits, VectorPath::Unfiltered));
        };

        if set.len() <= EXACT_SEARCH_THRESHOLD && index.len() > EXACT_SEARCH_THRESHOLD {
            let embeddings = self
                .storage
                .load_node_embeddings_for(set.iter().copied(), model)
                .await?;
            // Vectores de otra dimensión no son comparables; el índice ya
            // rechaza la query por dimensión, así que aquí solo pueden venir
            // de embeddings viejos del mismo modelo. Se omiten en vez de
            // envenenar el ranking con una distancia sin sentido.
            let dim = index.dimension();
            let hits = rank_exact(
                vector,
                embeddings
                    .iter()
                    .filter(|e| e.vector.len() == dim)
                    .map(|e| (e.node_id, e.vector.as_slice())),
                candidates,
            );
            return Ok((hits, VectorPath::ExactOverAllowed));
        }

        let hits = index.search_knn_filtered(vector, candidates, ef, |id| set.contains(id))?;
        Ok((hits, VectorPath::HnswFiltered))
    }

    /// Build the allowed NodeId set from a label/property filter. `None` means no
    /// restriction (empty filter or no filter). An empty set means nothing matches.
    #[cfg(feature = "hybrid")]
    async fn hybrid_allowed_set(
        &self,
        filter: Option<&HybridFilter>,
    ) -> Result<Option<HashSet<NodeId>>> {
        let Some(filter) = filter else { return Ok(None) };
        if filter.is_empty() {
            return Ok(None);
        }
        let mut set: Option<HashSet<NodeId>> = None;

        if let Some(label) = &filter.label {
            let by_label: HashSet<NodeId> = self
                .get_nodes_by_label(label)
                .await?
                .into_iter()
                .map(|n| n.id)
                .collect();
            set = Some(intersect(set, by_label));
        }
        for (prop, val) in &filter.props {
            let by_prop: HashSet<NodeId> = self
                .get_all_nodes_by_property(prop, val)
                .await?
                .into_iter()
                .collect();
            set = Some(intersect(set, by_prop));
        }
        Ok(set)
    }

    /// Resolve the full-text index to query: the caller-provided name, or the
    /// first `FullText` index (preferring one whose label matches the filter).
    #[cfg(feature = "hybrid")]
    async fn resolve_fulltext_index(
        &self,
        given: Option<&str>,
        filter: Option<&HybridFilter>,
    ) -> Result<String> {
        if let Some(name) = given {
            return Ok(name.to_string());
        }
        let metas = self.index_manager.list_indexes().await;
        let fulltext: Vec<_> = metas
            .into_iter()
            .filter(|m| m.index_type == IndexType::FullText)
            .collect();
        if fulltext.is_empty() {
            return Err(NopalError::index_error(
                "search_hybrid: no full-text index exists — create one with \
                 `create index on <Label>(<property>) type fulltext`"
                    .to_string(),
            ));
        }
        let wanted_label = filter.and_then(|f| f.label.as_deref());
        let chosen = wanted_label
            .and_then(|label| fulltext.iter().find(|m| m.label == label))
            .or_else(|| fulltext.first())
            .unwrap();
        Ok(chosen.name.clone())
    }
}

/// Intersect an optional running set with a new set.
#[cfg(feature = "hybrid")]
fn intersect(acc: Option<HashSet<NodeId>>, next: HashSet<NodeId>) -> HashSet<NodeId> {
    match acc {
        None => next,
        Some(cur) => cur.intersection(&next).copied().collect(),
    }
}

#[cfg(all(test, feature = "hybrid"))]
mod tests {
    use super::*;
    use crate::embeddings::EXACT_SEARCH_THRESHOLD;
    use crate::types::Node;

    /// Cosine distance computed independently of the engine, as ground truth.
    fn cosine_ref(a: &[f32], b: &[f32]) -> f32 {
        let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
        for (x, y) in a.iter().zip(b) {
            dot += (*x as f64) * (*y as f64);
            na += (*x as f64) * (*x as f64);
            nb += (*y as f64) * (*y as f64);
        }
        if na > 0.0 && nb > 0.0 {
            (1.0 - dot / (na * nb).sqrt()).max(0.0) as f32
        } else {
            0.0
        }
    }

    /// Deterministic pseudo-random vectors without pulling a RNG dependency
    /// into the lib tests: a cheap LCG is enough to spread points around.
    fn vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32 / (1u64 << 31) as f32) - 1.0
        };
        (0..n).map(|_| (0..dim).map(|_| next()).collect()).collect()
    }

    /// The planner must take the exact branch when the allowed set is small and
    /// the index is large — and that branch must rank exactly like brute force.
    ///
    /// Asserting only the returned nodes would not protect this: the filtered
    /// HNSW walk finds the same nodes at this scale, so the branch could be
    /// deleted and an outcome-only test would still pass.
    #[tokio::test]
    async fn small_allowed_set_takes_the_exact_branch_and_ranks_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();

        let dim = 8;
        let n = EXACT_SEARCH_THRESHOLD + 120;
        let vecs = vectors(n, dim, 4242);

        let mut allowed_ids: Vec<(NodeId, Vec<f32>)> = Vec::new();
        for (i, v) in vecs.iter().enumerate() {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
            if i % 50 == 0 {
                allowed_ids.push((id, v.clone()));
            }
        }

        let index = graph.get_or_build_embedding_index("m").await.unwrap();
        assert!(index.len() > EXACT_SEARCH_THRESHOLD, "índice por encima del umbral");
        assert!(allowed_ids.len() <= EXACT_SEARCH_THRESHOLD, "permitidos por debajo");

        let allowed: HashSet<NodeId> = allowed_ids.iter().map(|(id, _)| *id).collect();
        let query = &vecs[7];
        let k = 10;

        let (hits, path) = graph
            .vector_path(&index, query, "m", Some(&allowed), k, 30)
            .await
            .unwrap();

        assert_eq!(
            path,
            VectorPath::ExactOverAllowed,
            "conjunto permitido chico sobre índice grande ⇒ camino exacto"
        );

        let mut expected: Vec<(NodeId, f32)> = allowed_ids
            .iter()
            .map(|(id, v)| (*id, cosine_ref(query, v)))
            .collect();
        expected.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        expected.truncate(k);

        let got: Vec<NodeId> = hits.iter().map(|(id, _)| *id).collect();
        let want: Vec<NodeId> = expected.iter().map(|(id, _)| *id).collect();
        assert_eq!(got, want, "el camino exacto debe rankear como la fuerza bruta");
        for ((_, g), (_, e)) in hits.iter().zip(&expected) {
            assert!((g - e).abs() < 1e-5, "distancia difiere: {g} vs {e}");
        }
    }

    /// Sin filtro no hay conjunto permitido que acotar: se consulta el índice
    /// directamente, sin lecturas de storage.
    #[tokio::test]
    async fn no_filter_uses_the_plain_index_path() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();
        let dim = 4;
        for v in vectors(20, dim, 9) {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            graph.add_node_embedding(id, v, "m").await.unwrap();
        }
        let index = graph.get_or_build_embedding_index("m").await.unwrap();
        let (hits, path) = graph
            .vector_path(&index, &[1.0, 0.0, 0.0, 0.0], "m", None, 5, 30)
            .await
            .unwrap();
        assert_eq!(path, VectorPath::Unfiltered);
        assert_eq!(hits.len(), 5);
    }

    /// Un conjunto permitido GRANDE no se materializa: va por el grafo con el
    /// filtro nativo, y el resultado sigue sin contener nada fuera del filtro.
    #[tokio::test]
    async fn large_allowed_set_uses_the_native_filtered_walk() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();

        let dim = 8;
        let n = EXACT_SEARCH_THRESHOLD * 2;
        let vecs = vectors(n, dim, 77);
        let mut allowed: HashSet<NodeId> = HashSet::new();
        for (i, v) in vecs.iter().enumerate() {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
            // Deja fuera 1 de cada 4: permitido grande pero no total.
            if i % 4 != 0 {
                allowed.insert(id);
            }
        }

        let index = graph.get_or_build_embedding_index("m").await.unwrap();
        assert!(allowed.len() > EXACT_SEARCH_THRESHOLD, "permitidos por encima del umbral");

        let (hits, path) = graph
            .vector_path(&index, &vecs[3], "m", Some(&allowed), 10, 30)
            .await
            .unwrap();

        assert_eq!(path, VectorPath::HnswFiltered);
        assert!(
            hits.iter().all(|(id, _)| allowed.contains(id)),
            "el camino filtrado no debe devolver nada fuera del conjunto permitido"
        );
    }
}
