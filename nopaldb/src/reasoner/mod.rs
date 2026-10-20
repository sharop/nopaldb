//! OWL-EL Reasoner — Steps 5, 9 and CR3 of the NopalDB ontological roadmap,
//! plus disjoint classes and ⊥ (`owl:Nothing`, #235).
//!
//! Implements:
//! - CR1 (transitivity): A ⊑ B ∧ B ⊑ C → A ⊑ C
//! - CR2 (conjunction):  A ⊑ B ∧ A ⊑ C ∧ B ⊓ C ⊑ D → A ⊑ D
//! - CR3 (existential):  A ⊑ ∃R.B ∧ B ⊑ C ∧ ∃R.C ⊑ D → A ⊑ D
//! - CR4 (⊥ through an existential): A ⊑ ∃R.B ∧ B ⊑ ⊥ → A ⊑ ⊥
//!
//! ⊥ is the built-in class [`OWL_NOTHING`]. Disjoint classes
//! ([`ELReasoner::assert_disjoint`]) are normalised to `A ⊓ B ⊑ ⊥` per pair,
//! so they stay inside EL++ and in polynomial time; a class below ⊥ is
//! **unsatisfiable** ([`ELReasoner::unsatisfiable_classes`]) and
//! [`ELReasoner::why_unsatisfiable`] says which disjoint pair (or existential)
//! caused it.
//!
//! The reasoner is a standalone value type that operates over a [`TaxonomyIndex`].
//! It does NOT write inferences back to the graph automatically — callers decide
//! whether to materialize them.
//!
//! # Feature gate
//! Compiled only when the `reasoner` feature is enabled.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::{NopalError, Result};
use crate::index::taxonomy::TaxonomyIndex;
use crate::types::NodeId;

/// ⊥ — `owl:Nothing`, the empty class. Built in: the reasoner knows it without
/// [`ELReasoner::register_class`]. A class below it is unsatisfiable.
///
/// A fixed id so it is the same in every process and every graph: the UUID v5
/// of `http://www.w3.org/2002/07/owl#Nothing` in the URL namespace.
pub const OWL_NOTHING: NodeId = uuid::Uuid::from_u128(0xdb39_00d2_6051_5ea5_a5b9_1126_1cdb_f979);

/// Edge type of a disjointness between two class nodes in the graph
/// (`owl:disjointWith`, and each pair of an `owl:AllDisjointClasses`).
/// [`ELReasoner::from_graph_at`] reads it back.
pub const EDGE_DISJOINT_WITH: &str = "disjointWith";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// An EL axiom asserted into the reasoner.
///
/// Naming follows OWL 2 EL profile (Baader, Brandt, Lutz 2005).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Axiom {
    /// `sub ⊑ super_class` — sub is a subclass of super_class.
    SubClassOf {
        /// The subclass (more specific).
        sub: NodeId,
        /// The superclass (more general).
        super_class: NodeId,
    },
    /// `left ⊓ right ⊑ result` — conjunction inclusion (CR2).
    ///
    /// Means: if a class X is a subclass of both `left` and `right`,
    /// then X is also a subclass of `result`.
    ConjunctionInclusion {
        /// Left conjunct.
        left: NodeId,
        /// Right conjunct.
        right: NodeId,
        /// Conclusion superclass.
        result: NodeId,
    },
    /// `sub ⊑ ∃role.filler` — existential restriction on the left side of CR3.
    ExistentialRestriction {
        /// The subclass bearing the restriction.
        sub: NodeId,
        /// The property/role name.
        role: String,
        /// The filler class (B in ∃R.B).
        filler: NodeId,
    },
    /// `∃role.filler ⊑ result` — existential domain on the right side of CR3.
    ExistentialDomain {
        /// The property/role name.
        role: String,
        /// The filler class (C in ∃R.C).
        filler: NodeId,
        /// The conclusion superclass.
        result: NodeId,
    },
    /// `DisjointClasses(c1 … cn)` — no two of the classes share an instance.
    /// Normalised to `ci ⊓ cj ⊑ ⊥` for every pair (n·(n−1)/2 conjunctions).
    DisjointClasses {
        /// Two or more distinct classes.
        classes: Vec<NodeId>,
    },
}

/// An inference derived by the reasoner via a completion rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inference {
    /// The axiom that was derived.
    pub axiom: Axiom,
    /// Which completion rule fired to produce this inference.
    pub rule: CompletionRule,
}

/// Which OWL-EL completion rule produced an inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionRule {
    /// CR1: transitivity — A ⊑ B ∧ B ⊑ C → A ⊑ C
    CR1,
    /// CR2: conjunction — A ⊑ B ∧ A ⊑ C ∧ B ⊓ C ⊑ D → A ⊑ D
    CR2,
    /// CR3: existential — A ⊑ ∃R.B ∧ B ⊑ C ∧ ∃R.C ⊑ D → A ⊑ D
    CR3,
    /// CR4: ⊥ through an existential — A ⊑ ∃R.B ∧ B ⊑ ⊥ → A ⊑ ⊥
    CR4,
}

/// Why a class is unsatisfiable (below ⊥): the first cause the
/// classification found. Answers "rejected because its parents X and Y are
/// disjoint" without walking the derivation.
///
/// It is not the full proof: a class with several causes reports one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsatisfiable {
    /// CR2: the class is below both `left` and `right`, and `left ⊓ right ⊑ ⊥`
    /// (a disjoint pair, or a conjunction asserted into ⊥).
    Disjoint {
        /// One class of the pair.
        left: NodeId,
        /// The other class of the pair.
        right: NodeId,
    },
    /// The class was asserted directly as a subclass of ⊥.
    Asserted,
    /// CR1: the class is below `parent`, which was asserted below ⊥.
    Inherited {
        /// The class asserted below ⊥.
        parent: NodeId,
    },
    /// The class is below `∃role.filler` and either `filler` is
    /// unsatisfiable (CR4) or `∃role.C ⊑ ⊥` was asserted for a `C` above the
    /// filler (CR3); `rule` says which.
    Existential {
        /// The role of the restriction.
        role: String,
        /// The filler class.
        filler: NodeId,
        /// `CR4` or `CR3`.
        rule: CompletionRule,
    },
}

// ---------------------------------------------------------------------------
// ELReasoner
// ---------------------------------------------------------------------------

/// An OWL-EL reasoner that operates over a [`TaxonomyIndex`].
///
/// [`classify_all`](Self::classify_all) computes the full closure of CR1–CR4
/// (EL completion over subsumer sets with a work queue: every rule sees what
/// the others derived, so the result is a fixpoint, not one pass). The
/// `assert_*` methods apply their own rule incrementally, one step, and
/// return what that step derived; `classify_all` completes the rest.
///
/// The reasoner owns a cloned working copy of the `TaxonomyIndex` so that
/// it can mutate it incrementally without affecting the caller's original index.
///
/// All methods are synchronous — no Tokio runtime is needed.
pub struct ELReasoner {
    taxonomy: TaxonomyIndex,
    /// Pairs `(sub, super_class)` derived by CR1–CR4 (not directly asserted),
    /// with the rule that derived them. One byte per pair: a 1 000-class
    /// chain derives ~500 000 pairs, so the explanation of ⊥ lives apart in
    /// `unsat`, not here.
    derived: HashMap<(NodeId, NodeId), CompletionRule>,
    /// CR2 conjunction axioms: `(left, right, result)` meaning `left ⊓ right ⊑ result`.
    conjunctions: Vec<(NodeId, NodeId, NodeId)>,
    /// `conjunctions` deduplicated, with the conjuncts in a fixed order.
    conjunction_keys: HashSet<(NodeId, NodeId, NodeId)>,
    /// CR3 left-side axioms: `(sub=A, role=R, filler=B)` for `A ⊑ ∃R.B`.
    existentials: Vec<(NodeId, String, NodeId)>,
    /// CR3 right-side axioms: `(role=R, filler=C, result=D)` for `∃R.C ⊑ D`.
    existential_domains: Vec<(String, NodeId, NodeId)>,
    /// Unsatisfiable classes and their cause, as of the last `classify_all`.
    unsat: HashMap<NodeId, Unsatisfiable>,
    /// `true` while nothing changed since the last `classify_all`.
    saturated: bool,
}

impl Default for ELReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl ELReasoner {
    /// Create a new, empty reasoner.
    pub fn new() -> Self {
        Self::with_taxonomy(TaxonomyIndex::new())
    }

    fn with_taxonomy(taxonomy: TaxonomyIndex) -> Self {
        Self {
            taxonomy,
            derived: HashMap::new(),
            conjunctions: Vec::new(),
            conjunction_keys: HashSet::new(),
            existentials: Vec::new(),
            existential_domains: Vec::new(),
            unsat: HashMap::new(),
            saturated: false,
        }
    }

    /// ⊥ — [`OWL_NOTHING`].
    pub fn nothing() -> NodeId {
        OWL_NOTHING
    }

    /// Register a class node (label) in the internal taxonomy.
    ///
    /// Idempotent — calling with the same `id` twice overwrites the label.
    pub fn register_class(&mut self, id: NodeId, label: impl Into<String>) {
        self.taxonomy.register_class(id, label);
        self.saturated = false;
    }

    /// Build a reasoner from an existing [`TaxonomyIndex`].
    ///
    /// Clones the taxonomy so the reasoner owns its own working copy.
    /// Does NOT run classification — call [`classify_all`](Self::classify_all)
    /// to saturate.
    pub fn from_taxonomy(taxonomy: &TaxonomyIndex) -> Self {
        Self::with_taxonomy(taxonomy.clone())
    }

    /// Build an [`ELReasoner`] from a historical graph snapshot at `timestamp`.
    ///
    /// Reconstructs the class hierarchy by:
    /// 1. Fetching all `NodeKind::Class` nodes valid at `timestamp`.
    /// 2. Fetching all `subClassOf` edges at `timestamp`.
    /// 3. Fetching all [`EDGE_DISJOINT_WITH`] edges at `timestamp` (what
    ///    `import_turtle` writes for `owl:disjointWith` and
    ///    `owl:AllDisjointClasses`) as disjoint pairs.
    /// 4. Populating a fresh [`TaxonomyIndex`] and wrapping it.
    ///
    /// Does NOT run classification — call [`classify_all`](Self::classify_all)
    /// to derive transitive inferences over the historical snapshot.
    ///
    /// # Errors
    /// Propagates storage errors from [`Graph::get_class_nodes_at`] or
    /// [`Graph::get_edges_of_type_at`].
    ///
    /// [`Graph::get_class_nodes_at`]: crate::graph::Graph::get_class_nodes_at
    /// [`Graph::get_edges_of_type_at`]: crate::graph::Graph::get_edges_of_type_at
    pub async fn from_graph_at(
        graph: &crate::graph::Graph,
        timestamp: u64,
    ) -> crate::error::Result<Self> {
        let mut taxonomy = TaxonomyIndex::new();

        // Step 1 — register all Class nodes valid at timestamp.
        let class_nodes = graph.get_class_nodes_at(timestamp).await?;
        for node in &class_nodes {
            taxonomy.register_class(node.id, &node.label);
        }

        // Step 2 — wire subClassOf edges valid at timestamp.
        // Convention: edge.source = child (subclass), edge.target = parent (superclass)
        // → add_subclass(parent=target, child=source)
        let registered_ids: std::collections::HashSet<NodeId> =
            class_nodes.iter().map(|n| n.id).collect();

        let subclass_edges = graph.get_edges_of_type_at("subClassOf", timestamp).await?;
        for edge in &subclass_edges {
            // Only wire if both endpoints are known Class nodes.
            if registered_ids.contains(&edge.source) && registered_ids.contains(&edge.target) {
                // Ignore errors for self-loops or duplicate edges.
                let _ = taxonomy.add_subclass(edge.target, edge.source);
            }
        }

        let mut reasoner = Self::with_taxonomy(taxonomy);

        // Step 3 — disjoint pairs. Symmetric: the pair is normalised, so an
        // edge written both ways (or twice) is one conjunction.
        let disjoint_edges = graph.get_edges_of_type_at(EDGE_DISJOINT_WITH, timestamp).await?;
        for edge in &disjoint_edges {
            if edge.source != edge.target
                && registered_ids.contains(&edge.source)
                && registered_ids.contains(&edge.target)
            {
                reasoner.add_conjunction(edge.source, edge.target, OWL_NOTHING);
            }
        }

        Ok(reasoner)
    }

    /// Records `left ⊓ right ⊑ result` once (the conjuncts commute).
    /// `false` if it was already there.
    fn add_conjunction(&mut self, left: NodeId, right: NodeId, result: NodeId) -> bool {
        let key = if left <= right { (left, right, result) } else { (right, left, result) };
        if !self.conjunction_keys.insert(key) {
            return false;
        }
        self.conjunctions.push((left, right, result));
        self.saturated = false;
        true
    }

    /// `sub ⊑ sup` as known right now: equal, derived, or through the
    /// asserted hierarchy.
    fn subsumes(&mut self, sub: NodeId, sup: NodeId) -> bool {
        sub == sup || self.derived.contains_key(&(sub, sup)) || self.taxonomy.is_subclass_of(sub, sup)
    }

    /// Records a derived pair; `true` if it is new.
    fn derive(&mut self, sub: NodeId, sup: NodeId, rule: CompletionRule) -> bool {
        if self.derived.contains_key(&(sub, sup)) {
            return false;
        }
        self.derived.insert((sub, sup), rule);
        true
    }

    /// Assert a new SubClassOf axiom: `sub ⊑ super_class`.
    ///
    /// Adds the direct edge to the taxonomy and applies CR1 incrementally to
    /// propagate new transitive inferences. Returns the list of NEW inferences
    /// derived (not including the asserted direct edge itself).
    ///
    /// `super_class` may be [`OWL_NOTHING`]: that declares `sub` unsatisfiable.
    ///
    /// # Errors
    /// Returns [`NopalError::custom`] if `sub == super_class` (self-loop) or
    /// if `sub` is [`OWL_NOTHING`] (⊥ is below every class already).
    pub fn assert_subclass(
        &mut self,
        sub: NodeId,
        super_class: NodeId,
    ) -> Result<Vec<Inference>> {
        if sub == super_class {
            return Err(NopalError::custom(
                "assert_subclass: self-loops are not allowed (sub == super_class)",
            ));
        }
        if sub == OWL_NOTHING {
            return Err(NopalError::custom(
                "assert_subclass: owl:Nothing is a subclass of every class already; it cannot be the subclass",
            ));
        }

        // TaxonomyIndex convention: add_subclass(parent, child) means child ⊑ parent.
        // To assert sub ⊑ super_class: add_subclass(super_class, sub).
        self.taxonomy.add_subclass(super_class, sub)?;
        self.saturated = false;

        let mut new_inferences: Vec<Inference> = Vec::new();
        let mut push = |r: &mut Self, s: NodeId, sup: NodeId| {
            if r.derive(s, sup, CompletionRule::CR1) {
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub: s, super_class: sup },
                    rule: CompletionRule::CR1,
                });
            }
        };

        // CR1 forward: sub ⊑ super_class and super_class ⊑ X → derive sub ⊑ X
        let ancestors_of_super: Vec<NodeId> = self.taxonomy.ancestors(super_class);
        for ancestor in &ancestors_of_super {
            push(self, sub, *ancestor);
        }

        // CR1 backward: Y ⊑ sub and sub ⊑ super_class → derive Y ⊑ super_class (and Y ⊑ X)
        let descendants_of_sub: Vec<NodeId> = self.taxonomy.descendants(sub);
        for desc in &descendants_of_sub {
            push(self, *desc, super_class);
            for ancestor in &ancestors_of_super {
                push(self, *desc, *ancestor);
            }
        }

        Ok(new_inferences)
    }

    /// Assert a conjunction inclusion axiom: `left ⊓ right ⊑ result`.
    ///
    /// Records the axiom and immediately applies CR2 to derive any new
    /// `X ⊑ result` inferences for all classes X already known to be
    /// subclasses of both `left` and `right`. `result` may be
    /// [`OWL_NOTHING`]: that makes `left` and `right` disjoint.
    pub fn assert_conjunction(
        &mut self,
        left: NodeId,
        right: NodeId,
        result: NodeId,
    ) -> Result<Vec<Inference>> {
        self.add_conjunction(left, right, result);
        // Apply CR2 immediately for existing knowledge.
        Ok(self.apply_cr2_for_conjunction(left, right, result))
    }

    /// Assert that `classes` are pairwise disjoint (`owl:AllDisjointClasses`;
    /// two classes = `owl:disjointWith`): `ci ⊓ cj ⊑ ⊥` for every pair.
    ///
    /// Applies CR2 for each pair now and returns what it derived; a class
    /// below two of them becomes unsatisfiable. Asserting a pair again is a
    /// no-op.
    ///
    /// # Errors
    /// Fewer than two classes, a repeated class, or [`OWL_NOTHING`] among
    /// them (⊥ is disjoint with everything already).
    pub fn assert_disjoint(&mut self, classes: &[NodeId]) -> Result<Vec<Inference>> {
        if classes.len() < 2 {
            return Err(NopalError::custom("assert_disjoint: needs at least two classes"));
        }
        let distinct: HashSet<NodeId> = classes.iter().copied().collect();
        if distinct.len() != classes.len() {
            return Err(NopalError::custom("assert_disjoint: a class appears twice"));
        }
        if distinct.contains(&OWL_NOTHING) {
            return Err(NopalError::custom(
                "assert_disjoint: owl:Nothing is disjoint with every class already",
            ));
        }
        let mut out = Vec::new();
        for (i, &a) in classes.iter().enumerate() {
            for &b in &classes[i + 1..] {
                out.extend(self.assert_conjunction(a, b, OWL_NOTHING)?);
            }
        }
        Ok(out)
    }

    /// Assert any [`Axiom`]: dispatches to the matching `assert_*` method.
    pub fn assert_axiom(&mut self, axiom: Axiom) -> Result<Vec<Inference>> {
        match axiom {
            Axiom::SubClassOf { sub, super_class } => self.assert_subclass(sub, super_class),
            Axiom::ConjunctionInclusion { left, right, result } => self.assert_conjunction(left, right, result),
            Axiom::ExistentialRestriction { sub, role, filler } => self.assert_existential(sub, &role, filler),
            Axiom::ExistentialDomain { role, filler, result } => self.assert_existential_domain(&role, filler, result),
            Axiom::DisjointClasses { classes } => self.assert_disjoint(&classes),
        }
    }

    /// Assert a CR3 left-side axiom: `sub ⊑ ∃role.filler`.
    ///
    /// Records the axiom and immediately applies CR3 against all known
    /// `∃role.C ⊑ D` domains where `filler ⊑ C` (direct or derived),
    /// deriving `sub ⊑ D` for each match.
    ///
    /// # Errors
    /// Returns an error if `sub == filler`.
    pub fn assert_existential(
        &mut self,
        sub: NodeId,
        role: &str,
        filler: NodeId,
    ) -> Result<Vec<Inference>> {
        if sub == filler {
            return Err(NopalError::custom(
                "assert_existential: sub == filler is not meaningful",
            ));
        }
        self.existentials.push((sub, role.to_string(), filler));
        self.saturated = false;
        Ok(self.apply_cr3_for_existential(sub, role, filler))
    }

    /// Assert a CR3 right-side axiom: `∃role.filler ⊑ result`.
    ///
    /// Records the axiom and immediately applies CR3 against all known
    /// `A ⊑ ∃role.B` restrictions where `B ⊑ filler` (direct or derived),
    /// deriving `A ⊑ result` for each match.
    pub fn assert_existential_domain(
        &mut self,
        role: &str,
        filler: NodeId,
        result: NodeId,
    ) -> Result<Vec<Inference>> {
        self.existential_domains.push((role.to_string(), filler, result));
        self.saturated = false;
        Ok(self.apply_cr3_for_domain(role, filler, result))
    }

    /// Apply CR3 given a new left-side axiom `sub ⊑ ∃role.filler`.
    ///
    /// Scans existing `∃role.C ⊑ D` domains; fires when `filler ⊑ C`.
    fn apply_cr3_for_existential(
        &mut self,
        sub: NodeId,
        role: &str,
        filler: NodeId,
    ) -> Vec<Inference> {
        let mut new_inferences: Vec<Inference> = Vec::new();
        let domains = self.existential_domains.clone();

        for (r, c, d) in &domains {
            if r != role || sub == *d {
                continue;
            }
            if self.subsumes(filler, *c)
                && !self.taxonomy.direct_parents(sub).contains(d)
                && self.derive(sub, *d, CompletionRule::CR3)
            {
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub, super_class: *d },
                    rule: CompletionRule::CR3,
                });
            }
        }

        new_inferences
    }

    /// Apply CR3 given a new right-side axiom `∃role.filler ⊑ result`.
    ///
    /// Scans existing `A ⊑ ∃role.B` restrictions; fires when `B ⊑ filler`.
    fn apply_cr3_for_domain(
        &mut self,
        role: &str,
        filler: NodeId,
        result: NodeId,
    ) -> Vec<Inference> {
        let mut new_inferences: Vec<Inference> = Vec::new();
        let existentials = self.existentials.clone();

        for (a, r, b) in &existentials {
            if r != role || *a == result {
                continue;
            }
            if self.subsumes(*b, filler)
                && !self.taxonomy.direct_parents(*a).contains(&result)
                && self.derive(*a, result, CompletionRule::CR3)
            {
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub: *a, super_class: result },
                    rule: CompletionRule::CR3,
                });
            }
        }

        new_inferences
    }

    /// Apply CR2 for a single conjunction axiom `left ⊓ right ⊑ result`.
    ///
    /// For every class X of the hierarchy (registered, or an endpoint of an
    /// asserted subclass axiom) such that X ⊑ left AND X ⊑ right (asserted
    /// or derived so far), derive X ⊑ result. A class known only through
    /// other conjunctions or existentials waits for `classify_all`.
    fn apply_cr2_for_conjunction(
        &mut self,
        left: NodeId,
        right: NodeId,
        result: NodeId,
    ) -> Vec<Inference> {
        let all_nodes = self.taxonomy.hierarchy_ids();
        let mut new_inferences: Vec<Inference> = Vec::new();

        for &x in &all_nodes {
            if x == result {
                continue;
            }
            if self.subsumes(x, left)
                && self.subsumes(x, right)
                && !self.taxonomy.direct_parents(x).contains(&result)
                && self.derive(x, result, CompletionRule::CR2)
            {
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub: x, super_class: result },
                    rule: CompletionRule::CR2,
                });
            }
        }

        new_inferences
    }

    /// Run CR1–CR4 to saturation over all currently known axioms.
    ///
    /// EL completion: every class X has a set of subsumers S(X), every new
    /// `Y ∈ S(X)` goes through a work queue and fires the rules whose
    /// premises mention Y, and `A ⊑ ∃R.B` is inherited by the subclasses of
    /// A. So what one rule derives feeds the others (a pair derived by CR3
    /// can satisfy a CR2 conjunction, a class made unsatisfiable by CR2 makes
    /// its existential holders unsatisfiable by CR4) and the result is the
    /// fixpoint. Until 0.6.17 this was one pass in rule order, and CR2 only
    /// saw the asserted hierarchy.
    ///
    /// Returns the inferences that are new since the last call (asserted
    /// edges excluded), sorted. Idempotent: with nothing asserted since the
    /// last call it returns an empty Vec without recomputing.
    pub fn classify_all(&mut self) -> Vec<Inference> {
        if self.saturated {
            return Vec::new();
        }
        let completion = Completion::run(self);

        let mut new_inferences: Vec<Inference> = Vec::new();
        for (&x, sups) in &completion.s {
            let direct = self.taxonomy.direct_parents(x);
            for (&y, &rule) in sups {
                if y == x || direct.contains(&y) || self.derived.contains_key(&(x, y)) {
                    continue;
                }
                self.derived.insert((x, y), rule);
                new_inferences.push(Inference { axiom: Axiom::SubClassOf { sub: x, super_class: y }, rule });
            }
        }
        new_inferences.sort_by_key(|i| match &i.axiom {
            Axiom::SubClassOf { sub, super_class } => (*sub, *super_class),
            _ => (NodeId::nil(), NodeId::nil()),
        });
        self.unsat = completion.unsat;
        self.saturated = true;
        new_inferences
    }

    /// Returns `true` if `sub` is known to be a subclass of `super_class`,
    /// considering both direct (asserted) and inferred axioms.
    pub fn is_subclass_of(&mut self, sub: NodeId, super_class: NodeId) -> bool {
        if sub == super_class {
            return false;
        }
        // Check derived set first (O(1)).
        if self.derived.contains_key(&(sub, super_class)) {
            return true;
        }
        // Fall through to taxonomy BFS (handles the direct-edge case).
        self.taxonomy.is_subclass_of(sub, super_class)
    }

    /// `false` if `class` is below ⊥ (no individual can belong to it).
    /// Runs [`classify_all`](Self::classify_all) first (a no-op when nothing
    /// changed). ⊥ itself is unsatisfiable.
    pub fn is_satisfiable(&mut self, class: NodeId) -> bool {
        self.classify_all();
        class != OWL_NOTHING && !self.unsat.contains_key(&class)
    }

    /// Every class below ⊥, sorted by id. Runs
    /// [`classify_all`](Self::classify_all) first, so it is complete.
    /// ⊥ itself is not listed.
    pub fn unsatisfiable_classes(&mut self) -> Vec<NodeId> {
        self.classify_all();
        let mut out: Vec<NodeId> = self.unsat.keys().copied().collect();
        out.sort();
        out
    }

    /// Why `class` is unsatisfiable, or `None` if it is satisfiable. Runs
    /// [`classify_all`](Self::classify_all) first.
    pub fn why_unsatisfiable(&mut self, class: NodeId) -> Option<Unsatisfiable> {
        self.classify_all();
        self.unsat.get(&class).cloned()
    }

    /// All superclasses of `node` — asserted ancestors and derived ones.
    /// Includes [`OWL_NOTHING`] for an unsatisfiable class once classified.
    pub fn superclasses(&self, node: NodeId) -> Vec<NodeId> {
        let mut out = self.taxonomy.ancestors(node);
        let known: HashSet<NodeId> = out.iter().copied().collect();
        let mut extra: Vec<NodeId> = self
            .derived
            .keys()
            .filter(|(s, sup)| *s == node && !known.contains(sup))
            .map(|(_, sup)| *sup)
            .collect();
        extra.sort();
        out.extend(extra);
        out
    }

    /// All subclasses of `node` — asserted descendants and derived ones.
    pub fn subclasses(&mut self, node: NodeId) -> Vec<NodeId> {
        let mut out = self.taxonomy.descendants(node);
        let known: HashSet<NodeId> = out.iter().copied().collect();
        let mut extra: Vec<NodeId> = self
            .derived
            .keys()
            .filter(|(sub, sup)| *sup == node && !known.contains(sub))
            .map(|(sub, _)| *sub)
            .collect();
        extra.sort();
        out.extend(extra);
        out
    }

    /// Number of direct subClassOf edges currently in the working taxonomy.
    pub fn axiom_count(&self) -> usize {
        self.taxonomy.edge_count()
    }

    /// All inferences derived since construction or the last
    /// [`clear_derived`](Self::clear_derived) call, each with the rule that
    /// derived it. (Until 0.6.17 every one was labelled CR1.)
    pub fn derived_inferences(&self) -> Vec<Inference> {
        self.derived
            .iter()
            .map(|(&(sub, super_class), &rule)| Inference {
                axiom: Axiom::SubClassOf { sub, super_class },
                rule,
            })
            .collect()
    }

    /// Clear the derived inference set (and the unsatisfiability causes).
    ///
    /// Does NOT affect the underlying taxonomy direct edges. Useful for
    /// re-running [`classify_all`](Self::classify_all) after adding more axioms.
    pub fn clear_derived(&mut self) {
        self.derived.clear();
        self.unsat.clear();
        self.saturated = false;
    }
}

// ---------------------------------------------------------------------------
// EL completion (classify_all)
// ---------------------------------------------------------------------------

enum Work {
    /// `y` was added to S(x).
    Sub(NodeId, NodeId),
    /// `x ⊑ ∃role.b` became known (asserted on x or inherited from a superclass).
    Link(NodeId, String, NodeId),
}

/// One classification run: subsumer sets with the rule that first put each
/// subsumer there, the existential links, and the causes of ⊥.
struct Completion {
    /// x → { y ∈ S(x) → rule }. Reflexive pairs are implicit.
    s: HashMap<NodeId, HashMap<NodeId, CompletionRule>>,
    links: HashSet<(NodeId, String, NodeId)>,
    /// b → the (x, role) with `x ⊑ ∃role.b`.
    pred: HashMap<NodeId, Vec<(NodeId, String)>>,
    queue: VecDeque<Work>,
    unsat: HashMap<NodeId, Unsatisfiable>,
}

impl Completion {
    fn run(r: &mut ELReasoner) -> Self {
        // Indices over the told axioms.
        let mut conj_by_operand: HashMap<NodeId, Vec<(NodeId, NodeId, NodeId, NodeId)>> = HashMap::new();
        for &(l, rr, res) in &r.conjunctions {
            conj_by_operand.entry(l).or_default().push((rr, l, rr, res));
            if rr != l {
                conj_by_operand.entry(rr).or_default().push((l, l, rr, res));
            }
        }
        let mut exist_by_sub: HashMap<NodeId, Vec<(String, NodeId)>> = HashMap::new();
        for (a, role, b) in &r.existentials {
            exist_by_sub.entry(*a).or_default().push((role.clone(), *b));
        }
        let mut domains: HashMap<(String, NodeId), Vec<NodeId>> = HashMap::new();
        for (role, c, d) in &r.existential_domains {
            domains.entry((role.clone(), *c)).or_default().push(*d);
        }

        // Every class the axioms mention, registered or not.
        let mut universe: HashSet<NodeId> = r.taxonomy.hierarchy_ids().into_iter().collect();
        for &(l, rr, res) in &r.conjunctions {
            universe.extend([l, rr, res]);
        }
        for (a, _, b) in &r.existentials {
            universe.extend([*a, *b]);
        }
        for (_, c, d) in &r.existential_domains {
            universe.extend([*c, *d]);
        }
        universe.remove(&OWL_NOTHING);
        let mut universe: Vec<NodeId> = universe.into_iter().collect();
        universe.sort(); // deterministic first causes

        let mut c = Completion {
            s: HashMap::new(),
            links: HashSet::new(),
            pred: HashMap::new(),
            queue: VecDeque::new(),
            unsat: HashMap::new(),
        };
        for &x in &universe {
            c.s.entry(x).or_default();
            c.queue.push_back(Work::Sub(x, x));
        }

        while let Some(work) = c.queue.pop_front() {
            match work {
                Work::Sub(x, y) => {
                    // CR1 over the asserted hierarchy.
                    for z in r.taxonomy.direct_parents(y) {
                        let cause = if y == x { Unsatisfiable::Asserted } else { Unsatisfiable::Inherited { parent: y } };
                        c.add(x, z, CompletionRule::CR1, || cause);
                    }
                    // CR2: the other conjunct already in S(x).
                    if let Some(list) = conj_by_operand.get(&y) {
                        for &(other, l, rr, res) in list {
                            if c.has(x, other) {
                                c.add(x, res, CompletionRule::CR2, || Unsatisfiable::Disjoint { left: l, right: rr });
                            }
                        }
                    }
                    // `y ⊑ ∃R.B` holds for x too.
                    if let Some(list) = exist_by_sub.get(&y) {
                        for (role, b) in list {
                            c.link(x, role.clone(), *b);
                        }
                    }
                    // x is the filler of some `w ⊑ ∃R.x`: CR3 / CR4 for w.
                    if let Some(holders) = c.pred.get(&x).cloned() {
                        for (w, role) in holders {
                            if y == OWL_NOTHING {
                                c.add(w, OWL_NOTHING, CompletionRule::CR4, || Unsatisfiable::Existential { role: role.clone(), filler: x, rule: CompletionRule::CR4 });
                            }
                            if let Some(ds) = domains.get(&(role.clone(), y)) {
                                for &d in ds {
                                    c.add(w, d, CompletionRule::CR3, || Unsatisfiable::Existential { role: role.clone(), filler: x, rule: CompletionRule::CR3 });
                                }
                            }
                        }
                    }
                }
                Work::Link(x, role, b) => {
                    let mut fillers: Vec<NodeId> = c.s.get(&b).map(|m| m.keys().copied().collect()).unwrap_or_default();
                    fillers.push(b);
                    for f in fillers {
                        if f == OWL_NOTHING {
                            c.add(x, OWL_NOTHING, CompletionRule::CR4, || Unsatisfiable::Existential { role: role.clone(), filler: b, rule: CompletionRule::CR4 });
                        }
                        if let Some(ds) = domains.get(&(role.clone(), f)) {
                            for &d in ds {
                                c.add(x, d, CompletionRule::CR3, || Unsatisfiable::Existential { role: role.clone(), filler: b, rule: CompletionRule::CR3 });
                            }
                        }
                    }
                }
            }
        }
        c
    }

    fn has(&self, x: NodeId, y: NodeId) -> bool {
        x == y || self.s.get(&x).is_some_and(|m| m.contains_key(&y))
    }

    /// `y ∈ S(x)` by `rule`; `cause` is recorded when `y` is ⊥.
    fn add(&mut self, x: NodeId, y: NodeId, rule: CompletionRule, cause: impl FnOnce() -> Unsatisfiable) {
        if x == y {
            return;
        }
        let set = self.s.entry(x).or_default();
        if set.contains_key(&y) {
            return;
        }
        set.insert(y, rule);
        if y == OWL_NOTHING {
            self.unsat.entry(x).or_insert_with(cause);
        }
        self.queue.push_back(Work::Sub(x, y));
    }

    fn link(&mut self, x: NodeId, role: String, b: NodeId) {
        if self.links.insert((x, role.clone(), b)) {
            self.pred.entry(b).or_default().push((x, role.clone()));
            self.queue.push_back(Work::Link(x, role, b));
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn nid() -> NodeId {
        Uuid::new_v4()
    }

    // Build a reasoner with a pre-wired taxonomy chain.
    // Returns (reasoner, ids...) in ascending hierarchy order (most general first).
    fn chain(labels: &[&str]) -> (ELReasoner, Vec<NodeId>) {
        let mut tax = TaxonomyIndex::new();
        let ids: Vec<NodeId> = labels.iter().map(|_| nid()).collect();
        for (i, label) in labels.iter().enumerate() {
            tax.register_class(ids[i], *label);
        }
        // Wire: ids[0] is root (most general), ids[last] is leaf (most specific)
        // add_subclass(parent, child): child ⊑ parent
        for i in 0..ids.len() - 1 {
            tax.add_subclass(ids[i], ids[i + 1]).unwrap();
        }
        (ELReasoner::from_taxonomy(&tax), ids)
    }

    // -----------------------------------------------------------------------
    // Test 1 — basic two-hop chain
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_basic_chain() {
        // A ⊑ B (direct), B ⊑ C (direct)
        let (mut r, ids) = chain(&["C", "B", "A"]); // ids[0]=C (root), ids[1]=B, ids[2]=A (leaf)
        let a = ids[2];
        let b = ids[1];
        let c = ids[0];

        let inferred = r.classify_all();

        // A ⊑ C must be derived (two hops)
        assert!(r.is_subclass_of(a, c), "A must be subclass of C (transitively)");
        assert!(r.is_subclass_of(a, b), "A must be subclass of B (directly)");
        assert!(!r.is_subclass_of(c, a), "C must NOT be subclass of A");

        // Only A ⊑ C is a CR1 derivation (A ⊑ B is direct, B ⊑ C is direct)
        assert_eq!(inferred.len(), 1);
        assert_eq!(
            inferred[0].axiom,
            Axiom::SubClassOf { sub: a, super_class: c }
        );
        assert_eq!(inferred[0].rule, CompletionRule::CR1);
    }

    // -----------------------------------------------------------------------
    // Test 2 — three-hop chain
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_three_hop_chain() {
        // LivingThing ← Animal ← Mammal ← Dog
        let (mut r, ids) = chain(&["LivingThing", "Animal", "Mammal", "Dog"]);
        let living = ids[0];
        let animal = ids[1];
        let mammal = ids[2];
        let dog = ids[3];

        r.classify_all();

        assert!(r.is_subclass_of(dog, animal));
        assert!(r.is_subclass_of(dog, living));
        assert!(r.is_subclass_of(mammal, living));
        assert!(!r.is_subclass_of(living, dog));

        // 3 CR1 derivations: Dog⊑Animal, Dog⊑LivingThing, Mammal⊑LivingThing
        let derived = r.derived_inferences();
        assert_eq!(derived.len(), 3, "expected exactly 3 CR1 inferences");

        // Direct edges (Dog⊑Mammal, Mammal⊑Animal, Animal⊑LivingThing) must NOT be in derived
        let has_dog_mammal = derived.iter().any(|i| {
            i.axiom == Axiom::SubClassOf { sub: dog, super_class: mammal }
        });
        assert!(!has_dog_mammal, "Dog⊑Mammal is direct, not a CR1 derivation");
    }

    // -----------------------------------------------------------------------
    // Test 3 — single direct edge → no CR1 inference
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_no_inference_for_direct_only() {
        let mut tax = TaxonomyIndex::new();
        let a = nid();
        let b = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        // Only A → B (A is parent, B is child: B ⊑ A)
        tax.add_subclass(a, b).unwrap();

        let mut r = ELReasoner::from_taxonomy(&tax);
        let inferred = r.classify_all();

        assert!(inferred.is_empty(), "no transitive inference possible with a single edge");
        assert!(r.is_subclass_of(b, a), "direct edge B⊑A still detected");
        assert!(r.derived_inferences().is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 4 — incremental assert_subclass
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_incremental_assert_subclass() {
        let mut tax = TaxonomyIndex::new();
        let a = nid();
        let b = nid();
        let c = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        // Start: only A → B (B ⊑ A)
        tax.add_subclass(a, b).unwrap();

        let mut r = ELReasoner::from_taxonomy(&tax);

        // Incrementally assert B ⊑ C (i.e. C is subclass of B… wait, let me
        // be careful: assert_subclass(sub=c, super_class=b) means c ⊑ b).
        // Then by CR1: c ⊑ b and b ⊑ a → c ⊑ a.
        let inferred = r.assert_subclass(c, b).unwrap();

        // c ⊑ a must be derived immediately (no need to call classify_all)
        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: c, super_class: a }),
            "CR1 must derive c ⊑ a immediately"
        );
        assert!(r.is_subclass_of(c, a), "is_subclass_of(c, a) must be true after assert");
        assert_eq!(inferred[0].rule, CompletionRule::CR1);
    }

    // -----------------------------------------------------------------------
    // Test 5 — diamond hierarchy: no duplicate derivation
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_diamond_hierarchy() {
        // A is root; B ⊑ A, C ⊑ A, D ⊑ B, D ⊑ C → diamond
        let mut tax = TaxonomyIndex::new();
        let a = nid();
        let b = nid();
        let c = nid();
        let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        // add_subclass(parent, child)
        tax.add_subclass(a, b).unwrap(); // b ⊑ a
        tax.add_subclass(a, c).unwrap(); // c ⊑ a
        tax.add_subclass(b, d).unwrap(); // d ⊑ b
        tax.add_subclass(c, d).unwrap(); // d ⊑ c

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.classify_all();

        assert!(r.is_subclass_of(d, a), "D must be subclass of A");

        // D ⊑ A should appear EXACTLY ONCE in derived
        let derived = r.derived_inferences();
        let count_d_a = derived
            .iter()
            .filter(|i| i.axiom == Axiom::SubClassOf { sub: d, super_class: a })
            .count();
        assert_eq!(count_d_a, 1, "D ⊑ A must appear exactly once (no duplicates)");
    }

    // -----------------------------------------------------------------------
    // Test 6 — self-loop rejected
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_self_loop_rejected() {
        let mut r = ELReasoner::new();
        let a = nid();
        let result = r.assert_subclass(a, a);
        assert!(result.is_err(), "self-loop must return Err");
    }

    // -----------------------------------------------------------------------
    // Test 7 — from_taxonomy snapshot
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_from_taxonomy_snapshot() {
        let mut tax = TaxonomyIndex::new();
        let animal = nid();
        let mammal = nid();
        let dog = nid();
        tax.register_class(animal, "Animal");
        tax.register_class(mammal, "Mammal");
        tax.register_class(dog, "Dog");
        tax.add_subclass(animal, mammal).unwrap(); // mammal ⊑ animal
        tax.add_subclass(mammal, dog).unwrap();    // dog ⊑ mammal

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.classify_all();

        assert!(r.is_subclass_of(dog, animal));

        // superclasses(dog) = [mammal, animal]
        let sup = r.superclasses(dog);
        assert_eq!(sup.len(), 2);
        assert!(sup.contains(&mammal));
        assert!(sup.contains(&animal));

        // subclasses(animal) = [mammal, dog]
        let sub = r.subclasses(animal);
        assert_eq!(sub.len(), 2);
        assert!(sub.contains(&mammal));
        assert!(sub.contains(&dog));
    }

    // -----------------------------------------------------------------------
    // Test 8 — clear_derived
    // -----------------------------------------------------------------------
    #[test]
    fn test_cr1_clear_derived() {
        let (mut r, ids) = chain(&["C", "B", "A"]); // A ⊑ B ⊑ C
        let a = ids[2];
        let c = ids[0];

        r.classify_all();
        assert!(!r.derived_inferences().is_empty(), "derived set must be non-empty after classify");

        r.clear_derived();
        assert!(r.derived_inferences().is_empty(), "derived set must be empty after clear");

        // Direct taxonomy still works (is_subclass_of falls through to BFS)
        assert!(r.is_subclass_of(a, c), "BFS fallback must still detect A ⊑ C after clear_derived");

        // But derived_inferences remains empty (BFS result is NOT re-added to derived)
        assert!(r.derived_inferences().is_empty());
    }

    // -----------------------------------------------------------------------
    // Step 8 — from_graph_at tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_snapshot_at_before_class_added() {
        let dir = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::Graph::open(dir.path().to_str().unwrap()).await.unwrap();

        // Snapshot BEFORE adding any classes.
        let timestamp_before = 0u64;

        let mut reasoner = ELReasoner::from_graph_at(&graph, timestamp_before).await.unwrap();
        reasoner.classify_all();

        // No classes should be in the taxonomy.
        assert_eq!(reasoner.axiom_count(), 0, "no edges before any class is added");
    }

    #[tokio::test]
    async fn test_snapshot_at_after_class_added() {
        use crate::types::{Node, NodeKind};

        let dir = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::Graph::open(dir.path().to_str().unwrap()).await.unwrap();

        // Add a Class node.
        let mut node = Node::new("Animal");
        node.kind = NodeKind::Class;
        graph.add_node(node).await.unwrap();

        // Snapshot after adding — use a large timestamp (all nodes are valid).
        let timestamp_after = u64::MAX;

        let mut reasoner = ELReasoner::from_graph_at(&graph, timestamp_after).await.unwrap();
        reasoner.classify_all();

        // "Animal" class should appear.
        assert!(
            reasoner.taxonomy.find_by_label("Animal").is_some(),
            "Animal class should be visible at timestamp_after"
        );
    }

    #[tokio::test]
    async fn test_snapshot_derives_cr1_at_point() {
        use crate::types::{Node, NodeKind, Edge};

        let dir = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::Graph::open(dir.path().to_str().unwrap()).await.unwrap();

        // Add class nodes.
        let mut animal = Node::new("Animal");
        animal.kind = NodeKind::Class;
        let animal_id = graph.add_node(animal).await.unwrap();

        let mut mammal = Node::new("Mammal");
        mammal.kind = NodeKind::Class;
        let mammal_id = graph.add_node(mammal).await.unwrap();

        let mut dog = Node::new("Dog");
        dog.kind = NodeKind::Class;
        let dog_id = graph.add_node(dog).await.unwrap();

        // Add subClassOf edges: Mammal ⊑ Animal, Dog ⊑ Mammal
        graph.add_edge(Edge {
            id: Uuid::new_v4(),
            source: mammal_id,
            target: animal_id,
            edge_type: "subClassOf".to_string(),
            properties: Default::default(),
        }).await.unwrap();
        graph.add_edge(Edge {
            id: Uuid::new_v4(),
            source: dog_id,
            target: mammal_id,
            edge_type: "subClassOf".to_string(),
            properties: Default::default(),
        }).await.unwrap();

        // Build reasoner at current timestamp.
        let mut reasoner = ELReasoner::from_graph_at(&graph, u64::MAX).await.unwrap();
        reasoner.classify_all();

        // Dog ⊑ Animal (transitively via CR1)
        let animal_tax_id = reasoner.taxonomy.find_by_label("Animal").unwrap();
        let dog_tax_id = reasoner.taxonomy.find_by_label("Dog").unwrap();
        assert!(
            reasoner.is_subclass_of(dog_tax_id, animal_tax_id),
            "CR1 must derive Dog ⊑ Animal from historical snapshot"
        );
    }

    // -----------------------------------------------------------------------
    // Step 9 — CR2 tests
    // -----------------------------------------------------------------------

    // Test 1 — basic CR2: A ⊑ B, A ⊑ C, B ⊓ C ⊑ D → derive A ⊑ D
    #[test]
    fn test_cr2_basic_conjunction() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        tax.add_subclass(b, a).unwrap(); // A ⊑ B
        tax.add_subclass(c, a).unwrap(); // A ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        let inferred = r.assert_conjunction(b, c, d).unwrap();

        // A ⊑ D must be derived immediately by CR2
        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR2 must derive A ⊑ D"
        );
        assert!(inferred.iter().all(|i| i.rule == CompletionRule::CR2));
    }

    // Test 2 — CR2 must NOT fire when only one conjunct is satisfied
    #[test]
    fn test_cr2_no_fire_without_both_conjuncts() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        // A ⊑ B, but NOT A ⊑ C
        tax.add_subclass(b, a).unwrap();

        let mut r = ELReasoner::from_taxonomy(&tax);
        let inferred = r.assert_conjunction(b, c, d).unwrap();

        // A ⊑ D must NOT be derived
        let has_a_d = inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d });
        assert!(!has_a_d, "CR2 must not fire when only one conjunct is satisfied");
    }

    // Test 3 — CR2 interacts with CR1: A ⊑ B ⊑ E, A ⊑ C, E ⊓ C ⊑ D → derive A ⊑ D
    #[test]
    fn test_cr2_chain_with_cr1() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid(); let e = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        tax.register_class(e, "E");
        tax.add_subclass(b, a).unwrap(); // A ⊑ B
        tax.add_subclass(e, b).unwrap(); // B ⊑ E  → A ⊑ E (by CR1)
        tax.add_subclass(c, a).unwrap(); // A ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.classify_all(); // saturate CR1 first → A ⊑ E derived

        // E ⊓ C ⊑ D: since A ⊑ E (by CR1) and A ⊑ C (direct), derive A ⊑ D
        let inferred = r.assert_conjunction(e, c, d).unwrap();

        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR2 must fire after CR1 derives A ⊑ E"
        );
    }

    // Test 4 — self-loop in conjunction result is skipped (result == subject)
    #[test]
    fn test_cr2_self_loop_skipped() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.add_subclass(b, a).unwrap(); // A ⊑ B
        tax.add_subclass(c, a).unwrap(); // A ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        // assert B ⊓ C ⊑ A — but A ⊑ A is a self-loop, should NOT appear in derived
        let inferred = r.assert_conjunction(b, c, a).unwrap();

        // A ⊑ A must not be produced (result == a, and x == result is skipped)
        let has_self = inferred.iter().any(|i| {
            if let Axiom::SubClassOf { sub, super_class } = i.axiom {
                sub == super_class
            } else { false }
        });
        assert!(!has_self, "CR2 must not generate X ⊑ X self-loops");
    }

    // -----------------------------------------------------------------------
    // CR3 tests — existential rule
    // -----------------------------------------------------------------------

    // Test CR3-1 — basic: A⊑∃R.B, B⊑C (direct), ∃R.C⊑D → derive A⊑D
    #[test]
    fn test_cr3_basic_existential() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        // B ⊑ C (direct)
        tax.add_subclass(c, b).unwrap();

        let mut r = ELReasoner::from_taxonomy(&tax);

        // Assert A ⊑ ∃role.B  and  ∃role.C ⊑ D
        r.assert_existential(a, "role", b).unwrap();
        let inferred = r.assert_existential_domain("role", c, d).unwrap();

        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR3 must derive A ⊑ D"
        );
        assert!(inferred.iter().all(|i| i.rule == CompletionRule::CR3));
    }

    // Test CR3-2 — filler matches domain exactly (B == C)
    #[test]
    fn test_cr3_filler_equals_domain_filler() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(d, "D");

        let mut r = ELReasoner::from_taxonomy(&tax);

        // A ⊑ ∃R.B  and  ∃R.B ⊑ D  (same filler → same B==C case)
        r.assert_existential(a, "R", b).unwrap();
        let inferred = r.assert_existential_domain("R", b, d).unwrap();

        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR3 must fire when filler == domain filler"
        );
    }

    // Test CR3-3 — no fire when roles differ
    #[test]
    fn test_cr3_no_fire_different_roles() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A"); tax.register_class(b, "B");
        tax.register_class(c, "C"); tax.register_class(d, "D");
        tax.add_subclass(c, b).unwrap(); // B ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.assert_existential(a, "roleX", b).unwrap();
        let inferred = r.assert_existential_domain("roleY", c, d).unwrap();

        assert!(
            !inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR3 must NOT fire when roles differ"
        );
    }

    // Test CR3-4 — no fire when B is NOT subclass of C
    #[test]
    fn test_cr3_no_fire_filler_not_subclass() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A"); tax.register_class(b, "B");
        tax.register_class(c, "C"); tax.register_class(d, "D");
        // B and C are unrelated

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.assert_existential(a, "R", b).unwrap();
        let inferred = r.assert_existential_domain("R", c, d).unwrap();

        assert!(
            !inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR3 must NOT fire when B is not subclass of C"
        );
    }

    // Test CR3-5 — CR3 + CR1: A⊑∃R.B, B⊑M⊑C (chain), ∃R.C⊑D → derive A⊑D
    #[test]
    fn test_cr3_with_cr1_chain() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let m = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A"); tax.register_class(b, "B");
        tax.register_class(m, "M"); tax.register_class(c, "C"); tax.register_class(d, "D");
        // B ⊑ M ⊑ C (chain, so B ⊑ C transitively via CR1)
        tax.add_subclass(m, b).unwrap(); // B ⊑ M
        tax.add_subclass(c, m).unwrap(); // M ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.classify_all(); // saturate CR1: B ⊑ C derived

        r.assert_existential(a, "R", b).unwrap();
        let inferred = r.assert_existential_domain("R", c, d).unwrap();

        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "CR3 must derive A ⊑ D when B ⊑ C is inferred by CR1"
        );
    }

    // Test CR3-6 — idempotent: asserting same axioms twice produces no duplicates
    #[test]
    fn test_cr3_idempotent() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let d = nid();
        tax.register_class(a, "A"); tax.register_class(b, "B"); tax.register_class(d, "D");

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.assert_existential(a, "R", b).unwrap();
        let first  = r.assert_existential_domain("R", b, d).unwrap();
        let second = r.assert_existential_domain("R", b, d).unwrap();

        assert!(!first.is_empty(), "first call must derive A ⊑ D");
        assert!(second.is_empty(), "second call must not duplicate A ⊑ D");
    }

    // Test CR3-7 — classify_all saturates CR3
    #[test]
    fn test_cr3_via_classify_all() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A"); tax.register_class(b, "B");
        tax.register_class(c, "C"); tax.register_class(d, "D");
        tax.add_subclass(c, b).unwrap(); // B ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        r.assert_existential(a, "R", b).unwrap();
        r.assert_existential_domain("R", c, d).unwrap();
        // Clear derived to simulate fresh classification.
        r.clear_derived();

        let inferred = r.classify_all();

        assert!(
            inferred.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: a, super_class: d }),
            "classify_all must saturate CR3: A ⊑ D"
        );
    }

    // Test 5 — idempotent: assert same conjunction twice → no duplicate derivations
    #[test]
    fn test_cr2_idempotent() {
        let mut tax = TaxonomyIndex::new();
        let a = nid(); let b = nid(); let c = nid(); let d = nid();
        tax.register_class(a, "A");
        tax.register_class(b, "B");
        tax.register_class(c, "C");
        tax.register_class(d, "D");
        tax.add_subclass(b, a).unwrap(); // A ⊑ B
        tax.add_subclass(c, a).unwrap(); // A ⊑ C

        let mut r = ELReasoner::from_taxonomy(&tax);
        let first  = r.assert_conjunction(b, c, d).unwrap();
        let second = r.assert_conjunction(b, c, d).unwrap();

        // Second call must not produce A ⊑ D again (already in derived set)
        assert!(!first.is_empty(), "first call should derive A ⊑ D");
        assert!(second.is_empty(), "second call must produce no new inferences (idempotent)");
    }

    // -----------------------------------------------------------------------
    // #235 — disjoint classes and ⊥ (owl:Nothing)
    // -----------------------------------------------------------------------

    /// Receta and Utensilio are disjoint; Hibrido is declared below both.
    fn disjoint_fixture() -> (ELReasoner, NodeId, NodeId, NodeId, NodeId) {
        let mut r = ELReasoner::new();
        let (cosa, receta, utensilio, hibrido) = (nid(), nid(), nid(), nid());
        for (id, l) in [(cosa, "Cosa"), (receta, "Receta"), (utensilio, "Utensilio"), (hibrido, "Hibrido")] {
            r.register_class(id, l);
        }
        r.assert_subclass(receta, cosa).unwrap();
        r.assert_subclass(utensilio, cosa).unwrap();
        r.assert_subclass(hibrido, receta).unwrap();
        r.assert_subclass(hibrido, utensilio).unwrap();
        r.assert_disjoint(&[receta, utensilio]).unwrap();
        (r, receta, utensilio, hibrido, cosa)
    }

    fn is_pair(cause: &Option<Unsatisfiable>, a: NodeId, b: NodeId) -> bool {
        matches!(cause, Some(Unsatisfiable::Disjoint { left, right })
            if (*left == a && *right == b) || (*left == b && *right == a))
    }

    #[test]
    fn test_disjoint_parents_make_the_class_unsatisfiable() {
        let (mut r, receta, utensilio, hibrido, cosa) = disjoint_fixture();
        assert_eq!(r.unsatisfiable_classes(), vec![hibrido]);
        assert!(!r.is_satisfiable(hibrido));
        assert!(r.is_satisfiable(receta) && r.is_satisfiable(utensilio) && r.is_satisfiable(cosa));
        assert!(r.is_subclass_of(hibrido, OWL_NOTHING));
        assert!(is_pair(&r.why_unsatisfiable(hibrido), receta, utensilio), "{:?}", r.why_unsatisfiable(hibrido));
        assert_eq!(r.why_unsatisfiable(receta), None);
        assert!(r.superclasses(hibrido).contains(&OWL_NOTHING), "⊥ shows up among the superclasses");
        assert!(!r.is_satisfiable(OWL_NOTHING), "⊥ itself is empty");
    }

    #[test]
    fn test_subclass_of_unsatisfiable_is_unsatisfiable() {
        let (mut r, receta, utensilio, hibrido, _) = disjoint_fixture();
        let nieto = nid();
        r.register_class(nieto, "Nieto");
        r.assert_subclass(nieto, hibrido).unwrap();
        let mut expected = vec![hibrido, nieto];
        expected.sort();
        assert_eq!(r.unsatisfiable_classes(), expected);
        assert!(is_pair(&r.why_unsatisfiable(nieto), receta, utensilio), "the inherited case names the pair too");
    }

    #[test]
    fn test_a_subsumption_derived_by_cr3_feeds_cr2() {
        // A ⊑ ∃r.B, ∃r.B ⊑ Receta ⇒ A ⊑ Receta (CR3, only derived);
        // A ⊑ Utensilio asserted; Receta, Utensilio disjoint ⇒ A ⊑ ⊥.
        // Until 0.6.17 CR2 only asked the asserted hierarchy and missed it.
        let mut r = ELReasoner::new();
        let (a, b, receta, utensilio) = (nid(), nid(), nid(), nid());
        for (id, l) in [(a, "A"), (b, "B"), (receta, "Receta"), (utensilio, "Utensilio")] {
            r.register_class(id, l);
        }
        r.assert_existential(a, "r", b).unwrap();
        r.assert_existential_domain("r", b, receta).unwrap();
        r.assert_subclass(a, utensilio).unwrap();
        r.assert_disjoint(&[receta, utensilio]).unwrap();
        assert_eq!(r.unsatisfiable_classes(), vec![a]);
    }

    #[test]
    fn test_existential_on_an_unsatisfiable_filler_propagates_bottom() {
        let (mut r, _, _, hibrido, _) = disjoint_fixture();
        let (plato, menu) = (nid(), nid());
        r.register_class(plato, "Plato");
        r.register_class(menu, "Menu");
        r.assert_existential(plato, "usa", hibrido).unwrap(); // Plato ⊑ ∃usa.Hibrido
        r.assert_subclass(menu, plato).unwrap(); // Menu ⊑ Plato inherits the restriction
        let unsat = r.unsatisfiable_classes();
        assert!(unsat.contains(&plato) && unsat.contains(&menu), "{unsat:?}");
        assert_eq!(
            r.why_unsatisfiable(plato),
            Some(Unsatisfiable::Existential { role: "usa".into(), filler: hibrido, rule: CompletionRule::CR4 })
        );
        let rules: HashSet<CompletionRule> = r.derived_inferences().iter().map(|i| i.rule).collect();
        assert!(rules.contains(&CompletionRule::CR2) && rules.contains(&CompletionRule::CR4), "{rules:?}");
    }

    #[test]
    fn test_existential_chain_through_bottom_needs_more_than_one_round() {
        // C ⊑ ⊥ (asserted), B ⊑ ∃r.C, A ⊑ ∃s.B, Y ⊑ A: CR4 twice, then CR1.
        let mut r = ELReasoner::new();
        let (a, b, c, y) = (nid(), nid(), nid(), nid());
        r.assert_subclass(c, OWL_NOTHING).unwrap();
        r.assert_existential(b, "r", c).unwrap();
        r.assert_existential(a, "s", b).unwrap();
        r.assert_subclass(y, a).unwrap();
        let mut expected = vec![a, b, c, y];
        expected.sort();
        assert_eq!(r.unsatisfiable_classes(), expected, "unregistered classes are classified too");
        assert_eq!(r.why_unsatisfiable(c), Some(Unsatisfiable::Asserted));
    }

    #[test]
    fn test_domain_into_bottom_is_reported_as_cr3() {
        // ∃r.B ⊑ ⊥ asserted as a domain: A ⊑ ∃r.B is unsatisfiable by CR3, and
        // the cause says CR3, the same rule `derived_inferences` reports.
        let mut r = ELReasoner::new();
        let (a, b) = (nid(), nid());
        r.assert_existential(a, "r", b).unwrap();
        r.assert_existential_domain("r", b, OWL_NOTHING).unwrap();
        assert_eq!(r.unsatisfiable_classes(), vec![a]);
        assert_eq!(
            r.why_unsatisfiable(a),
            Some(Unsatisfiable::Existential { role: "r".into(), filler: b, rule: CompletionRule::CR3 })
        );
        assert_eq!(r.derived.get(&(a, OWL_NOTHING)), Some(&CompletionRule::CR3));
    }

    #[test]
    fn test_incremental_disjoint_sees_unregistered_classes() {
        // x was never registered, only asserted below a and b: the
        // incremental CR2 of assert_disjoint still derives x ⊑ ⊥.
        let mut r = ELReasoner::new();
        let (x, a, b) = (nid(), nid(), nid());
        r.assert_subclass(x, a).unwrap();
        r.assert_subclass(x, b).unwrap();
        let inf = r.assert_disjoint(&[a, b]).unwrap();
        assert!(inf.iter().any(|i| i.axiom == Axiom::SubClassOf { sub: x, super_class: OWL_NOTHING }), "{inf:?}");
        assert!(r.is_subclass_of(x, OWL_NOTHING));
    }

    #[test]
    fn test_asserted_bottom_is_inherited_by_subclasses() {
        let mut r = ELReasoner::new();
        let (x, hijo) = (nid(), nid());
        r.assert_subclass(x, OWL_NOTHING).unwrap();
        r.assert_subclass(hijo, x).unwrap();
        assert_eq!(r.why_unsatisfiable(x), Some(Unsatisfiable::Asserted));
        assert_eq!(r.why_unsatisfiable(hijo), Some(Unsatisfiable::Inherited { parent: x }));
    }

    #[test]
    fn test_schema_without_conflicts_has_no_unsatisfiable_class() {
        let (mut r, ids) = chain(&["Cosa", "Receta", "Postre"]);
        let otra = nid();
        r.register_class(otra, "Utensilio");
        r.assert_subclass(otra, ids[0]).unwrap();
        r.assert_disjoint(&[ids[1], otra]).unwrap();
        assert!(r.unsatisfiable_classes().is_empty());
        for id in ids.iter().chain([&otra]) {
            assert!(r.is_satisfiable(*id));
            assert!(!r.superclasses(*id).contains(&OWL_NOTHING));
        }
        assert!(r.derived_inferences().iter().all(|i| i.axiom != Axiom::SubClassOf { sub: ids[2], super_class: OWL_NOTHING }));
    }

    #[test]
    fn test_all_disjoint_of_four_is_six_pairs() {
        let mut r = ELReasoner::new();
        let ids: Vec<NodeId> = (0..4).map(|_| nid()).collect();
        r.assert_disjoint(&ids).unwrap();
        assert_eq!(r.conjunctions.len(), 6);
        assert!(r.conjunctions.iter().all(|&(_, _, res)| res == OWL_NOTHING));
        // Asserting a pair again, in either order, adds nothing.
        r.assert_disjoint(&[ids[3], ids[0]]).unwrap();
        assert_eq!(r.conjunctions.len(), 6);
        // A class below the 2nd and the 4th is unsatisfiable.
        let x = nid();
        r.assert_subclass(x, ids[1]).unwrap();
        r.assert_subclass(x, ids[3]).unwrap();
        assert_eq!(r.unsatisfiable_classes(), vec![x]);
        assert!(is_pair(&r.why_unsatisfiable(x), ids[1], ids[3]));
    }

    #[test]
    fn test_assert_disjoint_and_bottom_errors() {
        let mut r = ELReasoner::new();
        let (a, b) = (nid(), nid());
        assert!(r.assert_disjoint(&[a]).is_err(), "one class");
        assert!(r.assert_disjoint(&[a, b, a]).is_err(), "repeated class");
        assert!(r.assert_disjoint(&[a, OWL_NOTHING]).is_err(), "⊥ is already disjoint with all");
        assert!(r.assert_subclass(OWL_NOTHING, a).is_err(), "⊥ cannot be the subclass");
        assert_eq!(ELReasoner::nothing(), OWL_NOTHING);
        assert!(r.assert_axiom(Axiom::DisjointClasses { classes: vec![a, b] }).is_ok());
        assert_eq!(r.conjunctions.len(), 1);
    }

    #[test]
    fn test_classification_does_not_depend_on_assertion_order() {
        let ids: Vec<NodeId> = (0..8).map(|_| nid()).collect();
        let axioms = vec![
            Axiom::SubClassOf { sub: ids[1], super_class: ids[0] },
            Axiom::SubClassOf { sub: ids[2], super_class: ids[0] },
            Axiom::SubClassOf { sub: ids[3], super_class: ids[1] },
            Axiom::SubClassOf { sub: ids[3], super_class: ids[2] },
            Axiom::DisjointClasses { classes: vec![ids[1], ids[2]] },
            Axiom::ExistentialRestriction { sub: ids[4], role: "r".into(), filler: ids[3] },
            Axiom::ExistentialRestriction { sub: ids[5], role: "r".into(), filler: ids[6] },
            Axiom::ExistentialDomain { role: "r".into(), filler: ids[6], result: ids[1] },
            Axiom::SubClassOf { sub: ids[5], super_class: ids[2] },
            Axiom::SubClassOf { sub: ids[7], super_class: ids[4] },
        ];
        let outcome = |order: &[Axiom]| {
            let mut r = ELReasoner::new();
            for ax in order {
                r.assert_axiom(ax.clone()).unwrap();
            }
            r.classify_all();
            let mut pairs: Vec<(NodeId, NodeId)> = r.derived.keys().copied().collect();
            pairs.sort();
            (r.unsatisfiable_classes(), pairs)
        };
        let reference = outcome(&axioms);
        let mut expected_unsat = vec![ids[3], ids[4], ids[5], ids[7]];
        expected_unsat.sort();
        assert_eq!(reference.0, expected_unsat);
        for shift in 0..axioms.len() {
            let mut order = axioms.clone();
            order.rotate_left(shift);
            assert_eq!(outcome(&order), reference, "rotation {shift}");
            order.reverse();
            assert_eq!(outcome(&order), reference, "reversed rotation {shift}");
        }
    }

    #[test]
    fn test_unsatisfiability_at_scale() {
        // A chain of 1 000 classes under a class that is below two disjoint
        // ones: every class of the chain is unsatisfiable. Tiny graphs hid a
        // broken algorithm before (Leiden); this one runs in debug too.
        let mut r = ELReasoner::new();
        let (left, right, top) = (nid(), nid(), nid());
        r.assert_subclass(top, left).unwrap();
        r.assert_subclass(top, right).unwrap();
        r.assert_disjoint(&[left, right]).unwrap();
        let mut prev = top;
        let mut chain_ids = vec![top];
        for _ in 0..1_000 {
            let id = nid();
            r.assert_subclass(id, prev).unwrap();
            chain_ids.push(id);
            prev = id;
        }
        let t = std::time::Instant::now();
        let unsat = r.unsatisfiable_classes();
        let elapsed = t.elapsed();
        chain_ids.sort();
        assert_eq!(unsat, chain_ids);
        assert!(r.is_satisfiable(left) && r.is_satisfiable(right));
        assert!(elapsed < std::time::Duration::from_secs(60), "classification took {elapsed:?}");
        // Idempotent: a second call does not recompute.
        assert!(r.classify_all().is_empty());
    }
}
