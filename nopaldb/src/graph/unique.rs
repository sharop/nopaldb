// Unique indexes (M1-8).
//
// A unique index is an ordinary hash or btree user index with a flag: no two
// nodes of its label may hold the same value of its property. Nodes without
// the property, or with `null`, do not take part (as in SQL).
//
// Design notes:
//   * The check runs under the write gate, before anything reaches the WAL:
//     in the applier's validation phase for commits and direct writes, and
//     at the start of `add_nodes_batch` for the bulk loader. A rejected write
//     leaves no log record for the redo to materialize.
//   * The user index is the source of truth for the committed state. Under
//     the gate it reflects every applied write: the applier applies a whole
//     batch before releasing the gate, and the bulk loader and the index
//     build hold it too.
//   * Writes of one applier batch are validated before any of them is
//     applied, so `UniqueBatch` carries what the earlier ones will have left:
//     the last version of each node they wrote (or its deletion) and the
//     values they claimed. A value is free when every node that holds it is
//     deleted or rewritten without it by then.
//   * The flag is kept out of `metadata.bin` on purpose; see
//     `IndexMetadata::unique`. A version that does not know about unique
//     indexes opens the base, keeps the index as a plain one and can write
//     duplicates. Opening it again here logs them, and
//     `unique_index_duplicates` lists them; new duplicates are still refused.
//   * The applier skips the check while replaying the WAL: every record there
//     already passed it.

use std::collections::{HashMap, HashSet};

use crate::error::{NopalError, Result};
use crate::index::IndexType;
use crate::types::{Node, NodeId, PropertyValue};

use super::Graph;

/// How many conflicting values an error lists before eliding the rest.
const LISTED_CONFLICTS: usize = 5;

/// What the earlier writes of one applier batch (or one bulk load) will have
/// left once applied, for checking the later ones against it.
#[derive(Default)]
pub(crate) struct UniqueBatch {
    /// Last version of each node written so far; `None` = deleted.
    latest: HashMap<NodeId, Option<Node>>,
    /// `(index, value)` → nodes that claimed it. May go stale when a later
    /// write moves the node away; `holds` filters through `latest`.
    claims: HashMap<(String, PropertyValue), HashSet<NodeId>>,
}

impl UniqueBatch {
    /// Check one write-set (`deleted` applied before `written`, the order of
    /// `apply_commit_set`) and, if it passes, account for it so later sets
    /// see it.
    pub(crate) async fn admit(&mut self, graph: &Graph, deleted: &[NodeId], written: &[Node]) -> Result<()> {
        if !graph.index_manager.has_unique() {
            return Ok(());
        }
        let mut own: HashMap<NodeId, Option<&Node>> = HashMap::new();
        for id in deleted {
            own.insert(*id, None);
        }
        let mut own_claims: Vec<(String, PropertyValue, NodeId)> = Vec::new();
        for node in written {
            own.insert(node.id, Some(node));
            for (index, property) in graph.index_manager.unique_properties(&node.label).await {
                if let Some(value) = constrained_value(node, &property) {
                    own_claims.push((index, value.clone(), node.id));
                }
            }
        }

        for (index, value, id) in &own_claims {
            // A node rewritten later in the same set no longer carries this.
            if !matches!(own.get(id), Some(Some(n)) if self.holds(n, index, value, graph).await) {
                continue;
            }
            let mut holders: HashSet<NodeId> = graph.index_manager.lookup(index, value).await?.into_iter().collect();
            if let Some(ids) = self.claims.get(&(index.clone(), value.clone())) {
                holders.extend(ids);
            }
            holders.extend(own_claims.iter().filter(|(i, v, _)| i == index && v == value).map(|(_, _, n)| *n));
            holders.remove(id);
            for other in holders {
                let still_holds = match own.get(&other) {
                    Some(Some(n)) => self.holds(n, index, value, graph).await,
                    Some(None) => false,
                    None => match self.latest.get(&other) {
                        Some(Some(n)) => self.holds(n, index, value, graph).await,
                        Some(None) => false,
                        // Untouched by this batch: the index is the truth.
                        None => true,
                    },
                };
                if still_holds {
                    return Err(violation(index, value, &[other]));
                }
            }
        }

        for (id, node) in own {
            self.latest.insert(id, node.cloned());
        }
        for (index, value, id) in own_claims {
            self.claims.entry((index, value)).or_default().insert(id);
        }
        Ok(())
    }

    /// Account for a deletion that needs no check (it can only free values).
    pub(crate) fn note_deleted(&mut self, id: NodeId) {
        self.latest.insert(id, None);
    }

    /// Whether `node`, as written, holds `value` under the unique `index`.
    async fn holds(&self, node: &Node, index: &str, value: &PropertyValue, graph: &Graph) -> bool {
        graph
            .index_manager
            .unique_properties(&node.label)
            .await
            .iter()
            .any(|(i, property)| i == index && constrained_value(node, property) == Some(value))
    }
}

/// The value `node` holds for a unique `property`; `None` when it has none
/// or it is `null`, which a unique index ignores.
fn constrained_value<'n>(node: &'n Node, property: &str) -> Option<&'n PropertyValue> {
    node.properties.get(property).filter(|v| !matches!(v, PropertyValue::Null))
}

fn violation(index: &str, value: &PropertyValue, holders: &[NodeId]) -> NopalError {
    let holders: Vec<String> = holders.iter().map(|id| id.to_string()).collect();
    NopalError::UniqueViolation(format!(
        "{value:?} is already held by node {} under unique index {index}",
        holders.join(", ")
    ))
}

impl Graph {
    /// Create a unique index: a hash (or btree) index on `label.property`
    /// that refuses any write leaving two nodes of `label` with the same
    /// value. Nodes without the property, or with `null`, are not
    /// constrained. Fails, creating nothing, if the existing nodes already
    /// repeat a value; the error lists the first ones.
    pub async fn create_unique_index(&self, label: &str, property: &str, index_type: IndexType) -> Result<String> {
        self.deny_if_read_only("create_unique_index")?;
        if !matches!(index_type, IndexType::Hash | IndexType::BTree) {
            return Err(NopalError::index_error(format!(
                "a unique index must be hash or btree, not {index_type:?}"
            )));
        }
        // Held from the build to the flag: a write in between could add a
        // duplicate the build did not see.
        let gate = self.write_gate();
        let _gate = gate.lock().await;

        let name = self.create_index(label, property, index_type).await?;
        let mut duplicates = self.index_manager.duplicate_values(&name).await;
        duplicates.retain(|(value, _)| !matches!(value, PropertyValue::Null));
        if !duplicates.is_empty() {
            self.index_manager.drop_index(&name).await?;
            return Err(duplicates_error(&name, &duplicates));
        }

        self.index_manager.set_unique(&name, true).await?;
        if let Err(e) = crate::index::save_unique_names(&self.storage, &self.index_manager.unique_names().await).await {
            self.index_manager.drop_index(&name).await?;
            return Err(e);
        }
        log::info!("Created unique index {name}");
        Ok(name)
    }

    /// Values a unique index holds more than once, with their nodes. Empty
    /// for a healthy index; a version without unique indexes may have
    /// written duplicates (see `unique.rs`). Errors if `index_name` is not a
    /// unique index.
    pub async fn unique_index_duplicates(&self, index_name: &str) -> Result<Vec<(PropertyValue, Vec<NodeId>)>> {
        if !self.index_manager.unique_names().await.iter().any(|n| n == index_name) {
            return Err(NopalError::index_error(format!("{index_name} is not a unique index")));
        }
        let mut duplicates = self.index_manager.duplicate_values(index_name).await;
        duplicates.retain(|(value, _)| !matches!(value, PropertyValue::Null));
        Ok(duplicates)
    }

    /// Log the unique indexes that hold duplicates, once the base is open.
    pub(crate) async fn audit_unique_indexes(&self) {
        for name in self.index_manager.unique_names().await {
            if let Ok(duplicates) = self.unique_index_duplicates(&name).await
                && !duplicates.is_empty()
            {
                log::warn!(
                    "unique index {name} holds {} repeated value(s), written by a version without unique \
                     indexes; new duplicates are refused, the existing ones stay until fixed \
                     (see Graph::unique_index_duplicates)",
                    duplicates.len()
                );
            }
        }
    }

    /// `(index, property)` of the unique indexes of `label`.
    pub(crate) async fn unique_properties(&self, label: &str) -> Vec<(String, String)> {
        self.index_manager.unique_properties(label).await
    }

    /// Drop a unique index's flag from the catalog after the index is gone.
    pub(crate) async fn forget_unique_index(&self) -> Result<()> {
        crate::index::save_unique_names(&self.storage, &self.index_manager.unique_names().await).await
    }
}

fn duplicates_error(index: &str, duplicates: &[(PropertyValue, Vec<NodeId>)]) -> NopalError {
    let listed: Vec<String> = duplicates
        .iter()
        .take(LISTED_CONFLICTS)
        .map(|(value, ids)| {
            let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            format!("{value:?} on nodes {}", ids.join(", "))
        })
        .collect();
    let more = duplicates.len().saturating_sub(LISTED_CONFLICTS);
    let more = if more > 0 { format!(" and {more} more") } else { String::new() };
    NopalError::UniqueViolation(format!(
        "cannot create unique index {index}: {} repeated value(s): {}{more}",
        duplicates.len(),
        listed.join("; ")
    ))
}
