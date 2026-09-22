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
//     key so two racing creates cannot both insert. This is best-effort, not a
//     transactional unique constraint (follow-up M1-8).
//   * Embedding updates go through `add_node_embedding`, which overwrites the
//     persisted vector and invalidates the cached HNSW index (rebuilt on
//     demand), sidestepping the incremental index's no-reindex rule.

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

        let mut tx = self.begin_transaction().await?;
        let mut state = TxState::default();
        let plan = self.upsert_in_tx(&mut tx, &req, &mut state).await?;

        if !plan.wrote {
            // Nothing to write — abort the empty transaction so an unchanged
            // re-run costs zero WAL records.
            tx.rollback_async().await?;
            return Ok((UpsertOutcome::Unchanged, plan.node_id));
        }
        tx.commit().await?;
        self.apply_embedding_after_commit(&req, &plan).await?;
        Ok((plan.final_outcome(), plan.node_id))
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

            let mut tx = self.begin_transaction().await?;
            let mut state = TxState::default();
            let mut plans = Vec::with_capacity(chunk.len());
            for req in chunk {
                plans.push(self.upsert_in_tx(&mut tx, req, &mut state).await?);
            }

            if plans.iter().any(|p| p.wrote) {
                tx.commit().await?;
                for (req, plan) in chunk.iter().zip(&plans) {
                    self.apply_embedding_after_commit(req, plan).await?;
                }
            } else {
                tx.rollback_async().await?;
            }
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
