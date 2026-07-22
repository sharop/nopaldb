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

/// HNSW `ef_search` default when the query does not override it (mirrors the
/// embeddings index default).
#[cfg(feature = "hybrid")]
const DEFAULT_EF_SEARCH: usize = 30;

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
            let hits = match &allowed {
                Some(set) => index.search_knn_filtered(vector, candidates, ef, |id| set.contains(id))?,
                None => index.search_knn_with_ef(vector, candidates, ef)?,
            };
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
