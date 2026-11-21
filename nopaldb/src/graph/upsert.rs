// Idempotent upsert primitive (issue M1-4).
//
// `upsert_node` writes a "desired state" for a node identified by a business
// key `(label, key_property, value)` — create if absent, update if changed,
// no-op if identical. This is the write primitive incremental pipelines and
// second-brain ingestion need: re-running the same upsert over unchanged data
// costs zero writes.
//
// Design notes (see the module tests and issue M1-4):
//   * Identity lookup uses `Transaction::get_nodes_by_label_and_property`,
//     which since 0.6.3 resolves through the property index (O(matches));
//     before that it deserialized every node of the database per row, so a
//     batch was quadratic in the size of the graph (#151).
//   * `upsert_batch` runs one transaction per chunk of `UPSERT_TX_ROWS` rows
//     (one WAL fsync per chunk instead of one per row). Rows of a chunk are
//     atomic together; rows that repeat a key inside a chunk see the earlier
//     row's node (the same NodeId, no duplicate). Per-key locks of a chunk are
//     taken in hash order so two concurrent batches cannot deadlock.
//   * Update overwrites the node under its existing NodeId (re-adding the same
//     id in a tx overwrites on commit). Retracting the entries the overwrite
//     invalidates is the applier's job — it reads the previous node before
//     `insert_node` replaces it, which is the only moment the old values still
//     exist. This module used to reconcile the diff itself, post-commit; that
//     workaround was narrower than the bug (it only reached the property index,
//     never the user indexes) and is gone now that the applier does it.
//   * A process-global per-key lock serializes concurrent upserts of the SAME
//     key so two racing creates cannot both insert. On its own it is
//     best-effort: a write that is not an upsert (`add_node`, NQL `add`, the
//     bulk loader), or a link stub (stubs take no lock), can still create the
//     key in between the lookup and the commit. A unique index on the key
//     (M1-8) is the actual constraint: the commit that would duplicate it is
//     refused before the WAL, so the upsert retries the whole row (or chunk)
//     from a fresh transaction, and the new lookup finds the winner and
//     updates it. The lock stays as an optimization, so racing upserts of one
//     key queue up instead of failing and retrying. Without a unique index the
//     retry never fires and the behavior is the old best-effort one.
//   * Embedding updates go through `add_node_embedding`, which overwrites the
//     persisted vector and updates the cached HNSW index in place: the old
//     point becomes a tombstone and the new one is inserted (#113). The index
//     is rebuilt from storage only when tombstones pass the threshold in
//     `HnswIndex::needs_rebuild`.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{NopalError, Result};
use crate::transaction::Transaction;
use crate::types::{Edge, Node, NodeId, PropertyValue};

use super::Graph;

/// Rows per transaction in [`Graph::upsert_batch`]. Bounds the write set held
/// in memory and the time the per-key locks of a chunk stay taken; a caller
/// that needs a whole batch to be atomic sends at most this many rows.
pub const UPSERT_TX_ROWS: usize = 1024;

/// Attempts of one row (or chunk) whose commit loses its key to a concurrent
/// write and is refused by a unique index. The retry after a refusal always
/// sees the winner (the applier applies it before validating any later
/// batch), so more than one retry only happens if the key keeps being
/// recreated.
const UNIQUE_KEY_ATTEMPTS: usize = 3;

/// What an upsert did to the target node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Created,
    Updated,
    Unchanged,
}

impl UpsertOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpsertOutcome::Created => "created",
            UpsertOutcome::Updated => "updated",
            UpsertOutcome::Unchanged => "unchanged",
        }
    }
}

/// A declarative outgoing edge to reconcile as part of an upsert. The target is
/// resolved by its own business key; if it does not exist and `create_target_stub`
/// is set, a stub node `{target_key: target_key_value}` is created (the Obsidian
/// wikilink pattern: link to a note that may not exist yet).
#[derive(Debug, Clone)]
pub struct LinkSpec {
    pub edge_type: String,
    pub target_label: String,
    pub target_key: String,
    pub target_key_value: PropertyValue,
    pub props: HashMap<String, PropertyValue>,
    pub create_target_stub: bool,
}

/// Desired state for a node keyed by `(label, key)`. `props` is the complete
/// desired property map and MUST contain `key`.
#[derive(Debug, Clone)]
pub struct UpsertRequest {
    pub label: String,
    pub key: String,
    pub props: HashMap<String, PropertyValue>,
    /// Optional `(vector, model)` embedding to attach/refresh.
    pub embedding: Option<(Vec<f32>, String)>,
    pub links: Vec<LinkSpec>,
}

/// A node of [`Graph::upsert_pattern`]: its label and the properties (under
/// a unique index) it is found by.
#[derive(Debug, Clone)]
pub(crate) struct UpsertNode {
    pub label: String,
    pub identity: Vec<(String, PropertyValue)>,
}

/// An edge of [`Graph::upsert_pattern`], between two of its nodes (indexes
/// into the pattern's nodes), found by `(source, edge_type, target)`.
#[derive(Debug, Clone)]
pub(crate) struct UpsertEdge {
    pub source: usize,
    pub edge_type: String,
    pub target: usize,
}

/// The property maps the nodes and edges of a pattern must end up with, in
/// the pattern's order.
pub(crate) struct UpsertMaps {
    pub nodes: Vec<HashMap<String, PropertyValue>>,
    pub edges: Vec<HashMap<String, PropertyValue>>,
}

/// What [`Graph::upsert_pattern`] did to each node and edge, in order.
#[derive(Debug, Clone, Default)]
pub(crate) struct UpsertPatternOutcome {
    pub nodes: Vec<(UpsertOutcome, NodeId)>,
    pub edges: Vec<(UpsertOutcome, crate::types::EdgeId)>,
}

/// Process-global map of per-key locks. Keyed by a hash of `(label, key, value)`
/// so different keys never contend; collisions across databases only serialize
/// a little extra, which is harmless.
fn key_locks() -> &'static Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<u64, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key_lock_id(label: &str, key: &str, value: &PropertyValue) -> u64 {
    let mut h = DefaultHasher::new();
    label.hash(&mut h);
    key.hash(&mut h);
    // PropertyValue isn't Hash; hash its debug form — stable enough for a lock key.
    format!("{value:?}").hash(&mut h);
    h.finish()
}

fn key_lock_by_id(id: u64) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = key_locks().lock().unwrap();
    map.entry(id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
}

fn key_lock_for(label: &str, key: &str, value: &PropertyValue) -> Arc<tokio::sync::Mutex<()>> {
    key_lock_by_id(key_lock_id(label, key, value))
}

/// Exact identity of a business key inside one batch (no hashing: a collision
/// here would silently merge two rows into one node).
fn key_id(label: &str, key: &str, value: &PropertyValue) -> String {
    format!("{label}\0{key}\0{value:?}")
}

/// What one row did inside the transaction, before commit.
struct RowPlan {
    outcome: UpsertOutcome,
    node_id: NodeId,
    /// The row added something to the tx (node, edge) — or its embedding
    /// changed, which is written after commit.
    wrote: bool,
    embedding_changed: bool,
}

/// Nodes and edges this transaction has already added, so later rows of the
/// same chunk resolve against them instead of the committed state.
#[derive(Default)]
struct TxState {
    /// key id → (node, its props as written in this tx)
    nodes: HashMap<String, (NodeId, HashMap<String, PropertyValue>)>,
    edges: HashSet<(NodeId, String, NodeId)>,
}

impl Graph {
    /// Idempotently write the desired state of a node keyed by `(label, key)`.
    /// Returns the outcome and the node's id.
    pub async fn upsert_node(&self, req: UpsertRequest) -> Result<(UpsertOutcome, NodeId)> {
        let key_value = req.key_value()?;

        // Serialize concurrent upserts of the same business key.
        let lock = key_lock_for(&req.label, &req.key, &key_value);
        let _guard = lock.lock().await;

        let retry = self.keys_are_unique(std::slice::from_ref(&req)).await;
        let mut attempt = 1;
        loop {
            let mut tx = self.begin_transaction().await?;
            let mut state = TxState::default();
            let plan = self.upsert_in_tx(&mut tx, &req, &mut state).await?;

            if !plan.wrote {
                // Nothing to write. The tx has not touched the WAL (it writes at
                // commit), so the sync rollback ends it without the `Abort`
                // record `rollback_async` would append: an unchanged re-run
                // costs zero WAL records.
                tx.rollback()?;
                return Ok((UpsertOutcome::Unchanged, plan.node_id));
            }
            match tx.commit().await {
                Err(NopalError::UniqueViolation(_)) if retry && attempt < UNIQUE_KEY_ATTEMPTS => {
                    attempt += 1;
                    continue;
                }
                r => r?,
            }
            self.apply_embedding_after_commit(&req, &plan).await?;
            return Ok((plan.final_outcome(), plan.node_id));
        }
    }

    /// Whether a key `reqs` resolve (a row's own, or a link target it may
    /// create a stub for) has a unique index, so a commit refused by one of
    /// them can be retried and resolve to the winner. A refusal by a unique
    /// index on any other property would repeat on every attempt.
    async fn keys_are_unique(&self, reqs: &[UpsertRequest]) -> bool {
        if !self.index_manager.has_unique() {
            return false;
        }
        for req in reqs {
            let stubs = req.links.iter().filter(|l| l.create_target_stub);
            let keys = stubs.map(|l| (&l.target_label, &l.target_key));
            for (label, key) in std::iter::once((&req.label, &req.key)).chain(keys) {
                let unique = self.index_manager.unique_properties(label).await;
                if unique.iter().any(|(_, p)| p == key) {
                    return true;
                }
            }
        }
        false
    }

    /// Upsert many nodes: one transaction (one WAL fsync) per chunk of
    /// [`UPSERT_TX_ROWS`] rows, results per row in input order. A chunk is
    /// atomic: if a row fails, none of its chunk is written and the error is
    /// returned (earlier chunks stay committed). Repeating a key inside a
    /// batch updates the row created earlier in the batch, never duplicates it.
    pub async fn upsert_batch(
        &self,
        reqs: Vec<UpsertRequest>,
    ) -> Result<Vec<(UpsertOutcome, NodeId)>> {
        let mut out = Vec::with_capacity(reqs.len());
        let mut progress = self.ops.reporter("upsert_batch", Some(reqs.len() as u64));
        for chunk in reqs.chunks(UPSERT_TX_ROWS) {
            progress.tick(out.len() as u64);
            // Per-key locks of the chunk, distinct and in hash order, so two
            // concurrent batches sharing keys cannot take them crosswise.
            let mut ids: Vec<u64> = chunk
                .iter()
                .map(|r| Ok(key_lock_id(&r.label, &r.key, &r.key_value()?)))
                .collect::<Result<_>>()?;
            ids.sort_unstable();
            ids.dedup();
            let mut guards = Vec::with_capacity(ids.len());
            for id in ids {
                guards.push(key_lock_by_id(id).lock_owned().await);
            }

            let retry = self.keys_are_unique(chunk).await;
            let mut attempt = 1;
            let plans = loop {
                let mut tx = self.begin_transaction().await?;
                let mut state = TxState::default();
                let mut plans = Vec::with_capacity(chunk.len());
                for req in chunk {
                    plans.push(self.upsert_in_tx(&mut tx, req, &mut state).await?);
                }

                if !plans.iter().any(|p| p.wrote) {
                    // Sync rollback: no `Abort` record, as in `upsert_node`.
                    tx.rollback()?;
                    break plans;
                }
                match tx.commit().await {
                    Err(NopalError::UniqueViolation(_)) if retry && attempt < UNIQUE_KEY_ATTEMPTS => {
                        attempt += 1;
                        continue;
                    }
                    r => r?,
                }
                for (req, plan) in chunk.iter().zip(&plans) {
                    self.apply_embedding_after_commit(req, plan).await?;
                }
                break plans;
            };
            out.extend(plans.iter().map(|p| (p.final_outcome(), p.node_id)));
            drop(guards);
        }
        progress.finish(out.len() as u64);
        Ok(out)
    }

    /// One row of an upsert inside `tx`: resolve the identity (first against
    /// what this tx already wrote, then against the committed state through
    /// the property index), then add the node and its missing links.
    async fn upsert_in_tx(
        &self,
        tx: &mut Transaction,
        req: &UpsertRequest,
        state: &mut TxState,
    ) -> Result<RowPlan> {
        let key_value = req.key_value()?;
        let kid = key_id(&req.label, &req.key, &key_value);

        let (outcome, node_id) = if let Some((id, props_in_tx)) = state.nodes.get(&kid) {
            let id = *id;
            if *props_in_tx == req.props {
                (UpsertOutcome::Unchanged, id)
            } else {
                let node = Node::with_id(id, req.label.clone()).with_properties(req.props.clone());
                tx.add_node(node).await?;
                (UpsertOutcome::Updated, id)
            }
        } else {
            let existing = tx
                .get_nodes_by_label_and_property(&req.label, &req.key, &key_value)
                .await?;
            match existing.len() {
                0 => {
                    let node = Node::with_id(crate::types::fresh_id(), req.label.clone())
                        .with_properties(req.props.clone());
                    let id = node.id;
                    tx.add_node(node).await?;
                    (UpsertOutcome::Created, id)
                }
                1 => {
                    let old = &existing[0];
                    if old.properties == req.props {
                        (UpsertOutcome::Unchanged, old.id)
                    } else {
                        let node = Node::with_id(old.id, req.label.clone())
                            .with_properties(req.props.clone());
                        tx.add_node(node).await?;
                        (UpsertOutcome::Updated, old.id)
                    }
                }
                n => {
                    return Err(NopalError::AmbiguousUpsertKey(format!(
                        "{n} nodes match {}.{}={:?}; deduplicate before upserting",
                        req.label, req.key, key_value
                    )));
                }
            }
        };
        state.nodes.insert(kid, (node_id, req.props.clone()));

        // Resolve and reconcile links inside the same transaction. Only missing
        // edges are added (v1 does not delete edges — follow-up M1-4c).
        let mut links_added = 0usize;
        // Existing outgoing edges of the target node (committed state). For a
        // node created in this tx this is empty.
        let existing_edges = if outcome == UpsertOutcome::Created {
            Vec::new()
        } else {
            self.get_outgoing_edges(node_id).await?
        };
        for link in &req.links {
            let target_id = self.resolve_or_stub_target(tx, link, state).await?;
            let already = existing_edges
                .iter()
                .any(|e| e.edge_type == link.edge_type && e.target == target_id)
                || state
                    .edges
                    .contains(&(node_id, link.edge_type.clone(), target_id));
            if !already {
                let mut edge = Edge::new(node_id, target_id, link.edge_type.clone());
                edge.properties = link.props.clone();
                tx.add_edge(edge)?;
                state
                    .edges
                    .insert((node_id, link.edge_type.clone(), target_id));
                links_added += 1;
            }
        }

        let embedding_changed = self.embedding_differs(node_id, &req.embedding).await;
        let wrote = outcome != UpsertOutcome::Unchanged || links_added > 0 || embedding_changed;
        Ok(RowPlan { outcome, node_id, wrote, embedding_changed })
    }

    /// Attach/refresh the embedding if provided and changed. Runs after the
    /// commit: `add_node_embedding` writes the vector and invalidates the
    /// cached HNSW index on its own.
    #[allow(unused_variables)]
    async fn apply_embedding_after_commit(&self, req: &UpsertRequest, plan: &RowPlan) -> Result<()> {
        if !plan.embedding_changed {
            return Ok(());
        }
        #[cfg(feature = "embeddings")]
        if let Some((vector, model)) = &req.embedding {
            self.add_node_embedding(plan.node_id, vector.clone(), model).await?;
        }
        Ok(())
    }

    /// The write core of NQL `upsert`: find or create each node of a pattern
    /// by its `identity` (properties under a unique index of its label), then
    /// find or create each edge by `(source, type, target)`, all in one
    /// transaction. `desired` gets every node and edge as found (`true`) or as
    /// it would be created (a new empty node or edge with the id it will get),
    /// and returns the whole property map each must end up with.
    ///
    /// Unlike [`Graph::upsert_node`], whose `props` replace the whole map, the
    /// caller decides what survives (NQL keeps the existing properties and
    /// applies its `set`). The rest is shared: the per-key locks of
    /// `upsert_node`, taken for every node before the transaction in hash
    /// order, so both serialize on the same keys and two patterns cannot
    /// deadlock; nothing written when no map changes; and the retry when the
    /// commit loses a key to a concurrent write. Every identity is
    /// unique-indexed by construction, so a refusal always means a lost race.
    ///
    /// Edges have no unique index: the locks of both endpoints are what keeps
    /// two upserts from creating the same edge twice. A write that is not an
    /// upsert (`add_edge`, NQL `add`) can still add one in between.
    ///
    /// Errors with `AmbiguousUpsertKey` when the identity properties of a node
    /// point to different nodes (or one of them to several, written by a
    /// version without unique indexes), when two nodes of the pattern are the
    /// same node, or when several edges already join a pair with the type.
    pub(crate) async fn upsert_pattern(
        &self,
        nodes: &[UpsertNode],
        edges: &[UpsertEdge],
        desired: impl Fn(&[(Node, bool)], &[(Edge, bool)]) -> Result<UpsertMaps>,
    ) -> Result<UpsertPatternOutcome> {
        // Two nodes with one key would be one node created twice: the commit
        // would lose to the unique index on every attempt.
        let mut keys = HashSet::new();
        for n in nodes {
            for (k, v) in &n.identity {
                if !keys.insert(key_id(&n.label, k, v)) {
                    return Err(NopalError::AmbiguousUpsertKey(format!(
                        "two nodes of the upsert have {}.{k}={v:?}; write it once and reuse its variable",
                        n.label
                    )));
                }
            }
        }
        let mut ids: Vec<u64> = nodes
            .iter()
            .flat_map(|n| n.identity.iter().map(|(k, v)| key_lock_id(&n.label, k, v)))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut guards = Vec::with_capacity(ids.len());
        for id in ids {
            guards.push(key_lock_by_id(id).lock_owned().await);
        }

        let mut attempt = 1;
        loop {
            let mut tx = self.begin_transaction().await?;
            let mut found_nodes: Vec<(Node, bool)> = Vec::with_capacity(nodes.len());
            for n in nodes {
                let found = Self::find_by_identity(&tx, n).await?;
                if let Some(f) = &found
                    && found_nodes.iter().any(|(m, _)| m.id == f.id)
                {
                    return Err(NopalError::AmbiguousUpsertKey(format!(
                        "two nodes of the upsert are node {} ({}); write it once and reuse its variable",
                        f.id, n.label
                    )));
                }
                let exists = found.is_some();
                found_nodes.push((
                    found.unwrap_or_else(|| Node::with_id(crate::types::fresh_id(), n.label.clone())),
                    exists,
                ));
            }

            let mut found_edges: Vec<(Edge, bool)> = Vec::with_capacity(edges.len());
            for e in edges {
                let (source, target) = (&found_nodes[e.source], &found_nodes[e.target]);
                let existing = match (source.1, target.1) {
                    (true, true) => self
                        .get_outgoing_edges(source.0.id)
                        .await?
                        .into_iter()
                        .filter(|x| x.edge_type == e.edge_type && x.target == target.0.id)
                        .collect(),
                    _ => Vec::new(),
                };
                if existing.len() > 1 {
                    return Err(NopalError::AmbiguousUpsertKey(format!(
                        "{} edges {} join node {} to node {}; deduplicate before upserting",
                        existing.len(),
                        e.edge_type,
                        source.0.id,
                        target.0.id
                    )));
                }
                let found = existing.into_iter().next();
                let exists = found.is_some();
                found_edges.push((
                    found.unwrap_or_else(|| Edge::new(source.0.id, target.0.id, e.edge_type.clone())),
                    exists,
                ));
            }

            let maps = desired(&found_nodes, &found_edges)?;
            let outcome = |exists: bool, before: &HashMap<String, PropertyValue>, after: &HashMap<String, PropertyValue>| {
                match exists {
                    false => UpsertOutcome::Created,
                    true if before == after => UpsertOutcome::Unchanged,
                    true => UpsertOutcome::Updated,
                }
            };
            let mut out = UpsertPatternOutcome::default();
            for ((node, exists), props) in found_nodes.into_iter().zip(maps.nodes) {
                let o = outcome(exists, &node.properties, &props);
                if o != UpsertOutcome::Unchanged {
                    tx.add_node(Node::with_id(node.id, node.label).with_properties(props)).await?;
                }
                out.nodes.push((o, node.id));
            }
            for ((mut edge, exists), props) in found_edges.into_iter().zip(maps.edges) {
                let o = outcome(exists, &edge.properties, &props);
                if o != UpsertOutcome::Unchanged {
                    edge.properties = props;
                    tx.add_edge(edge.clone())?;
                }
                out.edges.push((o, edge.id));
            }

            if out.nodes.iter().chain(&out.edges).all(|(o, _)| *o == UpsertOutcome::Unchanged) {
                // Nothing to write. The tx has logged nothing (records go to
                // the WAL at commit), so the sync rollback ends it without the
                // `Abort` record `rollback_async` would append: an unchanged
                // upsert leaves the WAL as it was.
                tx.rollback()?;
                return Ok(out);
            }
            match tx.commit().await {
                Err(NopalError::UniqueViolation(_)) if attempt < UNIQUE_KEY_ATTEMPTS => attempt += 1,
                r => {
                    r?;
                    return Ok(out);
                }
            }
        }
    }

    /// The committed node of `n.label` that holds `n.identity`, if any. Every
    /// identity property must lead to that same node (or to none).
    async fn find_by_identity(tx: &Transaction, n: &UpsertNode) -> Result<Option<Node>> {
        let mut found: Option<Node> = None;
        for (key, value) in &n.identity {
            for node in tx.get_nodes_by_label_and_property(&n.label, key, value).await? {
                match &found {
                    Some(f) if f.id != node.id => {
                        return Err(NopalError::AmbiguousUpsertKey(format!(
                            "{}.{key}={value:?} is held by node {}, but another property of the \
                             upsert points to node {}",
                            n.label, node.id, f.id
                        )));
                    }
                    _ => found = Some(node),
                }
            }
        }
        Ok(found)
    }

    /// Delete the node identified by a business key `(label, key, value)` — the
    /// counterpart of `upsert_node` for incremental reconciliation (drop nodes
    /// whose source record disappeared). Returns the deleted `NodeId`, or `None`
    /// if no node matched (idempotent). Errors with `AmbiguousUpsertKey` if more
    /// than one node matches. Edges and index entries of the node are cleaned up
    /// by the underlying `delete_node`.
    pub async fn delete_node_by_key(
        &self,
        label: &str,
        key: &str,
        value: &PropertyValue,
    ) -> Result<Option<NodeId>> {
        // Same per-key lock as upsert, so a delete cannot race a concurrent
        // upsert of the same business key.
        let lock = key_lock_for(label, key, value);
        let _guard = lock.lock().await;

        let tx = self.begin_transaction().await?;
        let existing = tx.get_nodes_by_label_and_property(label, key, value).await?;
        // Read-only lookup; drop the transaction without committing.
        let id = match existing.len() {
            0 => return Ok(None),
            1 => existing[0].id,
            n => {
                return Err(NopalError::AmbiguousUpsertKey(format!(
                    "{n} nodes match {label}.{key}={value:?}; deduplicate before deleting"
                )));
            }
        };
        tx.rollback_async().await?;

        self.delete_node(id).await?;
        Ok(Some(id))
    }

    /// Resolve a link target by its business key — first among the nodes this
    /// tx already wrote (a row or stub earlier in the batch), then committed —
    /// creating a stub node when absent and requested.
    async fn resolve_or_stub_target(
        &self,
        tx: &mut Transaction,
        link: &LinkSpec,
        state: &mut TxState,
    ) -> Result<NodeId> {
        let kid = key_id(&link.target_label, &link.target_key, &link.target_key_value);
        if let Some((id, _)) = state.nodes.get(&kid) {
            return Ok(*id);
        }
        let found = tx
            .get_nodes_by_label_and_property(
                &link.target_label,
                &link.target_key,
                &link.target_key_value,
            )
            .await?;
        match found.len() {
            0 => {
                if link.create_target_stub {
                    let stub = Node::with_id(crate::types::fresh_id(), link.target_label.clone())
                        .with_property(link.target_key.clone(), link.target_key_value.clone());
                    let id = stub.id;
                    tx.add_node(stub.clone()).await?;
                    state.nodes.insert(kid, (id, stub.properties));
                    Ok(id)
                } else {
                    Err(NopalError::NodeNotFound(format!(
                        "link target {}.{}={:?} not found (set create_target_stub to create it)",
                        link.target_label, link.target_key, link.target_key_value
                    )))
                }
            }
            _ => Ok(found[0].id),
        }
    }

    /// True if `embedding` is Some and differs from what is currently stored
    /// (missing counts as differ). Always false when the feature is off.
    #[allow(unused_variables)]
    async fn embedding_differs(
        &self,
        node_id: NodeId,
        embedding: &Option<(Vec<f32>, String)>,
    ) -> bool {
        #[cfg(feature = "embeddings")]
        {
            if let Some((vector, model)) = embedding {
                return match self.get_node_embedding(node_id, model).await {
                    Ok(stored) => stored.vector != *vector,
                    Err(_) => true,
                };
            }
        }
        false
    }
}

impl UpsertRequest {
    /// The key property's value, or the error every upsert path reports when
    /// `props` does not carry the key.
    fn key_value(&self) -> Result<PropertyValue> {
        self.props.get(&self.key).cloned().ok_or_else(|| {
            NopalError::Custom(format!(
                "upsert: key '{}' missing from props for label '{}'",
                self.key, self.label
            ))
        })
    }
}

impl RowPlan {
    /// `Unchanged` props with new links or a new embedding is reported as
    /// `Updated`, like the single-row path always did.
    fn final_outcome(&self) -> UpsertOutcome {
        if self.outcome == UpsertOutcome::Unchanged && self.wrote {
            UpsertOutcome::Updated
        } else {
            self.outcome
        }
    }
}
