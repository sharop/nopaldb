// Hybrid search (issue M1-5): Reciprocal Rank Fusion of the full-text (tantivy)
// and vector (HNSW) retrieval paths, with an optional label/property filter.
//
// Both paths already exist; this fuses them. RRF is rank-based, which fits the
// full-text index (it returns node ids ordered by score, no raw scores) and the
// HNSW index (ordered by distance) uniformly:
//     score(d) = Σ_path 1 / (rrf_k + rank_path(d))
//
// A filter with properties is applied as a precomputed allowed-set (the HNSW
// filtered search takes a sync closure, so the set is built up front from the
// label scan + property index), then intersected into both paths. A filter
// with only a label skips the set: each path checks the label on the
// candidates it reads, in rank order (see `search_hybrid_explain`).

use std::collections::{HashMap, HashSet};

use crate::error::Result;
#[cfg(feature = "hybrid")]
use crate::error::NopalError;
#[cfg(feature = "hybrid")]
use crate::index::{IndexQuery, IndexType};
use crate::types::{NodeId, PropertyValue};
#[cfg(feature = "embeddings-index")]
use crate::types::Node;

use super::candidate_set::CandidateSet;
use super::Graph;

/// HNSW `ef_search` default when the query does not override it — the
/// embeddings index default itself, so both entry points cannot drift.
#[cfg(feature = "hybrid")]
use crate::embeddings::index::DEFAULT_EF_SEARCH;
/// Exact-path pieces shared with the embeddings index: the same cosine
/// distance and tie-break, so both branches of `vector_path` rank identically.
#[cfg(feature = "embeddings-index")]
use crate::embeddings::{exact_over_allowed_limit, rank_exact, FilteredSearchPath, EXACT_SEARCH_THRESHOLD};

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

/// What a node must satisfy to belong to a search's universe: the
/// `{label, props}` filter plus `prop ∈ [values]` lists (what NQL pushes from
/// `c.p in [...]`, #214 phase 2). Crate-internal on purpose: [`HybridFilter`]
/// has public fields, and growing it would break every caller that builds it
/// with a struct literal.
///
/// It is checked on hydrated nodes with the same equality the NQL evaluator
/// uses (`PropertyValue`'s `PartialEq`, typed: `Int(1)` is not
/// `Float(1.0)`), so a universe built here never disagrees with the `where`
/// that produced it. The property index only *proposes* members; a member
/// is confirmed here whenever the node is read anyway.
#[cfg(feature = "embeddings-index")]
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeFilter {
    pub label: Option<String>,
    pub eq: Vec<(String, PropertyValue)>,
    pub any_of: Vec<(String, Vec<PropertyValue>)>,
}

#[cfg(feature = "embeddings-index")]
impl NodeFilter {
    /// Only `search_hybrid` builds a filter from a `HybridFilter`.
    #[cfg(feature = "hybrid")]
    pub(crate) fn new(filter: Option<&HybridFilter>, any_of: Vec<(String, Vec<PropertyValue>)>) -> Self {
        let (label, eq) = match filter {
            Some(f) => (f.label.clone(), f.props.clone()),
            None => (None, Vec::new()),
        };
        NodeFilter { label, eq, any_of }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.label.is_none() && !self.has_props()
    }

    /// Anything beyond the label. A label-only filter keeps the exact
    /// behaviour it had before #214 phase 2 (same paths, same costs).
    pub(crate) fn has_props(&self) -> bool {
        !self.eq.is_empty() || !self.any_of.is_empty()
    }

    pub(crate) fn matches(&self, node: &Node) -> bool {
        self.label.as_deref().is_none_or(|l| l == node.label)
            && self.eq.iter().all(|(p, v)| node.properties.get(p) == Some(v))
            && self
                .any_of
                .iter()
                .all(|(p, vs)| node.properties.get(p).is_some_and(|x| vs.contains(x)))
    }
}

/// Whether the storage property index can answer `prop = value`: it keys
/// scalars only, and property names that fit in a `u16` length
/// (`encode_property_index_key`). Anything else must never seed a set from
/// the index — the lookup comes back empty and every member would be lost.
#[cfg(feature = "embeddings-index")]
fn indexable(prop: &str, value: &PropertyValue) -> bool {
    u16::try_from(prop.len()).is_ok()
        && matches!(
            value,
            PropertyValue::Null
                | PropertyValue::Bool(_)
                | PropertyValue::Int(_)
                | PropertyValue::Float(_)
                | PropertyValue::String(_)
        )
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
    /// The universe the search runs in (#214). `All` (the default) is the
    /// whole graph; an explicit set is only ever *reduced* by `filter`, never
    /// widened, and both paths compute their top-k inside it. See
    /// [`CandidateSet`] for the two invariants.
    pub candidates: CandidateSet,
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
            candidates: CandidateSet::All,
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
/// Matters to a caller because the three differ in *recall*, not just in
/// speed: the first two are exact, the third is approximate. A short result
/// means something different in each.
#[cfg(feature = "embeddings-index")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorPath {
    /// No filter: plain KNN over the whole index.
    Unfiltered,
    /// Allowed set small for its index ([`exact_over_allowed_limit`]): exact
    /// cosine over just those vectors, read from the index. Recall = 1 by
    /// construction.
    ExactOverAllowed,
    /// Filter over an index that is itself under `EXACT_SEARCH_THRESHOLD`:
    /// the index scans every point exactly and applies the predicate, no
    /// graph walk, no storage read. Recall = 1 by construction. Until 0.6.14
    /// this was reported as `HnswFiltered` (#218), which told a caller that
    /// a short result might be incomplete when it never was.
    ExactIndex,
    /// HNSW walk with the predicate applied natively during traversal.
    /// Approximate: a short result may mean "found no more", not "there are
    /// no more".
    HnswFiltered,
    /// Filter checked on the candidates: unfiltered KNN, the filter checked
    /// on the hydrated candidates in distance order, fetch escalated until
    /// enough of them pass. No set built. Approximate like `HnswFiltered`; a
    /// short result means the whole index was walked. The name is from when
    /// only a label-only filter took this path; since #214 phase 2 a filter
    /// with properties takes it too while it is broad enough (the name stays:
    /// callers read it as a string).
    LabelChecked,
}

#[cfg(feature = "embeddings-index")]
impl VectorPath {
    /// Whether this path ranked with recall = 1, so that a short vector
    /// branch means "there genuinely are no more allowed vectors".
    ///
    /// `ExactOverAllowed` and `ExactIndex` are exact by construction.
    /// `HnswFiltered` is a graph walk and never is. `Unfiltered` and
    /// `LabelChecked` ask the index for plain KNN, which is exact only while
    /// the index itself reads exactly (`HnswIndex::is_exact`): that is what
    /// `index_exact` carries. Kept here, not in the enum, because the same
    /// variant is exact or approximate depending on the index size — a
    /// variant per size would double the table for no information.
    pub fn is_exact(self, index_exact: bool) -> bool {
        match self {
            VectorPath::ExactOverAllowed | VectorPath::ExactIndex => true,
            VectorPath::HnswFiltered => false,
            VectorPath::Unfiltered | VectorPath::LabelChecked => index_exact,
        }
    }
}

/// Which branch resolved the text path of a hybrid search (#213).
///
/// Like [`VectorPath`], what matters to a caller is *recall*: with a filter,
/// every variant below is complete — a short text branch means "no more
/// allowed documents match the text", never "the global top-N ran out before
/// reaching them".
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPath {
    /// No filter: global top-N by BM25.
    Global,
    /// Filter checked on the hits: global top-N, the filter checked on the
    /// hits in rank order, fetch escalated ×4; enough hits passed before the
    /// cap. The ranks are the ones the allowed set would give. Named when
    /// only a label-only filter took this path; since #214 phase 2 a broad
    /// property filter takes it too (see [`VectorPath::LabelChecked`]).
    GlobalLabelChecked,
    /// Allowed set of at most [`TEXT_EXACT_THRESHOLD`] nodes: tantivy scored
    /// only the allowed nodes (`query_scored_in`), no global query at all.
    /// Complete by construction, cost proportional to the set and
    /// independent of how many documents match the text.
    TermSetOverAllowed,
    /// Large allowed set, or a filter too rare among the matches: the global
    /// escalation reached its cap still short (or, for a filter with
    /// properties, its pass rate said it would), so the allowed set was
    /// scored in chunks of [`TEXT_EXACT_THRESHOLD`] and merged. Complete;
    /// the expensive case, paid only when the cheap one failed.
    GlobalThenTermSet,
}

/// Qué pidió y qué obtuvo una de las dos ramas.
///
/// `returned < requested` es la señal de underfill. En la rama de texto
/// significa que el índice no tenía más coincidencias — con filtro, que no
/// hay más permitidos que coincidan (#213, ver [`TextPath`]); en la
/// vectorial depende del camino (ver [`VectorPath`]).
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone, Copy)]
pub struct BranchReport {
    /// Candidatos pedidos a la rama (`k × overfetch`).
    pub requested: usize,
    /// Candidatos que la rama entregó, ya intersectados con el filtro.
    pub returned: usize,
}

#[cfg(feature = "hybrid")]
impl BranchReport {
    /// `true` si la rama entregó menos de lo que se le pidió.
    pub fn underfilled(&self) -> bool {
        self.returned < self.requested
    }
}

/// Un hit con la traza de por qué quedó donde quedó.
///
/// El score RRF ordena, pero no explica: dos documentos con el mismo score
/// pueden haber llegado por ramas distintas. Aquí están los números crudos
/// que la fusión consumió y descartaba.
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone)]
pub struct ExplainedHit {
    pub node_id: NodeId,
    /// Score RRF fusionado — el mismo que devuelve `search_hybrid`.
    pub score: f32,
    /// Posición 0-based en la rama de texto, si apareció en ella.
    pub text_rank: Option<usize>,
    /// Score BM25 crudo de tantivy. `None` si el nodo no vino por texto.
    pub text_score: Option<f32>,
    /// Posición 0-based en la rama vectorial, si apareció en ella.
    pub vector_rank: Option<usize>,
    /// Distancia coseno cruda (0 = idénticos). `None` si no vino por vector.
    pub vector_distance: Option<f32>,
}

/// El resultado de una búsqueda híbrida con todo lo que el motor supo.
///
/// Lo produce [`Graph::search_hybrid_explain`]; `search_hybrid` es la vista
/// reducida del MISMO cómputo, no un segundo camino — por eso el explain no
/// puede alterar el resultado.
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone)]
pub struct HybridExplain {
    /// Los hits finales, ordenados como los devuelve `search_hybrid`.
    pub hits: Vec<ExplainedHit>,
    /// `k` solicitado.
    pub k: usize,
    /// Constante RRF efectiva.
    pub rrf_k: f32,
    /// Multiplicador de over-fetch efectivo.
    pub overfetch: usize,
    /// Candidatos pedidos por rama = `k × overfetch` (mínimo `k`).
    pub candidates: usize,
    /// `ef_search` efectivo del índice vectorial; `None` si no hubo rama vectorial.
    pub ef_search: Option<usize>,
    /// Índice full-text resuelto; `None` si no hubo rama de texto.
    pub text_index: Option<String>,
    /// Tamaño del conjunto permitido por el filtro; `None` = sin filtro, o
    /// filtro de solo etiqueta resuelto sin conjunto (ver
    /// [`VectorPath::LabelChecked`]). Igual a `candidate_set.size`; se
    /// conserva por compatibilidad.
    pub allowed_set_size: Option<usize>,
    /// De dónde salió el universo de la búsqueda y sus tamaños (#214).
    pub candidate_set: CandidateSetReport,
    /// Traza de la rama de texto; `None` si la query no traía texto.
    pub text: Option<BranchReport>,
    /// Traza de la rama vectorial; `None` si la query no traía vector.
    pub vector: Option<BranchReport>,
    /// Cómo se resolvió la rama vectorial.
    pub vector_path: Option<VectorPath>,
    /// Si la rama vectorial fue exacta (recall = 1), ya resuelto por camino y
    /// tamaño del índice (ver [`VectorPath::is_exact`]); `None` si la query
    /// no traía vector. Es la bandera que decide cómo leer un resultado
    /// corto sin conocer la tabla de caminos (#218).
    pub vector_exact: Option<bool>,
    /// Cómo se resolvió la rama de texto; `None` si la query no traía texto.
    pub text_path: Option<TextPath>,
}

/// Where the candidate set of a hybrid search came from (#214).
///
/// Output only: the engine needs just the set, but a caller debugging a
/// missing hit needs to know whether it was never *eligible* (left out when
/// the set was built) or eligible and outranked. `CandidateSetReport` gives
/// the sizes before and after the filter for the same reason.
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSource {
    /// No filter and no caller set: the whole graph.
    Unrestricted,
    /// `{label, props}` only (a label-only filter resolved without a set
    /// reports no size).
    Filter,
    /// The caller's ids, as given.
    Ids,
    /// The caller's ids reduced by `{label, props}`.
    IdsAndFilter,
}

/// The candidate set a hybrid search actually ran in (#214).
#[cfg(feature = "hybrid")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateSetReport {
    pub source: CandidateSource,
    /// Distinct ids the caller passed (duplicates count once); `None`
    /// without a caller set.
    pub caller_size: Option<usize>,
    /// Size of the set both paths ran in, after the filter; `None` when no
    /// set was built (unrestricted, or a label-only filter resolved by
    /// checking the label on the candidates). Equal to `allowed_set_size`.
    /// With caller ids and no filter the ids are taken unread, so `size`
    /// counts distinct ids as passed, not nodes confirmed to exist or to
    /// carry an embedding: fewer hits than `size` there is not a filter
    /// effect. Only a filter confirms membership.
    pub size: Option<usize>,
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
    ///
    /// Vista reducida de [`Graph::search_hybrid_explain`]: mismo cómputo, con
    /// la traza descartada. Delegar en vez de duplicar es lo que hace que el
    /// explain no pueda alterar el resultado — no hay un segundo camino que
    /// pueda divergir.
    #[cfg(feature = "hybrid")]
    pub async fn search_hybrid(&self, q: HybridQuery) -> Result<Vec<HybridHit>> {
        Ok(self
            .search_hybrid_explain(q)
            .await?
            .hits
            .into_iter()
            .map(|h| HybridHit {
                node_id: h.node_id,
                score: h.score,
                text_rank: h.text_rank,
                vector_rank: h.vector_rank,
            })
            .collect())
    }

    /// Como [`Graph::search_hybrid`], devolviendo además por qué cada hit
    /// quedó donde quedó y cómo se resolvió cada rama.
    ///
    /// El score RRF ordena pero no explica: es una suma de recíprocos de
    /// RANGOS, así que no dice si un documento subió por texto, por vector o
    /// por ambos, ni con qué fuerza. Esta variante conserva los números
    /// crudos que la fusión consume y descarta — score BM25, distancia
    /// coseno — más la configuración efectiva y el underfill por rama.
    #[cfg(feature = "hybrid")]
    pub async fn search_hybrid_explain(&self, q: HybridQuery) -> Result<HybridExplain> {
        self.search_hybrid_explain_with(q, Vec::new()).await
    }

    /// [`Self::search_hybrid_explain`] with `prop ∈ [values]` lists added to
    /// the filter: what NQL pushes from `c.p in [...]` (#214 phase 2). Kept
    /// crate-internal so [`HybridFilter`] does not grow a public field.
    #[cfg(feature = "hybrid")]
    pub(crate) async fn search_hybrid_explain_with(
        &self,
        q: HybridQuery,
        any_of: Vec<(String, Vec<PropertyValue>)>,
    ) -> Result<HybridExplain> {
        if q.text.is_none() && q.vector.is_none() {
            return Err(NopalError::Custom(
                "search_hybrid: provide at least one of `text` or `vector`".into(),
            ));
        }
        let candidates = q.k.saturating_mul(q.overfetch).max(q.k);

        let filter = NodeFilter::new(q.filter.as_ref(), any_of);

        // 1. A filter is checked on each path's candidates as they are read,
        //    in rank order. The ranks come out as they would from the allowed
        //    set (each path is restricted before the fusion, so the top-k
        //    stays computed inside the filter, #115) without building the set
        //    first. Filtering after the fusion instead would rank the
        //    filter's nodes against foreign ones and could return fewer than
        //    k. With a caller set there is a set already (#214): the filter
        //    is confirmed on its members, so the shortcut does not apply.
        //
        //    Until #214 phase 2 only a label-only filter took this path and
        //    any property filter built the set up front. That is cheap for a
        //    selective filter and very expensive for a broad one: the set
        //    reads every matching index entry and hydrates every member to
        //    confirm the label — 327 ms at 100k chunks for a property 90 % of
        //    them carry, against 1.6 ms for the label alone. Now a property
        //    filter starts here too, and the pass rate of what the first
        //    rounds read predicts whether the cap would run out: when it
        //    would, the set is built right away instead of escalating to the
        //    cap first (`predict`). A label-only filter keeps the exact
        //    behaviour it had (no prediction, same paths, same costs).
        //
        //    One exception, checked first and cheaply: a property with at
        //    most `SEED_PROBE_LIMIT` members (a bounded read of the property
        //    index, at most that many keys) builds the set right away, as
        //    every property filter did until 0.6.20 — for a small set that is
        //    the cheapest path and the exact one, and no candidate has to be
        //    read to find that out.
        let small = if q.candidates.is_all() && filter.has_props() {
            self.small_seed(&filter).await?
        } else {
            None
        };
        let checked = q.candidates.is_all() && !filter.is_empty() && small.is_none();
        let predict = checked && filter.has_props();
        let mut allowed = if checked { None } else { self.allowed_set(&q.candidates, &filter, small).await? };
        let has_filter = !filter.is_empty();
        let candidate_source = match (q.candidates.is_all(), has_filter) {
            (true, false) => CandidateSource::Unrestricted,
            (true, true) => CandidateSource::Filter,
            (false, false) => CandidateSource::Ids,
            (false, true) => CandidateSource::IdsAndFilter,
        };

        // 2. Full-text path → node ids ranked by relevance.
        let mut text_ranks: HashMap<NodeId, usize> = HashMap::new();
        let mut text_scores: HashMap<NodeId, f32> = HashMap::new();
        let mut text_index_used: Option<String> = None;
        let mut text_report: Option<BranchReport> = None;
        let mut text_path_used: Option<TextPath> = None;
        if let Some(text) = &q.text {
            let index_name = self.resolve_fulltext_index(q.text_index.as_deref(), q.filter.as_ref()).await?;
            let fq = IndexQuery::FullText(text.clone());
            // Pedir a tantivy solo lo que la rama usa (#174): `candidates`,
            // y ×4 mientras el filtro deje menos, hasta el tope de siempre
            // (1000, o `candidates` si es mayor). Antes pedía 1000 fijos y
            // leía el `node_id` de cada uno: 1.9 ms de los 2.2 ms de una
            // híbrida a 100k chunks.
            //
            // El tope NO es "no hay más" (#213): con un filtro selectivo los
            // permitidos pueden estar detrás de más de `cap` ajenos por BM25.
            // La pregunta de la rama es "top-k del texto DENTRO del conjunto
            // permitido", y el top-N global es solo una forma oportunista de
            // calcularlo: un conjunto chico se puntúa entero en tantivy
            // directamente (`query_scored_in`, coste proporcional al conjunto
            // e independiente del corpus); uno grande prueba el camino global
            // y, si agota el tope corto, se puntúa entero por trozos.
            //
            // Se descartó probar siempre primero el top-N global: su coste
            // crece con los documentos que contienen el término (0.25 ms a
            // 100k, ~2.5 ms a 1M), así que para un conjunto chico en un
            // corpus grande sería la parte cara de la consulta, y para un
            // corpus chico el ahorro es de décimas de milisegundo.
            let cap = crate::index::fulltext::QUERY_DEFAULT_LIMIT.max(candidates);
            // Con propiedades (#214 fase 2) la primera ronda pide el doble:
            // cada consulta a tantivy puntúa todos los documentos que tienen
            // el término, pida 40 u 80, así que una segunda ronda cuesta
            // tanto como la primera, y leer el doble de ids casi nada. Un
            // filtro de hasta el 50 % se resuelve así en una sola ronda.
            let first = if predict { candidates.saturating_mul(2) } else { candidates };
            let mut limit = first.clamp(1, cap);
            let small_set = allowed.as_ref().filter(|s| s.len() <= TEXT_EXACT_THRESHOLD);
            let (kept, path) = if let Some(set) = small_set {
                (
                    self.text_term_set_in_chunks(&index_name, &fq, set, candidates).await?,
                    TextPath::TermSetOverAllowed,
                )
            } else {
                loop {
                    let scored = self.index_manager.query_scored_top(&index_name, &fq, limit).await?;
                    // Menos de lo pedido = el índice no tiene más coincidencias.
                    let no_more = scored.len() < limit;
                    let at_cap = limit >= cap;
                    let examined = scored.len();
                    let scored = if checked {
                        self.keep_matching_in_rank_order(scored, &filter, candidates).await?
                    } else {
                        scored
                    };
                    let kept: Vec<(NodeId, Option<f32>)> = scored
                        .into_iter()
                        .filter(|(id, _)| allowed.as_ref().is_none_or(|s| s.contains(id)))
                        .take(candidates)
                        .collect();
                    let filtered = allowed.is_some() || checked;
                    if kept.len() >= candidates || no_more || !filtered {
                        let path = if checked { TextPath::GlobalLabelChecked } else { TextPath::Global };
                        break (kept, path);
                    }
                    // A ese ritmo de aciertos, ¿alcanzaría el tope? Si no,
                    // escalar hasta él es tiempo perdido: el conjunto ya.
                    let hopeless = predict && fetch_needed(kept.len(), examined, candidates) > cap;
                    if at_cap || hopeless {
                        // El conjunto se construye ahora (con solo etiqueta,
                        // índice `label_idx`, #207); la rama vectorial lo
                        // reutiliza.
                        if allowed.is_none() {
                            allowed = self.allowed_set(&q.candidates, &filter, None).await?;
                        }
                        let set = allowed.as_ref().expect("filtro ⇒ conjunto permitido");
                        break (
                            self.text_term_set_in_chunks(&index_name, &fq, set, candidates).await?,
                            TextPath::GlobalThenTermSet,
                        );
                    }
                    limit = next_fetch(limit, kept.len(), examined, candidates, predict).min(cap);
                }
            };
            for (id, score) in kept {
                let n = text_ranks.len();
                if text_ranks.insert(id, n).is_none()
                    && let Some(s) = score
                {
                    text_scores.insert(id, s);
                }
            }
            text_report = Some(BranchReport {
                requested: candidates,
                returned: text_ranks.len(),
            });
            text_index_used = Some(index_name);
            text_path_used = Some(path);
        }

        // 3. Vector path → node ids ranked by distance.
        let mut vector_ranks: HashMap<NodeId, usize> = HashMap::new();
        let mut vector_distances: HashMap<NodeId, f32> = HashMap::new();
        let mut vector_report: Option<BranchReport> = None;
        let mut vector_path_used: Option<VectorPath> = None;
        let mut vector_exact_used: Option<bool> = None;
        let mut ef_used: Option<usize> = None;
        if let Some((vector, model)) = &q.vector {
            let index = self.get_or_build_embedding_index(model).await?;
            let ef = q.ef_search.unwrap_or(DEFAULT_EF_SEARCH);
            // Con propiedades y el conjunto ya construido por la rama de texto
            // (el filtro resultó selectivo), el conjunto es lo correcto: es
            // exacto si es chico. Se midió comprobar la pertenencia en el
            // conjunto sobre un HNSW sin filtro cuando el conjunto es grande:
            // con 2 000 miembros de 100k, 45 ms contra 24 ms del recorrido
            // filtrado (pedir 2 000 vecinos es lo caro). Con solo etiqueta,
            // como siempre: se prueba la comprobación aunque el texto haya
            // construido el conjunto. Con propiedades la primera ronda pide el
            // doble (una búsqueda de 80 cuesta menos que dos de 40 y 57).
            let try_checked = checked && !(predict && allowed.is_some());
            let first_fetch = if predict { candidates.saturating_mul(2) } else { candidates };
            let checked_hits = if try_checked {
                self.vector_path_checked(&index, vector, &filter, candidates, first_fetch, ef, predict)
                    .await?
            } else {
                None
            };
            let (hits, path) = match checked_hits {
                Some(hits) => (hits, VectorPath::LabelChecked),
                None => {
                    // Sin filtro, conjunto del caller, o un filtro tan escaso
                    // entre los vecinos que la escalada llegó (o llegaría) al
                    // tope: el conjunto permitido es lo correcto.
                    if checked && allowed.is_none() {
                        allowed = self.allowed_set(&q.candidates, &filter, None).await?;
                    }
                    self.vector_path(&index, vector, allowed.as_ref(), candidates, ef)
                        .await?
                }
            };
            vector_report = Some(BranchReport {
                requested: candidates,
                returned: hits.len(),
            });
            vector_path_used = Some(path);
            vector_exact_used = Some(path.is_exact(index.read().unwrap_or_else(|e| e.into_inner()).is_exact()));
            ef_used = Some(ef);
            for (rank, (id, dist)) in hits.into_iter().enumerate() {
                if vector_ranks.insert(id, rank).is_none() {
                    vector_distances.insert(id, dist);
                }
            }
        }

        // 4. RRF fusion over the union of both paths.
        let mut ids: HashSet<NodeId> = HashSet::new();
        ids.extend(text_ranks.keys().copied());
        ids.extend(vector_ranks.keys().copied());

        let mut hits: Vec<ExplainedHit> = ids
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
                ExplainedHit {
                    node_id: id,
                    score,
                    text_rank: tr,
                    text_score: text_scores.get(&id).copied(),
                    vector_rank: vr,
                    vector_distance: vector_distances.get(&id).copied(),
                }
            })
            .collect();

        // Sort by score desc; break ties by node id for determinism.
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        hits.truncate(q.k);

        Ok(HybridExplain {
            hits,
            k: q.k,
            rrf_k: q.rrf_k,
            overfetch: q.overfetch,
            candidates,
            ef_search: ef_used,
            text_index: text_index_used,
            allowed_set_size: allowed.as_ref().map(|s| s.len()),
            candidate_set: CandidateSetReport {
                source: candidate_source,
                caller_size: q.candidates.len(),
                size: allowed.as_ref().map(|s| s.len()),
            },
            text: text_report,
            vector: vector_report,
            vector_path: vector_path_used,
            vector_exact: vector_exact_used,
            text_path: text_path_used,
        })
    }

    /// Top `candidates` by BM25 among `allowed`, scoring the set in chunks of
    /// [`TEXT_EXACT_THRESHOLD`] ids and merging (#213). Scores are comparable
    /// across chunks (same query, index-wide IDF), so the best `candidates`
    /// of the union of per-chunk tops is the best `candidates` of the whole
    /// set. Chunking bounds the size of each tantivy term set; the cost is
    /// still proportional to `allowed.len()`.
    ///
    /// Ties keep tantivy's order: with one chunk (the usual case) the hits
    /// are returned as collected, so a document ranks exactly where the
    /// global top-N would rank it; across chunks the merge is a stable sort
    /// by score over chunks of sorted ids, deterministic for a given set.
    #[cfg(feature = "hybrid")]
    async fn text_term_set_in_chunks(
        &self,
        index_name: &str,
        query: &IndexQuery,
        allowed: &HashSet<NodeId>,
        candidates: usize,
    ) -> Result<Vec<(NodeId, Option<f32>)>> {
        let chunk_size = TEXT_EXACT_THRESHOLD.max(1);
        if allowed.len() <= chunk_size {
            return self.index_manager.query_scored_in(index_name, query, allowed, candidates).await;
        }
        let mut ids: Vec<NodeId> = allowed.iter().copied().collect();
        ids.sort_unstable();
        let mut merged: Vec<(NodeId, Option<f32>)> = Vec::new();
        for chunk in ids.chunks(chunk_size) {
            let set: HashSet<NodeId> = chunk.iter().copied().collect();
            merged.extend(self.index_manager.query_scored_in(index_name, query, &set, candidates).await?);
        }
        merged.sort_by(|a, b| b.1.unwrap_or(0.0).total_cmp(&a.1.unwrap_or(0.0)));
        merged.truncate(candidates);
        Ok(merged)
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
    /// cheaper: no graph walk. "Small enough" grows with the index
    /// ([`exact_over_allowed_limit`], `100 × ∛N`, never under 1024): the walk
    /// gets costlier as the allowed fraction shrinks, the scan only with the
    /// set's size (#254).
    ///
    /// Only the caller knows the allowed cardinality — the index sees an opaque
    /// closure — so the decision lives here, not in `HnswIndex`.
    ///
    /// The exact branch is skipped when the index itself is under the threshold:
    /// there the index's filtered search is already exact over every point,
    /// without touching storage, and it says so (`FilteredSearchPath::Exact`),
    /// which is reported as [`VectorPath::ExactIndex`] rather than assumed
    /// to be the graph walk (#218).
    ///
    /// The exact branch reads the set's vectors from the index itself
    /// (`HnswIndex::vectors_for`), not from storage (#224). Until 0.6.15 it
    /// loaded them with one storage read per id — and, on redb, opened the
    /// embeddings keyspace per query, a write transaction that also dropped
    /// every cached read table — which made the vector branch 2.2 ms for 65
    /// ids where the text branch took 0.16. The index is trusted as the
    /// source: every writer of node embeddings updates the cached index in
    /// the same call, the `HnswFiltered` branch already reads only the
    /// index, and `get_or_build_embedding_index` builds or verifies it
    /// against storage before any search. There is deliberately no per-id
    /// fallback to storage: it would reintroduce a storage read for every
    /// allowed node that simply has no embedding (a candidate set mixing
    /// labels), which is the common case, to cover the microseconds between
    /// the storage write and the index insert that the other branch does
    /// not cover either.
    ///
    /// The index lock is taken only around the synchronous index calls, never
    /// across an `await`: the exact branch reads storage with the lock released.
    #[cfg(feature = "embeddings-index")]
    pub(crate) async fn vector_path(
        &self,
        index: &super::SharedHnswIndex,
        vector: &[f32],
        allowed: Option<&HashSet<NodeId>>,
        candidates: usize,
        ef: usize,
    ) -> Result<(Vec<(NodeId, f32)>, VectorPath)> {
        let Some(set) = allowed else {
            let hits = index.read().unwrap_or_else(|e| e.into_inner()).search_knn_with_ef(vector, candidates, ef)?;
            return Ok((hits, VectorPath::Unfiltered));
        };

        let index_len = index.read().unwrap_or_else(|e| e.into_inner()).len();
        if set.len() <= exact_over_allowed_limit(index_len) && index_len > EXACT_SEARCH_THRESHOLD {
            // Los vectores salen del índice, bajo el read lock y sin `await`
            // dentro; se puntúan con el lock ya liberado. Todo vector
            // indexado tiene la dimensión del índice (`insert` lo valida),
            // así que no hay nada que filtrar.
            let vectors = index
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .vectors_for(set.iter().copied());
            let hits = rank_exact(vector, vectors.iter().map(|(id, v)| (*id, v.as_slice())), candidates);
            return Ok((hits, VectorPath::ExactOverAllowed));
        }

        let outcome = index
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .search_knn_filtered_explained(vector, candidates, ef, |id| set.contains(id))?;
        let path = match outcome.path {
            FilteredSearchPath::Exact => VectorPath::ExactIndex,
            FilteredSearchPath::HnswFiltered => VectorPath::HnswFiltered,
        };
        Ok((outcome.hits, path))
    }

    /// Vector path for a filter checked on the candidates, without building
    /// the allowed set.
    ///
    /// Asks the index for `first_fetch` neighbours (at least `candidates`),
    /// reads them by id and keeps the first `candidates` that pass `filter`,
    /// in distance order. If fewer pass, the fetch grows ×4 and the search
    /// repeats; nodes already read are remembered. Stops when there are
    /// enough, when the fetch covers the whole index (then a short result is
    /// real), or at [`LABEL_CHECK_MAX_FETCH`], where it returns `None` and the
    /// caller falls back to the allowed set: a filter that rare among the
    /// neighbours is cheaper to enumerate than to find by walking.
    ///
    /// With `predict` (a filter with properties, #214 phase 2) it also
    /// returns `None` early, from the pass rate of what it has read: when
    /// reaching `candidates` would take a fetch above
    /// [`PREDICT_MAX_VECTOR_FETCH`], and when the set
    /// the rate implies is small enough to rank exactly
    /// ([`exact_over_allowed_limit`]) — the set path is then both cheaper and
    /// exact, which is what a property filter got before this path existed.
    #[cfg(feature = "embeddings-index")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn vector_path_checked(
        &self,
        index: &super::SharedHnswIndex,
        vector: &[f32],
        filter: &NodeFilter,
        candidates: usize,
        first_fetch: usize,
        ef: usize,
        predict: bool,
    ) -> Result<Option<Vec<(NodeId, f32)>>> {
        let index_len = index.read().unwrap_or_else(|e| e.into_inner()).len();
        let mut passes: HashMap<NodeId, bool> = HashMap::new();
        let mut fetch = first_fetch.max(candidates).max(1).min(LABEL_CHECK_MAX_FETCH.max(candidates));
        loop {
            let hits = index
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .search_knn_with_ef(vector, fetch, ef)?;
            let unknown: Vec<NodeId> = hits
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| !passes.contains_key(id))
                .collect();
            for (id, node) in unknown.iter().zip(self.get_nodes(&unknown).await?) {
                passes.insert(*id, node.is_some_and(|n| filter.matches(&n)));
            }
            let examined = hits.len();
            let kept: Vec<(NodeId, f32)> = hits
                .into_iter()
                .filter(|(id, _)| passes.get(id).copied().unwrap_or(false))
                .take(candidates)
                .collect();
            if kept.len() >= candidates || fetch >= index_len {
                return Ok(Some(kept));
            }
            if fetch >= LABEL_CHECK_MAX_FETCH {
                return Ok(None);
            }
            if predict {
                let needed = fetch_needed(kept.len(), examined, candidates);
                let implied_set = (kept.len() as f64 / examined.max(1) as f64) * index_len as f64;
                let small_enough_to_be_exact =
                    !kept.is_empty() && index_len > EXACT_SEARCH_THRESHOLD && implied_set <= exact_over_allowed_limit(index_len) as f64;
                if needed > PREDICT_MAX_VECTOR_FETCH || small_enough_to_be_exact {
                    return Ok(None);
                }
            }
            fetch = next_fetch(fetch, kept.len(), examined, candidates, predict).min(LABEL_CHECK_MAX_FETCH);
        }
    }

    /// The first `want` entries of a ranked list whose node passes `filter`,
    /// in the same order. Reads the nodes by id in blocks and stops as soon
    /// as it has enough, so a long full-text match list costs only the
    /// prefix it needs.
    #[cfg(feature = "hybrid")]
    async fn keep_matching_in_rank_order<T>(
        &self,
        ranked: Vec<(NodeId, T)>,
        filter: &NodeFilter,
        want: usize,
    ) -> Result<Vec<(NodeId, T)>> {
        let block = want.max(16);
        let mut kept = Vec::with_capacity(want);
        let mut rest = ranked.into_iter().peekable();
        while kept.len() < want && rest.peek().is_some() {
            let chunk: Vec<(NodeId, T)> = rest.by_ref().take(block).collect();
            let ids: Vec<NodeId> = chunk.iter().map(|(id, _)| *id).collect();
            let nodes = self.get_nodes(&ids).await?;
            for (entry, node) in chunk.into_iter().zip(nodes) {
                if kept.len() < want && node.is_some_and(|n| filter.matches(&n)) {
                    kept.push(entry);
                }
            }
        }
        Ok(kept)
    }

    /// The `k` nearest nodes for `vector` inside `ids ∩ filter`, best first,
    /// and how the vector path resolved them. What NQL `similar_to` runs
    /// when its pattern has a label or its `where` pushes predicates (#174b,
    /// #214 phase 2).
    ///
    /// Same steps as the vector branch of a filtered hybrid search: with no
    /// caller set, the filter is checked on the neighbours, asking for
    /// `k × overfetch` first and escalating ×4; if the filter is too rare
    /// among the nearest neighbours for that to fill `k` (or, with
    /// properties, predicted to be), the allowed set is built and searched
    /// (exact when it is small). So a filter with `k` embedded members
    /// returns `k` of them.
    #[cfg(feature = "embeddings-index")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn knn_in_filter(
        &self,
        model: &str,
        vector: &[f32],
        filter: &NodeFilter,
        ids: &CandidateSet,
        k: usize,
        overfetch: usize,
        ef: usize,
    ) -> Result<(Vec<(NodeId, f32)>, VectorPath)> {
        let index = self.get_or_build_embedding_index(model).await?;
        let small = if ids.is_all() && filter.has_props() { self.small_seed(filter).await? } else { None };
        if small.is_none() && ids.is_all() && !filter.is_empty() {
            let first_fetch = k.saturating_mul(overfetch.max(1));
            if let Some(hits) = self
                .vector_path_checked(&index, vector, filter, k, first_fetch, ef, filter.has_props())
                .await?
            {
                return Ok((hits, VectorPath::LabelChecked));
            }
        }
        let allowed = self.allowed_set(ids, filter, small).await?;
        self.vector_path(&index, vector, allowed.as_ref(), k, ef).await
    }

    /// Build the allowed NodeId set from the caller's candidate set and a
    /// filter. `None` means no restriction (`All` with an empty filter). An
    /// empty set means nothing matches.
    ///
    /// Each property the storage index can answer is looked up there (one
    /// lookup per equality, a union per `in` list; ids only) and intersected;
    /// starting from the label instead meant reading and deserializing every
    /// node of the label to get its ids: 190 ms for a label of 100k nodes,
    /// for a filter that leaves 20 (#213). When the members have to be read
    /// anyway — to confirm the label, or to check a value the index cannot
    /// key (a list, an object) — the whole filter is checked on them with
    /// the evaluator's equality, so a stale index entry never lets a
    /// non-matching node in. Without a label and with indexable values only
    /// nothing is read (as before #214 phase 2). With no indexable property
    /// the members come from the label (or, without one, from every node:
    /// the only correct source left). Until #214 phase 2 an unindexable
    /// value silently produced an empty set.
    ///
    /// A caller set (#214) is the *seed* of that same pipeline, so the
    /// filter can only keep or shrink it (never widen, invariant 2 of
    /// [`CandidateSet`]). Without a filter the ids are taken as given,
    /// unread: an id that does not exist never scores.
    #[cfg(feature = "embeddings-index")]
    async fn allowed_set(
        &self,
        candidates: &CandidateSet,
        filter: &NodeFilter,
        small: Option<Vec<NodeId>>,
    ) -> Result<Option<HashSet<NodeId>>> {
        let seed: Option<HashSet<NodeId>> = match candidates {
            CandidateSet::All => None,
            CandidateSet::Ids(ids) => Some(ids.to_hash_set()),
        };
        if filter.is_empty() || seed.as_ref().is_some_and(|s| s.is_empty()) {
            return Ok(seed);
        }
        // A small member list from the planner's probe: it is the set, and
        // whatever else the filter asks is checked on those few members
        // instead of reading the other properties' (possibly huge) lists.
        if let (None, Some(members)) = (&seed, small) {
            let predicates = filter.eq.len() + filter.any_of.len();
            if filter.label.is_none() && predicates == 1 {
                return Ok(Some(members.into_iter().collect()));
            }
            return Ok(Some(self.confirm(members, filter).await?));
        }
        let mut set = seed;
        let mut must_read = filter.label.is_some();

        for (prop, val) in &filter.eq {
            if !indexable(prop, val) {
                must_read = true;
                continue;
            }
            let by_prop: HashSet<NodeId> = self.get_all_nodes_by_property(prop, val).await?.into_iter().collect();
            set = Some(intersect(set, by_prop));
            if set.as_ref().is_some_and(|s| s.is_empty()) {
                return Ok(set);
            }
        }
        for (prop, values) in &filter.any_of {
            if !values.iter().all(|v| indexable(prop, v)) {
                must_read = true;
                continue;
            }
            let mut by_prop: HashSet<NodeId> = HashSet::new();
            for v in values {
                by_prop.extend(self.get_all_nodes_by_property(prop, v).await?);
            }
            set = Some(intersect(set, by_prop));
            if set.as_ref().is_some_and(|s| s.is_empty()) {
                return Ok(set);
            }
        }

        Ok(Some(match set {
            Some(members) if must_read => self.confirm(members.into_iter().collect(), filter).await?,
            Some(members) => members,
            None => {
                let nodes = match &filter.label {
                    Some(label) => self.get_nodes_by_label(label).await?,
                    None => self.get_all_nodes().await?,
                };
                nodes.into_iter().filter(|n| filter.matches(n)).map(|n| n.id).collect()
            }
        }))
    }

    /// The members that pass `filter`, read in blocks.
    #[cfg(feature = "embeddings-index")]
    async fn confirm(&self, ids: Vec<NodeId>, filter: &NodeFilter) -> Result<HashSet<NodeId>> {
        let mut confirmed = HashSet::with_capacity(ids.len());
        for chunk in ids.chunks(1024) {
            for (id, node) in chunk.iter().zip(self.get_nodes(chunk).await?) {
                if node.is_some_and(|n| filter.matches(&n)) {
                    confirmed.insert(*id);
                }
            }
        }
        Ok(confirmed)
    }

    /// The members of the most selective indexable predicate of `filter`,
    /// if one has at most [`SEED_PROBE_LIMIT`] of them; `None` otherwise.
    /// Bounded: each predicate reads at most the current best size + 1
    /// keys of the property index, so a broad filter costs at most
    /// `SEED_PROBE_LIMIT + 1` keys per predicate and a selective one only
    /// its own members.
    #[cfg(feature = "embeddings-index")]
    async fn small_seed(&self, filter: &NodeFilter) -> Result<Option<Vec<NodeId>>> {
        let mut best: Option<Vec<NodeId>> = None;
        let bound = |best: &Option<Vec<NodeId>>| best.as_ref().map_or(SEED_PROBE_LIMIT, |b| b.len());
        for (prop, value) in &filter.eq {
            if !indexable(prop, value) {
                continue;
            }
            if let Some(ids) = self.storage.get_nodes_by_property_up_to(prop, value, bound(&best)).await?
                && best.as_ref().is_none_or(|b| ids.len() < b.len())
            {
                best = Some(ids);
            }
        }
        'lists: for (prop, values) in &filter.any_of {
            if !values.iter().all(|v| indexable(prop, v)) {
                continue;
            }
            let limit = bound(&best);
            let mut ids: Vec<NodeId> = Vec::new();
            for value in values {
                match self.storage.get_nodes_by_property_up_to(prop, value, limit - ids.len()).await? {
                    Some(more) => ids.extend(more),
                    None => continue 'lists,
                }
            }
            if best.as_ref().is_none_or(|b| ids.len() < b.len()) {
                best = Some(ids);
            }
        }
        Ok(best)
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

/// Largest fetch the label-checked vector path tries before falling back to
/// the allowed set. Same bound as the filtered HNSW escalation.
#[cfg(feature = "embeddings-index")]
const LABEL_CHECK_MAX_FETCH: usize = crate::embeddings::MAX_FILTERED_EF_SEARCH;

/// Allowed sets up to this size are scored directly as one term set
/// ([`TextPath::TermSetOverAllowed`]); larger ones try the global top-N,
/// escalating, and fall back to the term set in chunks of this size only if
/// the cap runs out short ([`TextPath::GlobalThenTermSet`]).
///
/// Correctness never depends on it — the fallback is complete at any size —
/// only cost does. Measured on the `text_filter` bench (100k documents all
/// matching the query, release, #213): a term set costs ~4.3 µs per allowed
/// id (8 → 0.09 ms, 128 → 0.68 ms, 256 → 1.25 ms, 512 → 2.2 ms, 1000 →
/// 3.8 ms, 4000 → 12 ms), the escalation it replaces (128 → 512 → 1000)
/// ~2 ms. 512 is where the two meet; below it the term set is both complete
/// and cheaper.
#[cfg(feature = "hybrid")]
pub const TEXT_EXACT_THRESHOLD: usize = 512;

/// Largest member list of a property predicate that builds the allowed set
/// without trying the checked path first (#214 phase 2). The same size as
/// [`TEXT_EXACT_THRESHOLD`]: a set up to here is scored by the text branch
/// as one term set, and is far below the vector branch's exact threshold,
/// so it is both the cheapest and an exact path. The probe that decides it
/// reads at most this many keys of the property index.
#[cfg(feature = "embeddings-index")]
const SEED_PROBE_LIMIT: usize = 512;

/// With properties, the largest fetch the checked vector path accepts before
/// handing over to the allowed set. Measured at 100k vectors (64 dims, k =
/// 10): asking the HNSW index for ~2 000 neighbours and checking them cost
/// 45 ms, the filtered walk over the same 2 000-member set 24 ms; at ~500
/// neighbours the checked path wins (5 ms against 13 ms for a 10 % filter).
#[cfg(feature = "embeddings-index")]
const PREDICT_MAX_VECTOR_FETCH: usize = 1024;

/// How many ranked candidates a checked path would have to read to keep
/// `want`, at the pass rate of the `examined` it has read (`kept` passed).
/// With nothing kept, the rate is unknown until a sample of 128: below that
/// it answers 0 (keep escalating); from there on, `usize::MAX` (hopeless).
#[cfg(feature = "embeddings-index")]
fn fetch_needed(kept: usize, examined: usize, want: usize) -> usize {
    match kept {
        0 if examined < 128 => 0,
        0 => usize::MAX,
        _ => examined.saturating_mul(want).div_ceil(kept),
    }
}

/// The next fetch of a checked path. ×4 without `predict` (the label-only
/// escalation, unchanged since 0.6.10). With `predict` (#214 phase 2), what
/// the pass rate says is needed plus 25 %, never less than one more: a 90 %
/// filter that kept 36 of 40 asks for 57 next, not 160 — on an HNSW walk and
/// on tantivy, the size of the fetch is the cost.
#[cfg(feature = "embeddings-index")]
fn next_fetch(fetch: usize, kept: usize, examined: usize, want: usize, predict: bool) -> usize {
    let x4 = fetch.saturating_mul(4);
    if !predict || kept == 0 {
        return x4;
    }
    let needed = fetch_needed(kept, examined, want);
    needed.saturating_add(needed / 4).clamp(fetch + 1, x4)
}

/// Intersect an optional running set with a new set.
#[cfg(feature = "embeddings-index")]
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

        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        assert!(index.read().unwrap().len() > EXACT_SEARCH_THRESHOLD, "índice por encima del umbral");
        assert!(allowed_ids.len() <= EXACT_SEARCH_THRESHOLD, "permitidos por debajo");

        let allowed: HashSet<NodeId> = allowed_ids.iter().map(|(id, _)| *id).collect();
        let query = &vecs[7];
        let k = 10;

        let (hits, path) = graph
            .vector_path(index, query, Some(&allowed), k, 30)
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
        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        let (hits, path) = graph
            .vector_path(index, &[1.0, 0.0, 0.0, 0.0], None, 5, 30)
            .await
            .unwrap();
        assert_eq!(path, VectorPath::Unfiltered);
        assert_eq!(hits.len(), 5);
    }

    /// El corte del camino exacto crece con el índice (#254): `100 × ∛N`,
    /// nunca menos de 1024. Antes era 1024 fijo.
    #[test]
    fn exact_over_allowed_limit_grows_with_the_index() {
        use crate::embeddings::exact_over_allowed_limit;
        assert_eq!(exact_over_allowed_limit(0), EXACT_SEARCH_THRESHOLD);
        assert_eq!(exact_over_allowed_limit(1_000), EXACT_SEARCH_THRESHOLD, "índice chico: el corte de siempre");
        assert!((4_600..=4_650).contains(&exact_over_allowed_limit(100_000)));
        assert!((9_990..=10_000).contains(&exact_over_allowed_limit(1_000_000)));
        let mut last = 0;
        for n in [1_000, 10_000, 100_000, 1_000_000, 10_000_000] {
            let limit = exact_over_allowed_limit(n);
            assert!(limit >= last, "monótono");
            last = limit;
        }
    }

    /// Un conjunto de más de 1024 permitidos, chico para su índice, ya no
    /// recorre el grafo: se puntúa exacto (antes de #254 iba por
    /// `HnswFiltered`, aproximado y más caro).
    #[tokio::test]
    async fn allowed_set_over_1024_small_for_its_index_ranks_exactly() {
        use crate::embeddings::exact_over_allowed_limit;
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();

        let dim = 8;
        let n = 4_000;
        let vecs = vectors(n, dim, 2540);
        let mut items = Vec::with_capacity(n);
        let mut allowed_ids: Vec<(NodeId, Vec<f32>)> = Vec::new();
        for (i, v) in vecs.iter().enumerate() {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            items.push((id, v.clone()));
            if i % 8 < 3 {
                allowed_ids.push((id, v.clone()));
            }
        }
        graph.add_node_embeddings("m", items).await.unwrap();

        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        let limit = exact_over_allowed_limit(index.read().unwrap().len());
        assert!(
            allowed_ids.len() > EXACT_SEARCH_THRESHOLD && allowed_ids.len() <= limit,
            "{} permitidos: por encima de 1024 y bajo el corte {limit}",
            allowed_ids.len()
        );

        let allowed: HashSet<NodeId> = allowed_ids.iter().map(|(id, _)| *id).collect();
        let query = &vecs[11];
        let k = 10;
        let (hits, path) = graph.vector_path(index, query, Some(&allowed), k, 30).await.unwrap();
        assert_eq!(path, VectorPath::ExactOverAllowed);

        let mut expected: Vec<(NodeId, f32)> =
            allowed_ids.iter().map(|(id, v)| (*id, cosine_ref(query, v))).collect();
        expected.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        expected.truncate(k);
        let got: Vec<NodeId> = hits.iter().map(|(id, _)| *id).collect();
        let want: Vec<NodeId> = expected.iter().map(|(id, _)| *id).collect();
        assert_eq!(got, want, "exacto: el mismo top-k que la fuerza bruta");
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

        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        assert!(allowed.len() > EXACT_SEARCH_THRESHOLD, "permitidos por encima del umbral");

        let (hits, path) = graph
            .vector_path(index, &vecs[3], Some(&allowed), 10, 30)
            .await
            .unwrap();

        assert_eq!(path, VectorPath::HnswFiltered);
        assert!(
            hits.iter().all(|(id, _)| allowed.contains(id)),
            "el camino filtrado no debe devolver nada fuera del conjunto permitido"
        );
        assert!(!path.is_exact(index.read().unwrap().is_exact()), "el recorrido HNSW no es exacto");
    }

    /// Un filtro sobre un índice que entero cabe bajo el umbral no recorre
    /// el grafo: el índice puntúa todos los puntos y aplica el predicado.
    /// Hasta 0.6.13 se informaba `HnswFiltered`, y un caller que se abstiene
    /// leía un resultado definitivo como incierto (#218).
    #[tokio::test]
    async fn filter_over_a_small_index_is_reported_as_exact() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();
        let dim = 8;
        let vecs = vectors(65, dim, 2026);
        let mut allowed: HashSet<NodeId> = HashSet::new();
        for (i, v) in vecs.iter().enumerate() {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
            if i % 3 == 0 {
                allowed.insert(id);
            }
        }
        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        assert!(index.read().unwrap().is_exact(), "índice bajo el umbral ⇒ lectura exacta");

        let (hits, path) = graph
            .vector_path(index, &vecs[1], Some(&allowed), 8, 30)
            .await
            .unwrap();
        assert_eq!(path, VectorPath::ExactIndex);
        assert!(path.is_exact(true));
        assert_eq!(hits.len(), 8);
        assert!(hits.iter().all(|(id, _)| allowed.contains(id)));

        let (_, path) = graph.vector_path(index, &vecs[1], None, 8, 30).await.unwrap();
        assert_eq!(path, VectorPath::Unfiltered);
        assert!(path.is_exact(true), "KNN sin filtro sobre índice chico también es exacto");
    }

    /// El camino exacto lee del índice, no de storage (#224): lo que entra
    /// por `add_node_embedding` aparece y lo que sale por `delete_node`
    /// desaparece sin rebuild (queda como tombstone), con el mismo ranking
    /// que la fuerza bruta.
    #[tokio::test]
    async fn exact_branch_reads_the_index_not_storage() {
        let dir = tempfile::tempdir().unwrap();
        let graph = Graph::open(dir.path()).await.unwrap();
        let dim = 8;
        let n = EXACT_SEARCH_THRESHOLD + 50;
        let vecs = vectors(n, dim, 31337);
        let mut ids = Vec::with_capacity(n);
        for v in &vecs {
            let id = graph.add_node(Node::new("Frag")).await.unwrap();
            graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
            ids.push(id);
        }
        let shared = graph.get_or_build_embedding_index("m").await.unwrap();
        let index = &shared;
        let query = &vecs[5];

        // Un nodo nuevo, insertado DESPUÉS de construir el índice, con el
        // vector más cercano posible: la rama exacta lo ve porque el índice
        // cacheado lo recibió en la misma llamada.
        let newcomer = graph.add_node(Node::new("Frag")).await.unwrap();
        graph.add_node_embedding(newcomer, query.clone(), "m").await.unwrap();
        let allowed: HashSet<NodeId> = ids.iter().step_by(40).copied().chain([newcomer]).collect();
        assert!(allowed.len() <= EXACT_SEARCH_THRESHOLD);

        let (hits, path) = graph.vector_path(index, query, Some(&allowed), 5, 30).await.unwrap();
        assert_eq!(path, VectorPath::ExactOverAllowed);
        assert_eq!(hits[0].0, newcomer, "el recién insertado es el más cercano");
        assert!(hits[0].1.abs() < 1e-6);
        let mut expected: Vec<(NodeId, f32)> = allowed
            .iter()
            .filter(|id| **id != newcomer)
            .map(|id| (*id, cosine_ref(query, &vecs[ids.iter().position(|x| x == id).unwrap()])))
            .chain([(newcomer, 0.0)])
            .collect();
        expected.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let want: Vec<NodeId> = expected.iter().take(5).map(|(id, _)| *id).collect();
        assert_eq!(hits.iter().map(|(id, _)| *id).collect::<Vec<_>>(), want, "ranking = fuerza bruta");

        // Borrarlo lo saca de la rama exacta en el acto, sin reconstruir.
        graph.delete_node(newcomer).await.unwrap();
        assert_eq!(index.read().unwrap().tombstones(), 1, "borrado lógico, sin rebuild");
        let (hits, path) = graph.vector_path(index, query, Some(&allowed), 5, 30).await.unwrap();
        assert_eq!(path, VectorPath::ExactOverAllowed);
        assert!(hits.iter().all(|(id, _)| *id != newcomer), "el tombstone no puntúa");
        assert_eq!(hits.len(), 5);
    }

    /// La bandera resume la tabla de caminos: los dos caminos exactos lo son
    /// siempre, el recorrido HNSW nunca, y KNN plano (con o sin comprobación
    /// de etiqueta) solo mientras el índice lee exacto.
    #[test]
    fn fetch_prediction_follows_the_pass_rate() {
        // Nothing kept: keep escalating until a sample of 128, then hopeless.
        assert_eq!(fetch_needed(0, 40, 40), 0);
        assert_eq!(fetch_needed(0, 160, 40), usize::MAX);
        // 36 of 40 pass: 45 are needed for 40.
        assert_eq!(fetch_needed(36, 40, 40), 45);
        // 1 % pass rate: 4 000 for 40.
        assert_eq!(fetch_needed(4, 400, 40), 4000);
        // Label-only (no prediction) keeps the ×4 escalation of 0.6.10.
        assert_eq!(next_fetch(40, 36, 40, 40, false), 160);
        // With properties: what is needed plus 25 %, within (fetch, ×4].
        assert_eq!(next_fetch(40, 36, 40, 40, true), 56);
        assert_eq!(next_fetch(40, 39, 40, 40, true), 52);
        assert_eq!(next_fetch(40, 1, 40, 40, true), 160);
        assert_eq!(next_fetch(40, 0, 40, 40, true), 160);
    }

    #[test]
    fn vector_exact_follows_the_path_and_the_index_size() {
        for index_exact in [true, false] {
            assert!(VectorPath::ExactOverAllowed.is_exact(index_exact));
            assert!(VectorPath::ExactIndex.is_exact(index_exact));
            assert!(!VectorPath::HnswFiltered.is_exact(index_exact));
            assert_eq!(VectorPath::Unfiltered.is_exact(index_exact), index_exact);
            assert_eq!(VectorPath::LabelChecked.is_exact(index_exact), index_exact);
        }
    }
}
