// src/index/taxonomy.rs
//
// TaxonomyIndex — OWL subClassOf hierarchy with incremental BFS transitive closure.
//
// Manages a DAG of NodeKind::Class nodes. Supports:
//   - register_class: register a node with its label
//   - add_subclass / remove_subclass: DAG edge management
//   - descendants: transitive BFS downward (cached, invalidated on mutation)
//   - ancestors: transitive BFS upward (uncached)
//   - is_subclass_of: transitivity check
//   - direct_children / direct_parents: single-hop queries
//   - find_by_label: reverse label lookup
//   - register_class_iri / find_by_iri: classes by absolute IRI (Turtle imports)
//   - register_instance / types_of: declared types of an individual (its
//     `instanceOf` edges), so the NQL predicate sees every type, not the label
//   - resolve_class: one rule for naming a class from NQL (IRI, prefix:Local, label)
//   - is_instance_of: the `instanceOf(n, C)` predicate, direct or inherited

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::error::{NopalError, Result};
use crate::index::{Index, IndexQuery};
use crate::types::{NodeId, PropertyValue};

/// Index over OWL-style class hierarchies (subClassOf DAG).
///
/// Closure cache is lazily populated on first `descendants()` call and invalidated
/// whenever the DAG is mutated (`add_subclass` / `remove_subclass`).
#[derive(Debug, Default, Clone)]
pub struct TaxonomyIndex {
    /// parent_id → direct children
    children: HashMap<NodeId, Vec<NodeId>>,
    /// child_id → direct parents (inverse, kept in sync)
    parents: HashMap<NodeId, Vec<NodeId>>,
    /// node_id → label
    labels: HashMap<NodeId, String>,
    /// label → node_id (reverse lookup)
    label_to_id: HashMap<String, NodeId>,
    /// Transitive closure cache: root_id → Some(set) or None (invalidated)
    closure_cache: HashMap<NodeId, Option<HashSet<NodeId>>>,
    /// class IRI → node_id, for classes that have one (Turtle imports).
    iris: HashMap<String, NodeId>,
    /// node_id → class IRI (inverse of `iris`, kept in sync).
    iri_of: HashMap<NodeId, String>,
    /// individual → its declared types (the targets of its `instanceOf` edges).
    ///
    /// Behind an `Arc` because the NQL executor clones the whole index once per
    /// evaluated node (`get_taxonomy_sync` hands out snapshots): the class maps
    /// are small, this one grows with the data, and a refcount bump keeps the
    /// per-node cost where it was. Writers go through `Arc::make_mut`.
    instance_types: Arc<HashMap<NodeId, Vec<NodeId>>>,
    /// prefix → namespace IRI, the graph's RDF prefix catalog, so `resolve_class`
    /// can expand `flora:Rosa` without touching storage.
    prefixes: Arc<BTreeMap<String, String>>,
}

impl TaxonomyIndex {
    /// Create an empty TaxonomyIndex.
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------------
    // Class registration
    // -----------------------------------------------------------------------

    /// Register a Class node with its label. Idempotent — calling with the same
    /// `id` twice overwrites the label entry; the DAG edges are unchanged.
    pub fn register_class(&mut self, id: NodeId, label: impl Into<String>) {
        let label = label.into();
        // Remove stale reverse entry if the label changed.
        if let Some(old_label) = self.labels.get(&id)
            && old_label != &label
        {
            self.label_to_id.remove(old_label);
        }
        self.labels.insert(id, label.clone());
        self.label_to_id.insert(label, id);
        // Ensure adjacency entries exist even for leaf nodes.
        self.children.entry(id).or_default();
        self.parents.entry(id).or_default();
    }

    /// Record the absolute IRI of a registered class, so NQL can name it by IRI
    /// or by `prefix:Local`. A class without IRI (built by hand or by NQL) is
    /// reachable by label only.
    pub fn register_class_iri(&mut self, id: NodeId, iri: impl Into<String>) {
        let iri = iri.into();
        if let Some(old) = self.iri_of.insert(id, iri.clone()) {
            self.iris.remove(&old);
        }
        self.iris.insert(iri, id);
    }

    /// Lookup a class by its absolute IRI.
    pub fn find_by_iri(&self, iri: &str) -> Option<NodeId> {
        self.iris.get(iri).copied()
    }

    /// The IRI recorded for a class, if any.
    pub fn iri_of(&self, id: NodeId) -> Option<&str> {
        self.iri_of.get(&id).map(String::as_str)
    }

    /// Record that `individual` is a declared instance of `class` (one call per
    /// `instanceOf` edge). Idempotent.
    ///
    /// The types live here, in the snapshot, and not in the graph's edges at
    /// evaluation time on purpose: the executor evaluates predicates
    /// synchronously over a cloned index while storage is async, so an edge
    /// lookup per node would need a second, blocking path into storage. The
    /// importer and `rebuild_taxonomy_from_graph` already walk every
    /// `instanceOf` edge; recording it here costs one map entry.
    pub fn register_instance(&mut self, individual: NodeId, class: NodeId) {
        let types = Arc::make_mut(&mut self.instance_types).entry(individual).or_default();
        if !types.contains(&class) {
            types.push(class);
        }
    }

    /// Declared types of `individual`; empty when nothing was registered (a node
    /// created by hand or by NQL, or a database written before 0.5.10).
    pub fn types_of(&self, individual: NodeId) -> &[NodeId] {
        self.instance_types.get(&individual).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Number of individuals with at least one registered type.
    pub fn instance_count(&self) -> usize {
        self.instance_types.len()
    }

    /// Replace the prefix catalog used by [`Self::resolve_class`].
    pub fn set_prefixes(&mut self, prefixes: BTreeMap<String, String>) {
        self.prefixes = Arc::new(prefixes);
    }

    /// The prefix catalog known to this index.
    pub fn prefixes(&self) -> &BTreeMap<String, String> {
        &self.prefixes
    }

    /// Resolve the class named by an NQL argument. One rule for every
    /// ontology predicate, in this order:
    ///
    /// 1. an absolute IRI (`http://…`, or a blank node `_:…`) → [`Self::find_by_iri`];
    /// 2. `prefix:Local` → expanded with the prefix catalog and looked up by IRI;
    ///    if the prefix is unknown, the text is tried as a label, because that
    ///    is exactly how the importer labels a class whose local name collided
    ///    (`fauna:Rosa`);
    /// 3. anything else → by label, as before 0.5.11.
    pub fn resolve_class(&self, name: &str) -> Option<NodeId> {
        if name.contains("://") || name.starts_with("_:") {
            return self.find_by_iri(name);
        }
        if let Some((prefix, local)) = name.split_once(':')
            && let Some(ns) = self.prefixes.get(prefix)
            && let Some(id) = self.find_by_iri(&format!("{ns}{local}"))
        {
            return Some(id);
        }
        self.find_by_label(name)
    }

    /// The NQL `instanceOf(n, C)` test for an individual: some declared type
    /// `T` of the node is `C` itself or a transitive subclass of `C`.
    ///
    /// When the node has no registered types, its `label` is read as its (only)
    /// class, which is how every graph built by hand, by NQL `add`, or by an
    /// import older than 0.5.10 encodes the type. Before 0.5.11 this path
    /// answered `false` for direct instances (`is_subclass_of` is strict), so
    /// `instanceOf(n, "Planta")` missed every `:musgo a :Planta`.
    pub fn is_instance_of(&mut self, individual: NodeId, label: &str, class: NodeId) -> bool {
        let types = self.types_of(individual).to_vec();
        if types.is_empty() {
            return match self.find_by_label(label) {
                Some(t) => t == class || self.is_subclass_of(t, class),
                None => false,
            };
        }
        types.into_iter().any(|t| t == class || self.is_subclass_of(t, class))
    }

    /// The NQL `subClassOf(c, C)` test for a class node: a strict subclass
    /// (`c` is never a subclass of itself). Prefers the node's id and falls
    /// back to its label for class nodes the index never registered by id.
    pub fn is_class_subclass_of(&mut self, class_node: NodeId, label: &str, ancestor: NodeId) -> bool {
        if self.labels.contains_key(&class_node) {
            self.is_subclass_of(class_node, ancestor)
        } else {
            self.is_subclass_of_label(label, ancestor)
        }
    }

    // -----------------------------------------------------------------------
    // DAG mutation
    // -----------------------------------------------------------------------

    /// Add a direct subClassOf edge: `child` is a direct subclass of `parent`.
    ///
    /// Returns `Err` if `parent == child` (self-loop). Idempotent — adding the
    /// same edge twice is a no-op.
    pub fn add_subclass(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        if parent == child {
            return Err(NopalError::custom(
                "add_subclass: self-loops are not allowed in a class hierarchy",
            ));
        }

        // Idempotency: skip if edge already exists.
        let children_of_parent = self.children.entry(parent).or_default();
        if !children_of_parent.contains(&child) {
            children_of_parent.push(child);
        }
        let parents_of_child = self.parents.entry(child).or_default();
        if !parents_of_child.contains(&parent) {
            parents_of_child.push(parent);
        }
        // Ensure both ends have empty vecs in the opposite map.
        self.children.entry(child).or_default();
        self.parents.entry(parent).or_default();

        // Invalidate the closure cache for `parent` and all its ancestors.
        self.invalidate_cache_for(parent);

        Ok(())
    }

    /// Remove a direct subClassOf edge.
    ///
    /// Returns `Ok(())` even if the edge didn't exist (idempotent).
    pub fn remove_subclass(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        if let Some(children) = self.children.get_mut(&parent) {
            children.retain(|&c| c != child);
        }
        if let Some(parents) = self.parents.get_mut(&child) {
            parents.retain(|&p| p != parent);
        }

        self.invalidate_cache_for(parent);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Closure cache management
    // -----------------------------------------------------------------------

    /// Invalidate the closure cache for `node` and all its transitive ancestors.
    ///
    /// Uses BFS upward through `parents` to collect the full ancestor set
    /// (including `node` itself) and marks each entry as `None`.
    fn invalidate_cache_for(&mut self, node: NodeId) {
        let mut to_invalidate = vec![node];
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(node);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            to_invalidate.push(current);
            if let Some(pars) = self.parents.get(&current) {
                for &p in pars {
                    if !visited.contains(&p) {
                        queue.push_back(p);
                    }
                }
            }
        }

        for id in to_invalidate {
            self.closure_cache.insert(id, None);
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// All transitive descendants of `root` (BFS downward), excluding `root` itself.
    ///
    /// Result is cached. The cache is invalidated automatically on any DAG mutation
    /// that could affect `root`'s descendants.
    pub fn descendants(&mut self, root: NodeId) -> Vec<NodeId> {
        // Check cache first.
        if let Some(Some(cached)) = self.closure_cache.get(&root) {
            return cached.iter().copied().collect();
        }

        // BFS downward.
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut queue: VecDeque<NodeId> = VecDeque::new();

        // Seed with direct children.
        if let Some(direct) = self.children.get(&root) {
            for &c in direct {
                if c != root {
                    queue.push_back(c);
                }
            }
        }

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(kids) = self.children.get(&current) {
                for &kid in kids {
                    if !visited.contains(&kid) {
                        queue.push_back(kid);
                    }
                }
            }
        }

        // Store in cache.
        self.closure_cache.insert(root, Some(visited.clone()));
        visited.into_iter().collect()
    }

    /// All transitive ancestors of `node` (BFS upward), excluding `node` itself.
    ///
    /// Not cached (ancestor traversal is less frequent and cheaper to reason about
    /// without invalidation complexity).
    pub fn ancestors(&self, node: NodeId) -> Vec<NodeId> {
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut queue: VecDeque<NodeId> = VecDeque::new();

        if let Some(direct) = self.parents.get(&node) {
            for &p in direct {
                if p != node {
                    queue.push_back(p);
                }
            }
        }

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(pars) = self.parents.get(&current) {
                for &p in pars {
                    if !visited.contains(&p) {
                        queue.push_back(p);
                    }
                }
            }
        }

        visited.into_iter().collect()
    }

    /// Returns `true` if `child` is a direct or transitive subclass of `ancestor`.
    pub fn is_subclass_of(&mut self, child: NodeId, ancestor: NodeId) -> bool {
        if child == ancestor {
            return false;
        }
        self.descendants(ancestor).contains(&child)
    }

    /// Lookup a NodeId by its registered label. Case-sensitive.
    pub fn find_by_label(&self, label: &str) -> Option<NodeId> {
        self.label_to_id.get(label).copied()
    }

    /// Returns `true` if the class registered under `child_label` is a
    /// direct or transitive subclass of `ancestor_id`.
    ///
    /// Used by the NQL `instanceOf` / `subClassOf` predicate evaluators which
    /// have only a label available for the node under test.
    pub fn is_subclass_of_label(&mut self, child_label: &str, ancestor_id: NodeId) -> bool {
        match self.find_by_label(child_label) {
            Some(child_id) => self.is_subclass_of(child_id, ancestor_id),
            None => false,
        }
    }

    /// Direct (non-transitive) children of `parent`.
    pub fn direct_children(&self, parent: NodeId) -> Vec<NodeId> {
        self.children
            .get(&parent)
            .cloned()
            .unwrap_or_default()
    }

    /// Direct (non-transitive) parents of `child`.
    pub fn direct_parents(&self, child: NodeId) -> Vec<NodeId> {
        self.parents
            .get(&child)
            .cloned()
            .unwrap_or_default()
    }

    /// Number of registered Class nodes.
    pub fn size(&self) -> usize {
        self.labels.len()
    }

    /// All registered Class node IDs.
    pub fn all_class_ids(&self) -> Vec<NodeId> {
        self.labels.keys().copied().collect()
    }

    /// Total number of direct subClassOf edges in the DAG.
    pub fn edge_count(&self) -> usize {
        self.children.values().map(|v| v.len()).sum()
    }

    /// Reset the index to an empty state.
    pub fn clear(&mut self) {
        self.children.clear();
        self.parents.clear();
        self.labels.clear();
        self.label_to_id.clear();
        self.closure_cache.clear();
        self.iris.clear();
        self.iri_of.clear();
        self.instance_types = Arc::default();
        self.prefixes = Arc::default();
    }

    /// Remove a Class node from the index, cleaning up all DAG entries and
    /// invalidating affected cache entries.
    fn unregister_class(&mut self, id: NodeId) {
        // Remove label mappings.
        if let Some(label) = self.labels.remove(&id) {
            self.label_to_id.remove(&label);
        }
        if let Some(iri) = self.iri_of.remove(&id) {
            self.iris.remove(&iri);
        }

        // Remove `id` from children lists of its parents.
        if let Some(parent_ids) = self.parents.remove(&id) {
            for parent in &parent_ids {
                if let Some(kids) = self.children.get_mut(parent) {
                    kids.retain(|&k| k != id);
                }
            }
        }

        // Remove `id` from parents lists of its children.
        if let Some(child_ids) = self.children.remove(&id) {
            for child in &child_ids {
                if let Some(pars) = self.parents.get_mut(child) {
                    pars.retain(|&p| p != id);
                }
            }
        }

        // Invalidate cache for all ancestors (including `id` itself).
        self.invalidate_cache_for(id);
        // Clean up cache entry for the removed node itself.
        self.closure_cache.remove(&id);
    }
}

// ---------------------------------------------------------------------------
// Index trait implementation
// ---------------------------------------------------------------------------

impl Index for TaxonomyIndex {
    /// `value` is expected to be `PropertyValue::String(label)`.
    /// Registers the node as a Class with the given label.
    fn insert(&mut self, value: PropertyValue, node_id: NodeId) -> Result<()> {
        if let PropertyValue::String(label) = value {
            self.register_class(node_id, label);
        }
        Ok(())
    }

    /// Removes the node from the taxonomy index.
    fn remove(&mut self, _value: &PropertyValue, node_id: NodeId) -> Result<()> {
        self.unregister_class(node_id);
        Ok(())
    }

    /// Supports `IndexQuery::Equals(PropertyValue::String(label))` only.
    /// Returns the single matching NodeId (or empty vec if not found).
    fn query(&self, query: &IndexQuery) -> Result<Vec<NodeId>> {
        match query {
            IndexQuery::Equals(PropertyValue::String(label)) => {
                Ok(self.find_by_label(label).map(|id| vec![id]).unwrap_or_default())
            }
            _ => Err(NopalError::custom(
                "TaxonomyIndex only supports IndexQuery::Equals(String(label))",
            )),
        }
    }

    fn clear(&mut self) -> Result<()> {
        TaxonomyIndex::clear(self);
        Ok(())
    }

    fn size(&self) -> usize {
        TaxonomyIndex::size(self)
    }

    /// Called to add a subClassOf edge (parent → child).
    /// Delegates to `add_subclass`.
    fn add_relationship(&mut self, source: NodeId, target: NodeId) -> Result<()> {
        self.add_subclass(source, target)
    }

    fn as_taxonomy(&self) -> Option<&TaxonomyIndex> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id() -> NodeId {
        Uuid::new_v4()
    }

    // -- #93: IRIs, declared types, prefix resolution, direct instances --------

    fn flora() -> (TaxonomyIndex, NodeId, NodeId, NodeId, NodeId) {
        let mut idx = TaxonomyIndex::new();
        let (planta, arbol, medicinal, fauna_rosa) = (id(), id(), id(), id());
        idx.register_class(planta, "Planta");
        idx.register_class_iri(planta, "http://plantas.example/flora#Planta");
        idx.register_class(arbol, "Arbol");
        idx.register_class_iri(arbol, "http://plantas.example/flora#Arbol");
        idx.register_class(medicinal, "PlantaMedicinal");
        idx.register_class_iri(medicinal, "http://plantas.example/flora#PlantaMedicinal");
        idx.register_class(fauna_rosa, "fauna:Rosa");
        idx.register_class_iri(fauna_rosa, "http://plantas.example/fauna#Rosa");
        idx.add_subclass(planta, arbol).unwrap();
        idx.add_subclass(planta, medicinal).unwrap();
        idx.set_prefixes(BTreeMap::from([
            ("flora".to_string(), "http://plantas.example/flora#".to_string()),
            ("".to_string(), "http://plantas.example/".to_string()),
        ]));
        (idx, planta, arbol, medicinal, fauna_rosa)
    }

    #[test]
    fn test_resolve_class_by_iri_prefix_and_label() {
        let (idx, planta, arbol, _, fauna_rosa) = flora();
        assert_eq!(idx.resolve_class("Arbol"), Some(arbol));
        assert_eq!(idx.resolve_class("flora:Arbol"), Some(arbol));
        assert_eq!(idx.resolve_class("http://plantas.example/flora#Planta"), Some(planta));
        // Unknown prefix: the text is a label (how the importer names collisions).
        assert_eq!(idx.resolve_class("fauna:Rosa"), Some(fauna_rosa));
        assert_eq!(idx.resolve_class("flora:Hongo"), None);
        assert_eq!(idx.resolve_class("http://plantas.example/flora#Hongo"), None);
        assert_eq!(idx.resolve_class("Hongo"), None);
        // Empty prefix expands too.
        assert_eq!(idx.resolve_class(":Arbol"), None); // no class under the default namespace
    }

    #[test]
    fn test_register_class_iri_replaces_previous_iri() {
        let mut idx = TaxonomyIndex::new();
        let c = id();
        idx.register_class(c, "Planta");
        idx.register_class_iri(c, "http://a.example/Planta");
        idx.register_class_iri(c, "http://b.example/Planta");
        assert_eq!(idx.find_by_iri("http://a.example/Planta"), None);
        assert_eq!(idx.find_by_iri("http://b.example/Planta"), Some(c));
        assert_eq!(idx.iri_of(c), Some("http://b.example/Planta"));
    }

    #[test]
    fn test_is_instance_of_direct_inherited_and_multityped() {
        let (mut idx, planta, arbol, medicinal, fauna_rosa) = flora();
        let (roble, musgo, sauce) = (id(), id(), id());
        idx.register_instance(roble, arbol);
        idx.register_instance(musgo, planta);
        idx.register_instance(sauce, arbol);
        idx.register_instance(sauce, medicinal);
        idx.register_instance(sauce, medicinal); // idempotent
        assert_eq!(idx.types_of(sauce).len(), 2);
        assert_eq!(idx.instance_count(), 3);

        // direct
        assert!(idx.is_instance_of(roble, "Arbol", arbol));
        assert!(idx.is_instance_of(musgo, "Planta", planta));
        // inherited
        assert!(idx.is_instance_of(roble, "Arbol", planta));
        // second type counts even though the label says Arbol
        assert!(idx.is_instance_of(sauce, "Arbol", medicinal));
        // unrelated
        assert!(!idx.is_instance_of(roble, "Arbol", medicinal));
        assert!(!idx.is_instance_of(roble, "Arbol", fauna_rosa));
    }

    #[test]
    fn test_is_instance_of_falls_back_to_label_without_registered_types() {
        let (mut idx, planta, arbol, medicinal, _) = flora();
        let encino = id(); // never registered: a node added by hand
        assert!(idx.is_instance_of(encino, "Arbol", arbol), "direct instance by label");
        assert!(idx.is_instance_of(encino, "Arbol", planta), "inherited by label");
        assert!(!idx.is_instance_of(encino, "Arbol", medicinal));
        assert!(!idx.is_instance_of(encino, "Piedra", planta), "unknown label");
    }

    #[test]
    fn test_is_class_subclass_of_is_strict() {
        let (mut idx, planta, arbol, _, _) = flora();
        assert!(idx.is_class_subclass_of(arbol, "Arbol", planta));
        assert!(!idx.is_class_subclass_of(planta, "Planta", planta));
        // Unregistered class node id: label decides.
        assert!(idx.is_class_subclass_of(id(), "Arbol", planta));
    }

    #[test]
    fn test_clear_and_unregister_drop_iris_and_types() {
        let (mut idx, planta, arbol, _, _) = flora();
        let roble = id();
        idx.register_instance(roble, arbol);
        idx.unregister_class(arbol);
        assert_eq!(idx.find_by_iri("http://plantas.example/flora#Arbol"), None);
        assert_eq!(idx.find_by_iri("http://plantas.example/flora#Planta"), Some(planta));
        idx.clear();
        assert_eq!(idx.instance_count(), 0);
        assert!(idx.prefixes().is_empty());
        assert_eq!(idx.resolve_class("Planta"), None);
    }

    #[test]
    fn test_register_class() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        idx.register_class(animal, "Animal");

        assert_eq!(idx.size(), 1);
        assert_eq!(idx.find_by_label("Animal"), Some(animal));
        assert_eq!(idx.find_by_label("Unknown"), None);
    }

    #[test]
    fn test_add_subclass_simple() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");

        idx.add_subclass(animal, mammal).unwrap();

        let desc = idx.descendants(animal);
        assert_eq!(desc.len(), 1);
        assert!(desc.contains(&mammal));
    }

    #[test]
    fn test_transitive_closure() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        let desc = idx.descendants(animal);
        assert_eq!(desc.len(), 2);
        assert!(desc.contains(&mammal));
        assert!(desc.contains(&dog));
    }

    #[test]
    fn test_is_subclass_of() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        assert!(idx.is_subclass_of(dog, animal));
        assert!(idx.is_subclass_of(dog, mammal));
        assert!(idx.is_subclass_of(mammal, animal));
        assert!(!idx.is_subclass_of(animal, dog));
        assert!(!idx.is_subclass_of(dog, dog));
    }

    #[test]
    fn test_remove_subclass() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        // Remove Mammal → Dog
        idx.remove_subclass(mammal, dog).unwrap();

        let desc_animal = idx.descendants(animal);
        assert!(desc_animal.contains(&mammal));
        assert!(!desc_animal.contains(&dog));

        let desc_mammal = idx.descendants(mammal);
        assert!(desc_mammal.is_empty());
    }

    #[test]
    fn test_ancestors() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        let anc = idx.ancestors(dog);
        assert_eq!(anc.len(), 2);
        assert!(anc.contains(&mammal));
        assert!(anc.contains(&animal));

        // Animal has no ancestors
        let anc_animal = idx.ancestors(animal);
        assert!(anc_animal.is_empty());
    }

    #[test]
    fn test_direct_children() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        let dc = idx.direct_children(animal);
        assert_eq!(dc.len(), 1);
        assert!(dc.contains(&mammal));
        assert!(!dc.contains(&dog)); // only direct, not transitive
    }

    #[test]
    fn test_diamond_hierarchy() {
        // A → B, A → C, B → D, C → D
        let mut idx = TaxonomyIndex::new();
        let a = id();
        let b = id();
        let c = id();
        let d = id();
        idx.register_class(a, "A");
        idx.register_class(b, "B");
        idx.register_class(c, "C");
        idx.register_class(d, "D");

        idx.add_subclass(a, b).unwrap();
        idx.add_subclass(a, c).unwrap();
        idx.add_subclass(b, d).unwrap();
        idx.add_subclass(c, d).unwrap();

        let desc_a = idx.descendants(a);
        assert_eq!(desc_a.len(), 3, "A should have B, C, D as descendants");
        assert!(desc_a.contains(&b));
        assert!(desc_a.contains(&c));
        assert!(desc_a.contains(&d));

        // D's ancestors: B, C, A
        let anc_d = idx.ancestors(d);
        assert_eq!(anc_d.len(), 3);
        assert!(anc_d.contains(&a));
        assert!(anc_d.contains(&b));
        assert!(anc_d.contains(&c));

        // D is subclass of A (transitively)
        assert!(idx.is_subclass_of(d, a));
    }

    #[test]
    fn test_cache_invalidation() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();
        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");

        idx.add_subclass(animal, mammal).unwrap();

        // First call populates cache
        let desc = idx.descendants(animal);
        assert_eq!(desc.len(), 1);
        assert!(desc.contains(&mammal));

        // Add a new child — should invalidate cache for animal
        idx.add_subclass(mammal, dog).unwrap();

        // Cache must have been invalidated; re-query
        let desc2 = idx.descendants(animal);
        assert_eq!(desc2.len(), 2, "cache should be invalidated after add_subclass");
        assert!(desc2.contains(&dog));

        // Remove the edge and verify re-invalidation
        idx.remove_subclass(mammal, dog).unwrap();
        let desc3 = idx.descendants(animal);
        assert_eq!(desc3.len(), 1, "cache should be re-invalidated after remove_subclass");
        assert!(!desc3.contains(&dog));
    }

    #[test]
    fn test_self_loop_rejected() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        idx.register_class(animal, "Animal");

        let result = idx.add_subclass(animal, animal);
        assert!(result.is_err());
    }

    #[test]
    fn test_idempotent_add_subclass() {
        let mut idx = TaxonomyIndex::new();
        let a = id();
        let b = id();
        idx.register_class(a, "A");
        idx.register_class(b, "B");

        idx.add_subclass(a, b).unwrap();
        idx.add_subclass(a, b).unwrap(); // duplicate, should not panic or duplicate entry

        assert_eq!(idx.direct_children(a).len(), 1);
        assert_eq!(idx.direct_parents(b).len(), 1);
    }

    #[test]
    fn test_clear() {
        let mut idx = TaxonomyIndex::new();
        let a = id();
        let b = id();
        idx.register_class(a, "A");
        idx.register_class(b, "B");
        idx.add_subclass(a, b).unwrap();

        idx.clear();

        assert_eq!(idx.size(), 0);
        assert!(idx.find_by_label("A").is_none());
        assert!(idx.direct_children(a).is_empty());
        assert!(idx.descendants(a).is_empty());
    }

    // -----------------------------------------------------------------------
    // Index trait tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_index_trait_insert() {
        use crate::index::{Index, IndexQuery};

        let mut idx = TaxonomyIndex::new();
        let animal = id();

        // Insert via trait
        idx.insert(PropertyValue::String("Animal".to_string()), animal).unwrap();

        assert_eq!(idx.size(), 1);

        // Query via trait
        let result = idx.query(&IndexQuery::Equals(PropertyValue::String("Animal".to_string()))).unwrap();
        assert_eq!(result, vec![animal]);

        // Missing label returns empty vec
        let result2 = idx.query(&IndexQuery::Equals(PropertyValue::String("Unknown".to_string()))).unwrap();
        assert!(result2.is_empty());
    }

    #[test]
    fn test_index_trait_remove() {
        use crate::index::Index;

        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();

        idx.insert(PropertyValue::String("Animal".to_string()), animal).unwrap();
        idx.insert(PropertyValue::String("Mammal".to_string()), mammal).unwrap();
        idx.add_subclass(animal, mammal).unwrap();

        assert_eq!(idx.size(), 2);

        // Remove via trait
        idx.remove(&PropertyValue::String("Mammal".to_string()), mammal).unwrap();

        assert_eq!(idx.size(), 1);
        assert!(idx.direct_children(animal).is_empty());
        assert!(idx.find_by_label("Mammal").is_none());
    }

    #[test]
    fn test_index_trait_add_relationship() {
        use crate::index::Index;

        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();

        idx.insert(PropertyValue::String("Animal".to_string()), animal).unwrap();
        idx.insert(PropertyValue::String("Mammal".to_string()), mammal).unwrap();
        idx.insert(PropertyValue::String("Dog".to_string()), dog).unwrap();

        // Wire hierarchy via add_relationship (parent → child)
        idx.add_relationship(animal, mammal).unwrap();
        idx.add_relationship(mammal, dog).unwrap();

        // Transitive closure: Animal → Mammal → Dog
        assert!(idx.is_subclass_of(dog, animal));
        assert!(idx.is_subclass_of(mammal, animal));
        assert!(!idx.is_subclass_of(animal, dog));
    }

    #[test]
    fn test_unregister_class() {
        let mut idx = TaxonomyIndex::new();
        let animal = id();
        let mammal = id();
        let dog = id();

        idx.register_class(animal, "Animal");
        idx.register_class(mammal, "Mammal");
        idx.register_class(dog, "Dog");
        idx.add_subclass(animal, mammal).unwrap();
        idx.add_subclass(mammal, dog).unwrap();

        // Unregister Mammal — should clean up all adjacency maps.
        idx.unregister_class(mammal);

        assert_eq!(idx.size(), 2);
        assert!(idx.find_by_label("Mammal").is_none());

        // Animal should no longer have Mammal as a child.
        assert!(idx.direct_children(animal).is_empty());
        // Dog should have no parents.
        assert!(idx.direct_parents(dog).is_empty());
        // Cache for Animal should have been invalidated.
        let desc = idx.descendants(animal);
        assert!(desc.is_empty());
    }

    #[test]
    fn test_index_trait_query_non_string_errors() {
        use crate::index::{Index, IndexQuery};

        let idx = TaxonomyIndex::new();

        // Non-string query should return an error.
        let result = idx.query(&IndexQuery::Equals(PropertyValue::Int(42)));
        assert!(result.is_err());

        // GreaterThan queries are unsupported.
        let result2 = idx.query(&IndexQuery::GreaterThan(PropertyValue::Int(1)));
        assert!(result2.is_err());
    }
}
