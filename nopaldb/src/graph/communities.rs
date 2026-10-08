// Comunidades persistidas con keys estables (#190 c).
//
// `materialize_communities` guarda una jerarquía de comunidades (por ejemplo
// la de `LeidenCommunity::detect_hierarchy`) como nodos del grafo:
//
//   (:Community {partition, level, key, size})
//   (miembro)-[:IN_COMMUNITY]->(:Community)          un nivel por arista
//   (:Community nivel L)-[:PARENT_OF]->(:Community nivel L+1)
//
// Es un upsert de estado deseado, como `upsert_node`: lee lo que ya hay de la
// misma `partition`, calcula la diferencia y la escribe en UNA transacción
// (atómica; o queda la jerarquía vieja o la nueva). Recalcular sobre la misma
// partición no escribe nada.
//
// Keys estables. Al recalcular, cada comunidad nueva se empareja con una
// previa del mismo nivel por solapamiento de miembros (Jaccard ≥
// `min_jaccard`); si se empareja hereda su `key` y su `NodeId`, así que lo que
// cuelgue de ella (un reporte de G2, por ejemplo) sigue apuntando a la
// comunidad correcta. Con `min_jaccard > 0.5` el emparejamiento es único (dos
// comunidades disjuntas no pueden compartir más de la mitad con una tercera);
// por debajo, se asigna en orden de Jaccard descendente con desempate por key
// y por el menor miembro, así que es determinista. Una comunidad sin pareja
// recibe una key derivada de sus miembros (hash de los `NodeId` ordenados):
// la misma partición da las mismas keys, sin importar el orden de inserción.
//
// Descartado: key = id de la comunidad en la partición (`0..k`). Cambia con
// cualquier recálculo aunque la comunidad sea la misma, que es justo lo que
// las keys estables deben evitar.
//
// Reportes de comunidad (#191). La base no llama al LLM; da la convención y
// lo que sabe calcular:
//
//   (:Report {community_key, partition, level, title, summary, rating,
//             generated_at, source_version})-[:SUMMARIZES]->(:Community)
//
// `source_version` es la HUELLA del contenido que el reporte resume
// (`community_fingerprint`): miembros con su etiqueta y propiedades, y las
// aristas internas (entre miembros) con su tipo y propiedades. No solo el
// conjunto de miembros: una arista nueva dentro de una comunidad puede no
// moverla de partición y aun así cambia lo que su reporte debería decir. Una
// arista interna de C también es interna a todos sus ancestros (las
// comunidades gruesas contienen a C), así que esos quedan viejos con ella, y
// las hermanas no. `stale_reports` compara cada huella actual con la del
// reporte.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::error::{NopalError, Result};
use crate::types::{Edge, EdgeId, Node, NodeId, PropertyValue};

use super::Graph;

/// Etiqueta de los nodos de comunidad.
pub const COMMUNITY_LABEL: &str = "Community";
/// Arista miembro → comunidad (una por nivel).
pub const IN_COMMUNITY: &str = "IN_COMMUNITY";
/// Arista comunidad del nivel L → comunidad del nivel L+1 que contiene.
pub const PARENT_OF: &str = "PARENT_OF";

/// Opciones de [`Graph::materialize_communities`].
#[derive(Debug, Clone)]
pub struct CommunityMaterializeOptions {
    /// Nombre de la partición: varias jerarquías (otro `gamma`, otro alcance)
    /// conviven sin pisarse. Default: `"leiden"`.
    pub partition: String,
    /// Solapamiento mínimo (Jaccard, en `(0, 1]`) para que una comunidad nueva
    /// herede la key de una previa. Default: 0.5.
    pub min_jaccard: f64,
}

impl Default for CommunityMaterializeOptions {
    fn default() -> Self {
        CommunityMaterializeOptions { partition: "leiden".to_string(), min_jaccard: 0.5 }
    }
}

/// Qué hizo [`Graph::materialize_communities`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommunityMaterializeReport {
    /// Comunidades de la jerarquía nueva, todos los niveles.
    pub communities: usize,
    /// Comunidades nuevas que heredaron la key de una previa.
    pub kept_keys: usize,
    pub created: usize,
    /// Comunidades emparejadas cuyo `size` cambió.
    pub updated: usize,
    /// Comunidades previas sin pareja, borradas con sus aristas.
    pub deleted: usize,
    pub memberships_added: usize,
    pub memberships_removed: usize,
    pub parent_links_added: usize,
    pub parent_links_removed: usize,
}

impl CommunityMaterializeReport {
    /// `true` si no hubo que escribir nada.
    pub fn is_unchanged(&self) -> bool {
        self.created + self.updated + self.deleted == 0
            && self.memberships_added + self.memberships_removed == 0
            && self.parent_links_added + self.parent_links_removed == 0
    }
}

/// Etiqueta de los reportes de comunidad (#191).
pub const REPORT_LABEL: &str = "Report";
/// Arista reporte → comunidad que resume.
pub const SUMMARIZES: &str = "SUMMARIZES";

/// Contenido de un reporte de comunidad, el que produce el LLM del cliente.
#[derive(Debug, Clone, Default)]
pub struct CommunityReport {
    pub title: String,
    pub summary: String,
    /// Importancia que el LLM le asigna (convención de GraphRAG: 0–10).
    pub rating: Option<f64>,
    /// `(vector, modelo)` del resumen, para buscar reportes por similitud.
    pub embedding: Option<(Vec<f32>, String)>,
}

/// Por qué [`Graph::stale_reports`] lista una comunidad o un reporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReportStatus {
    /// La comunidad no tiene reporte.
    Missing,
    /// El contenido de la comunidad cambió desde que se escribió su reporte.
    Stale,
    /// El reporte apunta a una comunidad que ya no existe.
    Orphan,
}

impl ReportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReportStatus::Missing => "missing",
            ReportStatus::Stale => "stale",
            ReportStatus::Orphan => "orphan",
        }
    }
}

/// Una entrada de [`Graph::stale_reports`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleReport {
    pub status: ReportStatus,
    pub community_key: String,
    pub level: usize,
    /// La comunidad (`None` si es huérfano).
    pub community: Option<NodeId>,
    /// El reporte (`None` si falta).
    pub report: Option<NodeId>,
}

/// Una comunidad ya persistida.
struct Stored {
    id: NodeId,
    level: usize,
    key: String,
    size: i64,
    /// Miembro → arista `IN_COMMUNITY`.
    members: BTreeMap<NodeId, EdgeId>,
}

impl Graph {
    /// Persiste una jerarquía de comunidades con keys estables (#190 c).
    ///
    /// `levels[0]` es el nivel más grueso; cada mapa es `NodeId → comunidad`
    /// y todos cubren los mismos nodos. Cada comunidad del nivel L+1 debe
    /// caer dentro de una sola del nivel L (lo que devuelve
    /// `LeidenCommunity::detect_hierarchy`). Ver el comentario del módulo.
    pub async fn materialize_communities(
        &self,
        levels: &[HashMap<NodeId, usize>],
        options: &CommunityMaterializeOptions,
    ) -> Result<CommunityMaterializeReport> {
        let invalid = |msg: String| Err(NopalError::custom(format!("materialize_communities: {msg}")));
        if !(options.min_jaccard > 0.0 && options.min_jaccard <= 1.0) {
            return invalid("min_jaccard must be in (0, 1]".into());
        }
        if options.partition.is_empty() {
            return invalid("partition must not be empty".into());
        }
        let wanted = match desired_levels(levels) {
            Ok(w) => w,
            Err(msg) => return invalid(msg),
        };
        if let Some(first) = wanted.first() {
            let ids: Vec<NodeId> = first.iter().flatten().copied().collect();
            let missing = self.get_nodes(&ids).await?.iter().filter(|n| n.is_none()).count();
            if missing > 0 {
                return invalid(format!("{missing} member node(s) do not exist"));
            }
        }

        let stored = self.stored_communities(&options.partition).await?;
        let by_id: HashMap<NodeId, usize> = stored.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let parent_edges: HashMap<(NodeId, NodeId), EdgeId> = self
            .get_edges_by_label(PARENT_OF)
            .await?
            .into_iter()
            .filter(|e| by_id.contains_key(&e.source) && by_id.contains_key(&e.target))
            .map(|e| ((e.source, e.target), e.id))
            .collect();

        let mut report = CommunityMaterializeReport::default();
        let mut tx = self.begin_transaction().await?;
        let mut matched_stored: HashSet<usize> = HashSet::new();
        // Nodo de cada comunidad nueva, por nivel (para PARENT_OF).
        let mut node_of: Vec<Vec<NodeId>> = Vec::with_capacity(wanted.len());

        for (level, communities) in wanted.iter().enumerate() {
            report.communities += communities.len();
            let candidates: Vec<usize> = (0..stored.len()).filter(|&i| stored[i].level == level).collect();
            let pairing = pair_by_jaccard(communities, &stored, &candidates, options.min_jaccard);
            let mut ids = Vec::with_capacity(communities.len());
            for (c, members) in communities.iter().enumerate() {
                let size = members.len() as i64;
                let id = match pairing[c] {
                    Some(s) => {
                        let old = &stored[s];
                        matched_stored.insert(s);
                        report.kept_keys += 1;
                        if old.size != size {
                            report.updated += 1;
                            tx.add_node(community_node(old.id, &options.partition, level, &old.key, size)).await?;
                        }
                        for (member, edge) in &old.members {
                            if !members.contains(member) {
                                tx.delete_edge(*edge)?;
                                report.memberships_removed += 1;
                            }
                        }
                        for member in members.iter().filter(|m| !old.members.contains_key(m)) {
                            tx.add_edge(Edge::new(*member, old.id, IN_COMMUNITY))?;
                            report.memberships_added += 1;
                        }
                        old.id
                    }
                    None => {
                        let id = Node::new(COMMUNITY_LABEL).id;
                        let key = derived_key(&options.partition, level, members);
                        report.created += 1;
                        tx.add_node(community_node(id, &options.partition, level, &key, size)).await?;
                        for member in members {
                            tx.add_edge(Edge::new(*member, id, IN_COMMUNITY))?;
                            report.memberships_added += 1;
                        }
                        id
                    }
                };
                ids.push(id);
            }
            node_of.push(ids);
        }

        // Comunidades previas sin pareja: se borran (el borrado del nodo se
        // lleva sus aristas).
        let deleted: HashSet<NodeId> = (0..stored.len())
            .filter(|i| !matched_stored.contains(i))
            .map(|i| stored[i].id)
            .collect();
        for id in &deleted {
            tx.delete_node(*id)?;
        }
        report.deleted = deleted.len();

        let mut wanted_parents: BTreeSet<(NodeId, NodeId)> = BTreeSet::new();
        for level in 1..wanted.len() {
            let parent_of = parents(&wanted[level - 1], &wanted[level]);
            for (child, parent) in parent_of.into_iter().enumerate() {
                wanted_parents.insert((node_of[level - 1][parent], node_of[level][child]));
            }
        }
        for (pair, edge) in &parent_edges {
            if !wanted_parents.contains(pair) && !deleted.contains(&pair.0) && !deleted.contains(&pair.1) {
                tx.delete_edge(*edge)?;
                report.parent_links_removed += 1;
            }
        }
        for (parent, child) in &wanted_parents {
            if !parent_edges.contains_key(&(*parent, *child)) {
                tx.add_edge(Edge::new(*parent, *child, PARENT_OF))?;
                report.parent_links_added += 1;
            }
        }

        if report.is_unchanged() {
            tx.rollback()?;
        } else {
            tx.commit().await?;
        }
        Ok(report)
    }

    /// La huella actual del contenido de la comunidad `community_key`: lo que
    /// [`Self::upsert_community_report`] guarda como `source_version`. Ver
    /// el comentario del módulo.
    pub async fn community_fingerprint(&self, community_key: &str) -> Result<String> {
        let community = self.community_by_key(community_key).await?;
        let members: BTreeSet<NodeId> = self
            .get_incoming_edges(community.id)
            .await?
            .into_iter()
            .filter(|e| e.edge_type == IN_COMMUNITY)
            .map(|e| e.source)
            .collect();
        let mut reader = ContentReader::default();
        reader.fingerprint(self, &members).await
    }

    /// Escribe (o reescribe) el reporte de la comunidad `community_key`
    /// (#191): un upsert de `(:Report {community_key, ...})` con la arista
    /// `SUMMARIZES` y `source_version` = la huella actual de la comunidad.
    /// Un reporte por comunidad: reescribir reemplaza el anterior.
    pub async fn upsert_community_report(
        &self,
        community_key: &str,
        report: CommunityReport,
    ) -> Result<(super::upsert::UpsertOutcome, NodeId)> {
        let community = self.community_by_key(community_key).await?;
        let version = self.community_fingerprint(community_key).await?;
        let prop = |name: &str| community.properties.get(name).cloned().unwrap_or(PropertyValue::Null);
        let generated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let props = HashMap::from([
            ("community_key".to_string(), PropertyValue::String(community_key.to_string())),
            ("partition".to_string(), prop("partition")),
            ("level".to_string(), prop("level")),
            ("title".to_string(), PropertyValue::String(report.title)),
            ("summary".to_string(), PropertyValue::String(report.summary)),
            ("rating".to_string(), report.rating.map_or(PropertyValue::Null, PropertyValue::Float)),
            ("generated_at".to_string(), PropertyValue::Int(generated_at)),
            ("source_version".to_string(), PropertyValue::String(version)),
        ]);
        self.upsert_node(super::upsert::UpsertRequest {
            label: REPORT_LABEL.to_string(),
            key: "community_key".to_string(),
            props,
            embedding: report.embedding,
            links: vec![super::upsert::LinkSpec {
                edge_type: SUMMARIZES.to_string(),
                target_label: COMMUNITY_LABEL.to_string(),
                target_key: "key".to_string(),
                target_key_value: PropertyValue::String(community_key.to_string()),
                props: HashMap::new(),
                create_target_stub: false,
            }],
        })
        .await
    }

    /// Las comunidades de `partition` (de `level`, o de todos los niveles)
    /// sin reporte o con un reporte viejo, y los reportes huérfanos (su
    /// comunidad ya no existe), ordenados por nivel y key (#191). Una
    /// comunidad con su reporte al día no aparece.
    pub async fn stale_reports(&self, partition: &str, level: Option<usize>) -> Result<Vec<StaleReport>> {
        let communities = self.stored_communities(partition).await?;
        let mut reports: HashMap<String, (NodeId, usize, Option<String>)> = HashMap::new();
        for node in self.get_nodes_by_label(REPORT_LABEL).await? {
            if node.properties.get("partition").and_then(PropertyValue::as_str) != Some(partition) {
                continue;
            }
            let Some(key) = node.properties.get("community_key").and_then(PropertyValue::as_str) else {
                continue;
            };
            let report_level = node.properties.get("level").and_then(PropertyValue::as_i64).unwrap_or(-1);
            let version = node.properties.get("source_version").and_then(PropertyValue::as_str).map(str::to_string);
            reports.insert(key.to_string(), (node.id, usize::try_from(report_level).unwrap_or(usize::MAX), version));
        }
        let mut out = Vec::new();
        let mut reader = ContentReader::default();
        for c in communities.iter().filter(|c| level.is_none_or(|l| c.level == l)) {
            let report = reports.get(&c.key);
            let status = match report {
                None => Some(ReportStatus::Missing),
                Some((_, _, version)) => {
                    let members: BTreeSet<NodeId> = c.members.keys().copied().collect();
                    let current = reader.fingerprint(self, &members).await?;
                    (version.as_deref() != Some(current.as_str())).then_some(ReportStatus::Stale)
                }
            };
            if let Some(status) = status {
                out.push(StaleReport {
                    status,
                    community_key: c.key.clone(),
                    level: c.level,
                    community: Some(c.id),
                    report: report.map(|r| r.0),
                });
            }
        }
        let live: HashSet<&str> = communities.iter().map(|c| c.key.as_str()).collect();
        for (key, (id, report_level, _)) in &reports {
            if !live.contains(key.as_str()) && level.is_none_or(|l| *report_level == l) {
                out.push(StaleReport {
                    status: ReportStatus::Orphan,
                    community_key: key.clone(),
                    level: *report_level,
                    community: None,
                    report: Some(*id),
                });
            }
        }
        out.sort_by(|a, b| (a.level, &a.community_key, a.status).cmp(&(b.level, &b.community_key, b.status)));
        Ok(out)
    }

    async fn community_by_key(&self, community_key: &str) -> Result<Node> {
        let value = PropertyValue::String(community_key.to_string());
        for id in self.find_nodes_by_property("key", &value).await? {
            let node = self.get_node(id).await?;
            if node.label == COMMUNITY_LABEL {
                return Ok(node);
            }
        }
        Err(NopalError::custom(format!("community '{community_key}' not found")))
    }

    /// Las comunidades ya persistidas de `partition`, con sus miembros.
    async fn stored_communities(&self, partition: &str) -> Result<Vec<Stored>> {
        let mut stored: Vec<Stored> = Vec::new();
        for node in self.get_nodes_by_label(COMMUNITY_LABEL).await? {
            if node.properties.get("partition").and_then(PropertyValue::as_str) != Some(partition) {
                continue;
            }
            let (Some(level), Some(key)) = (
                node.properties.get("level").and_then(PropertyValue::as_i64),
                node.properties.get("key").and_then(PropertyValue::as_str),
            ) else {
                continue;
            };
            let size = node.properties.get("size").and_then(PropertyValue::as_i64).unwrap_or(-1);
            let level = usize::try_from(level).unwrap_or(usize::MAX);
            stored.push(Stored { id: node.id, level, key: key.to_string(), size, members: BTreeMap::new() });
        }
        stored.sort_by(|a, b| (a.level, &a.key).cmp(&(b.level, &b.key)));
        let index: HashMap<NodeId, usize> = stored.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        for edge in self.get_edges_by_label(IN_COMMUNITY).await? {
            if let Some(&i) = index.get(&edge.target) {
                stored[i].members.insert(edge.source, edge.id);
            }
        }
        Ok(stored)
    }
}

fn community_node(id: NodeId, partition: &str, level: usize, key: &str, size: i64) -> Node {
    Node::with_id(id, COMMUNITY_LABEL)
        .with_property("partition", partition)
        .with_property("level", level as i64)
        .with_property("key", key)
        .with_property("size", size)
}

/// Los niveles como listas de comunidades (miembros ordenados), con las
/// comunidades en orden de su menor miembro. Valida que todos los niveles
/// cubran los mismos nodos y que estén anidados.
fn desired_levels(levels: &[HashMap<NodeId, usize>]) -> std::result::Result<Vec<Vec<BTreeSet<NodeId>>>, String> {
    let mut out: Vec<Vec<BTreeSet<NodeId>>> = Vec::with_capacity(levels.len());
    for (l, level) in levels.iter().enumerate() {
        if l > 0 && level.len() != levels[0].len() {
            return Err(format!("level {l} has {} nodes, level 0 has {}", level.len(), levels[0].len()));
        }
        let mut groups: HashMap<usize, BTreeSet<NodeId>> = HashMap::new();
        for (node, c) in level {
            if l > 0 && !levels[0].contains_key(node) {
                return Err(format!("node {node} is in level {l} but not in level 0"));
            }
            groups.entry(*c).or_default().insert(*node);
        }
        let mut communities: Vec<BTreeSet<NodeId>> = groups.into_values().collect();
        communities.sort_by_key(|m| *m.first().expect("non-empty community"));
        if l > 0 {
            let coarse = &levels[l - 1];
            for members in &communities {
                let mut parents = members.iter().map(|m| coarse[m]);
                let first = parents.next().expect("non-empty community");
                if parents.any(|p| p != first) {
                    return Err(format!("a community of level {l} spans several communities of level {}", l - 1));
                }
            }
        }
        out.push(communities);
    }
    Ok(out)
}

/// Para cada comunidad de `fine`, el índice de la de `coarse` que la contiene.
fn parents(coarse: &[BTreeSet<NodeId>], fine: &[BTreeSet<NodeId>]) -> Vec<usize> {
    let mut owner: HashMap<NodeId, usize> = HashMap::new();
    for (c, members) in coarse.iter().enumerate() {
        for m in members {
            owner.insert(*m, c);
        }
    }
    fine.iter().map(|members| owner[members.first().expect("non-empty community")]).collect()
}

/// Empareja cada comunidad nueva con a lo sumo una previa de `candidates`
/// (Jaccard ≥ `min_jaccard`), de mayor a menor Jaccard; empates por key de
/// la previa y luego por el menor miembro de la nueva.
fn pair_by_jaccard(
    communities: &[BTreeSet<NodeId>],
    stored: &[Stored],
    candidates: &[usize],
    min_jaccard: f64,
) -> Vec<Option<usize>> {
    let mut owner: HashMap<NodeId, usize> = HashMap::new();
    for &s in candidates {
        for m in stored[s].members.keys() {
            owner.insert(*m, s);
        }
    }
    let mut pairs: Vec<(f64, &str, usize, usize)> = Vec::new();
    for (c, members) in communities.iter().enumerate() {
        let mut overlap: BTreeMap<usize, usize> = BTreeMap::new();
        for m in members {
            if let Some(&s) = owner.get(m) {
                *overlap.entry(s).or_insert(0) += 1;
            }
        }
        for (s, inter) in overlap {
            let union = members.len() + stored[s].members.len() - inter;
            let jaccard = inter as f64 / union as f64;
            if jaccard >= min_jaccard {
                pairs.push((jaccard, &stored[s].key, c, s));
            }
        }
    }
    // `c` ya está en orden del menor miembro (ver `desired_levels`).
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)).then_with(|| a.2.cmp(&b.2)));
    let mut pairing = vec![None; communities.len()];
    let mut taken: HashSet<usize> = HashSet::new();
    for (_, _, c, s) in pairs {
        if pairing[c].is_none() && !taken.contains(&s) {
            pairing[c] = Some(s);
            taken.insert(s);
        }
    }
    pairing
}

/// Lee el contenido de las comunidades para su huella, con caché de nodos y
/// de aristas salientes: en `stale_reports` un nodo está en una comunidad
/// por nivel y se lee una sola vez.
#[derive(Default)]
struct ContentReader {
    nodes: HashMap<NodeId, Option<Node>>,
    outgoing: HashMap<NodeId, Vec<Edge>>,
}

impl ContentReader {
    /// Huella de los miembros (etiqueta y propiedades) y de las aristas
    /// entre ellos (tipo y propiedades), en orden canónico: FNV-1a de 128
    /// bits, en hexadecimal. No entran los ids de arista: borrar y volver a
    /// crear la misma relación no cambia el contenido.
    async fn fingerprint(&mut self, graph: &Graph, members: &BTreeSet<NodeId>) -> Result<String> {
        let missing: Vec<NodeId> = members.iter().filter(|m| !self.nodes.contains_key(m)).copied().collect();
        if !missing.is_empty() {
            for (id, node) in missing.iter().zip(graph.get_nodes(&missing).await?) {
                self.nodes.insert(*id, node);
            }
        }
        let mut hash = Fnv128::default();
        let mut edges: Vec<Vec<u8>> = Vec::new();
        for member in members {
            hash.write(member.as_bytes());
            if let Some(Some(node)) = self.nodes.get(member) {
                hash.write(node.label.as_bytes());
                hash.write(&canonical_properties(&node.properties));
            }
            if !self.outgoing.contains_key(member) {
                let out = graph.get_outgoing_edges(*member).await?;
                self.outgoing.insert(*member, out);
            }
            for edge in &self.outgoing[member] {
                if members.contains(&edge.target) {
                    let mut bytes = Vec::new();
                    bytes.extend_from_slice(edge.source.as_bytes());
                    bytes.extend_from_slice(edge.target.as_bytes());
                    bytes.extend_from_slice(edge.edge_type.as_bytes());
                    bytes.push(0);
                    bytes.extend_from_slice(&canonical_properties(&edge.properties));
                    edges.push(bytes);
                }
            }
        }
        edges.sort();
        hash.write(b"edges");
        for edge in &edges {
            hash.write(edge);
        }
        Ok(format!("{:032x}", hash.0))
    }
}

/// Propiedades en orden de clave, cada valor como JSON: bytes estables.
fn canonical_properties(props: &HashMap<String, PropertyValue>) -> Vec<u8> {
    let sorted: BTreeMap<&String, &PropertyValue> = props.iter().collect();
    let mut out = Vec::new();
    for (key, value) in sorted {
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(&serde_json::to_vec(value).unwrap_or_default());
        out.push(0);
    }
    out
}

/// FNV-1a de 128 bits: estable entre versiones de Rust, a diferencia de
/// `DefaultHasher`.
struct Fnv128(u128);

impl Default for Fnv128 {
    fn default() -> Self {
        Fnv128(0x6c62_272e_07bb_0142_62b8_2175_6295_c58d)
    }
}

impl Fnv128 {
    fn write(&mut self, bytes: &[u8]) {
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
        for byte in bytes {
            self.0 ^= u128::from(*byte);
            self.0 = self.0.wrapping_mul(PRIME);
        }
        // Separador: ("ab","c") y ("a","bc") no deben dar lo mismo.
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(PRIME);
    }
}

/// Key de una comunidad sin pareja: `partition/L<level>/<hash>`, con el hash
/// (FNV-1a de 128 bits) de sus miembros ordenados. Estable entre versiones
/// de Rust, a diferencia de `DefaultHasher`.
fn derived_key(partition: &str, level: usize, members: &BTreeSet<NodeId>) -> String {
    // Bytes contiguos de los ids, sin separadores: así se calculaba desde
    // 0.6.11 (#190 c) y las keys ya persistidas no deben cambiar.
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
    let mut hash: u128 = Fnv128::default().0;
    for member in members {
        for byte in member.as_bytes() {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{partition}/L{level}/{hash:032x}")
}
