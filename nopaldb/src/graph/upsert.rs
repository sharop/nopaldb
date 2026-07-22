// Idempotent upsert primitive (issue M1-4).
//
// `upsert_node` writes a "desired state" for a node identified by a business
// key `(label, key_property, value)` — create if absent, update if changed,
// no-op if identical. This is the write primitive incremental pipelines and
// second-brain ingestion need: re-running the same upsert over unchanged data
// costs zero writes.
//
// Design notes (see the module tests and issue M1-4):
//   * Identity lookup uses `Transaction::get_nodes_by_label_and_property`.
//   * Update overwrites the node under its existing NodeId (re-adding the same
//     id in a tx overwrites on commit). The commit applier re-indexes the NEW
//     property values but does NOT drop stale entries for changed/removed keys
//     (`Storage::insert_node` is index-blind) — so we reconcile the index diff
//     here. This caller-side reconciliation is the interim fix; the structural
//     fix is to make the applier reindex on overwrite (follow-up M1-9).
//   * A process-global per-key lock serializes concurrent upserts of the SAME
//     key so two racing creates cannot both insert. This is best-effort, not a
//     transactional unique constraint (follow-up M1-8).
//   * Embedding updates go through `add_node_embedding`, which overwrites the
//     persisted vector and invalidates the cached HNSW index (rebuilt on
//     demand), sidestepping the incremental index's no-reindex rule.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{NopalError, Result};
use crate::types::{Edge, Node, NodeId, PropertyValue};

use super::Graph;

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

fn key_lock_for(label: &str, key: &str, value: &PropertyValue) -> Arc<tokio::sync::Mutex<()>> {
    let mut h = DefaultHasher::new();
    label.hash(&mut h);
    key.hash(&mut h);
    // PropertyValue isn't Hash; hash its debug form — stable enough for a lock key.
    format!("{value:?}").hash(&mut h);
    let id = h.finish();
    let mut map = key_locks().lock().unwrap();
    map.entry(id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
}

impl Graph {
    /// Idempotently write the desired state of a node keyed by `(label, key)`.
    /// Returns the outcome and the node's id.
    pub async fn upsert_node(&self, req: UpsertRequest) -> Result<(UpsertOutcome, NodeId)> {
        let key_value = req.props.get(&req.key).cloned().ok_or_else(|| {
            NopalError::Custom(format!(
                "upsert: key '{}' missing from props for label '{}'",
                req.key, req.label
            ))
        })?;

        // Serialize concurrent upserts of the same business key.
        let lock = key_lock_for(&req.label, &req.key, &key_value);
        let _guard = lock.lock().await;

        let mut tx = self.begin_transaction().await?;
        let existing = tx
            .get_nodes_by_label_and_property(&req.label, &req.key, &key_value)
            .await?;

        let (outcome, node_id, stale_index_entries) = match existing.len() {
            0 => {
                let node = Node::with_id(NodeId::new_v4(), req.label.clone())
                    .with_properties(req.props.clone());
                let id = node.id;
                tx.add_node(node).await?;
                (UpsertOutcome::Created, id, Vec::new())
            }
            1 => {
                let old = &existing[0];
                let id = old.id;
                let props_same = old.properties == req.props;

                if props_same {
                    (UpsertOutcome::Unchanged, id, Vec::new())
                } else {
                    // Stale index entries: old (key,value) pairs whose value is
                    // gone or changed. The commit applier re-adds the new ones.
                    let stale: Vec<(String, PropertyValue)> = old
                        .properties
                        .iter()
                        .filter(|(k, v)| req.props.get(*k) != Some(*v))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let node = Node::with_id(id, req.label.clone())
                        .with_properties(req.props.clone());
                    tx.add_node(node).await?;
                    (UpsertOutcome::Updated, id, stale)
                }
            }
            n => {
                return Err(NopalError::AmbiguousUpsertKey(format!(
                    "{n} nodes match {}.{}={:?}; deduplicate before upserting",
                    req.label, req.key, key_value
                )));
            }
        };

        // Resolve and reconcile links inside the same transaction. Only missing
        // edges are added (v1 does not delete edges — follow-up M1-4c).
        let mut links_added = 0usize;
        // Existing outgoing edges of the target node (committed state). For a
        // freshly created node this is empty.
        let existing_edges = if outcome == UpsertOutcome::Created {
            Vec::new()
        } else {
            self.get_outgoing_edges(node_id).await?
        };
        for link in &req.links {
            let target_id = self
                .resolve_or_stub_target(&mut tx, link)
                .await?;
            let already = existing_edges
                .iter()
                .any(|e| e.edge_type == link.edge_type && e.target == target_id);
            if !already {
                let mut edge = Edge::new(node_id, target_id, link.edge_type.clone());
                edge.properties = link.props.clone();
                tx.add_edge(edge)?;
                links_added += 1;
            }
        }

        // Decide whether an unchanged-props node is truly a no-op.
        let embedding_changed = self.embedding_differs(node_id, &req.embedding).await;
        if outcome == UpsertOutcome::Unchanged && links_added == 0 && !embedding_changed {
            // Nothing to write — abort the empty transaction so an unchanged
            // re-run costs zero WAL records.
            tx.rollback_async().await?;
            return Ok((UpsertOutcome::Unchanged, node_id));
        }

        tx.commit().await?;

        // Reconcile stale property-index entries for updated nodes (post-commit,
        // so the applier's re-add of new values has already run).
        for (prop, old_val) in &stale_index_entries {
            self.storage_remove_property_index(prop, old_val, node_id).await?;
        }

        // Attach/refresh the embedding if provided and changed.
        #[cfg(feature = "embeddings")]
        if let Some((vector, model)) = &req.embedding
            && embedding_changed
        {
            self.add_node_embedding(node_id, vector.clone(), model).await?;
        }

        let final_outcome = if outcome == UpsertOutcome::Unchanged {
            UpsertOutcome::Updated // props unchanged but links/embedding changed
        } else {
            outcome
        };
        Ok((final_outcome, node_id))
    }

    /// Upsert many nodes. v1 loops `upsert_node`; a batched fast path (one tx
    /// per batch, HNSW batch build) is follow-up M1-4b.
    pub async fn upsert_batch(
        &self,
        reqs: Vec<UpsertRequest>,
    ) -> Result<Vec<(UpsertOutcome, NodeId)>> {
        let mut out = Vec::with_capacity(reqs.len());
        for req in reqs {
            out.push(self.upsert_node(req).await?);
        }
        Ok(out)
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

    /// Resolve a link target by its business key, creating a stub node when
    /// absent and requested.
    async fn resolve_or_stub_target(
        &self,
        tx: &mut crate::transaction::Transaction,
        link: &LinkSpec,
    ) -> Result<NodeId> {
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
                    let stub = Node::with_id(NodeId::new_v4(), link.target_label.clone())
                        .with_property(link.target_key.clone(), link.target_key_value.clone());
                    let id = stub.id;
                    tx.add_node(stub).await?;
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
