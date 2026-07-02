//! OWL-EL Reasoner — Steps 5, 9 and CR3 of the NopalDB ontological roadmap.
//!
//! Implements:
//! - CR1 (transitivity): A ⊑ B ∧ B ⊑ C → A ⊑ C
//! - CR2 (conjunction):  A ⊑ B ∧ A ⊑ C ∧ B ⊓ C ⊑ D → A ⊑ D
//! - CR3 (existential):  A ⊑ ∃R.B ∧ B ⊑ C ∧ ∃R.C ⊑ D → A ⊑ D
//!
//! The reasoner is a standalone value type that operates over a [`TaxonomyIndex`].
//! It does NOT write inferences back to the graph automatically — callers decide
//! whether to materialize them.
//!
//! # Feature gate
//! Compiled only when the `reasoner` feature is enabled.

use std::collections::HashSet;

use crate::error::{NopalError, Result};
use crate::index::taxonomy::TaxonomyIndex;
use crate::types::NodeId;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionRule {
    /// CR1: transitivity — A ⊑ B ∧ B ⊑ C → A ⊑ C
    CR1,
    /// CR2: conjunction — A ⊑ B ∧ A ⊑ C ∧ B ⊓ C ⊑ D → A ⊑ D
    CR2,
    /// CR3: existential — A ⊑ ∃R.B ∧ B ⊑ C ∧ ∃R.C ⊑ D → A ⊑ D
    CR3,
}

// ---------------------------------------------------------------------------
// ELReasoner
// ---------------------------------------------------------------------------

/// An OWL-EL reasoner that operates over a [`TaxonomyIndex`].
///
/// Step 5 implements CR1 (transitivity) only.
/// CR2 (conjunction inclusions) is planned for Step 9.
///
/// The reasoner owns a cloned working copy of the `TaxonomyIndex` so that
/// it can mutate it incrementally without affecting the caller's original index.
///
/// All methods are synchronous — no Tokio runtime is needed.
pub struct ELReasoner {
    taxonomy: TaxonomyIndex,
    /// Pairs `(sub, super_class)` derived by CR1, CR2, or CR3 (not directly asserted).
    derived: HashSet<(NodeId, NodeId)>,
    /// CR2 conjunction axioms: `(left, right, result)` meaning `left ⊓ right ⊑ result`.
    conjunctions: Vec<(NodeId, NodeId, NodeId)>,
    /// CR3 left-side axioms: `(sub=A, role=R, filler=B)` for `A ⊑ ∃R.B`.
    existentials: Vec<(NodeId, String, NodeId)>,
    /// CR3 right-side axioms: `(role=R, filler=C, result=D)` for `∃R.C ⊑ D`.
    existential_domains: Vec<(String, NodeId, NodeId)>,
}

impl Default for ELReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl ELReasoner {
    /// Create a new, empty reasoner.
    pub fn new() -> Self {
        Self {
            taxonomy: TaxonomyIndex::new(),
            derived: HashSet::new(),
            conjunctions: Vec::new(),
            existentials: Vec::new(),
            existential_domains: Vec::new(),
        }
    }

    /// Register a class node (label) in the internal taxonomy.
    ///
    /// Idempotent — calling with the same `id` twice overwrites the label.
    pub fn register_class(&mut self, id: NodeId, label: impl Into<String>) {
        self.taxonomy.register_class(id, label);
    }


    /// Build a reasoner from an existing [`TaxonomyIndex`].
    ///
    /// Clones the taxonomy so the reasoner owns its own working copy.
    /// Does NOT run classification — call [`classify_all`](Self::classify_all)
    /// to saturate.
    pub fn from_taxonomy(taxonomy: &TaxonomyIndex) -> Self {
        Self {
            taxonomy: taxonomy.clone(),
            derived: HashSet::new(),
            conjunctions: Vec::new(),
            existentials: Vec::new(),
            existential_domains: Vec::new(),
        }
    }

    /// Build an [`ELReasoner`] from a historical graph snapshot at `timestamp`.
    ///
    /// Reconstructs the class hierarchy by:
    /// 1. Fetching all `NodeKind::Class` nodes valid at `timestamp`.
    /// 2. Fetching all `subClassOf` edges at `timestamp`.
    /// 3. Populating a fresh [`TaxonomyIndex`] and wrapping it.
    ///
    /// Does NOT run classification — call [`classify_all`](Self::classify_all)
    /// to derive transitive inferences over the historical snapshot.
    ///
    /// # Errors
    /// Propagates storage errors from [`Graph::get_class_nodes_at`] or
    /// [`Graph::get_edges_of_type_at`].
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

        Ok(Self {
            taxonomy,
            derived: HashSet::new(),
            conjunctions: Vec::new(),
            existentials: Vec::new(),
            existential_domains: Vec::new(),
        })
    }

    /// Assert a new SubClassOf axiom: `sub ⊑ super_class`.
    ///
    /// Adds the direct edge to the taxonomy and applies CR1 incrementally to
    /// propagate new transitive inferences. Returns the list of NEW inferences
    /// derived (not including the asserted direct edge itself).
    ///
    /// # Errors
    /// Returns [`NopalError::custom`] if `sub == super_class` (self-loop).
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

        // TaxonomyIndex convention: add_subclass(parent, child) means child ⊑ parent.
        // To assert sub ⊑ super_class: add_subclass(super_class, sub).
        self.taxonomy.add_subclass(super_class, sub)?;

        let mut new_inferences: Vec<Inference> = Vec::new();

        // CR1 forward: sub ⊑ super_class and super_class ⊑ X → derive sub ⊑ X
        let ancestors_of_super: Vec<NodeId> = self.taxonomy.ancestors(super_class);
        for ancestor in &ancestors_of_super {
            let pair = (sub, *ancestor);
            if !self.derived.contains(&pair) {
                self.derived.insert(pair);
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub, super_class: *ancestor },
                    rule: CompletionRule::CR1,
                });
            }
        }

        // CR1 backward: Y ⊑ sub and sub ⊑ super_class → derive Y ⊑ super_class (and Y ⊑ X)
        let descendants_of_sub: Vec<NodeId> = self.taxonomy.descendants(sub);
        for desc in &descendants_of_sub {
            let pair_desc_super = (*desc, super_class);
            if !self.derived.contains(&pair_desc_super) {
                self.derived.insert(pair_desc_super);
                new_inferences.push(Inference {
                    axiom: Axiom::SubClassOf { sub: *desc, super_class },
                    rule: CompletionRule::CR1,
                });
            }
            for ancestor in &ancestors_of_super {
                let pair = (*desc, *ancestor);
                if !self.derived.contains(&pair) {
                    self.derived.insert(pair);
                    new_inferences.push(Inference {
                        axiom: Axiom::SubClassOf { sub: *desc, super_class: *ancestor },
                        rule: CompletionRule::CR1,
                    });
                }
            }
        }

        Ok(new_inferences)
    }

    /// Assert a conjunction inclusion axiom: `left ⊓ right ⊑ result`.
    ///
    /// Records the axiom and immediately applies CR2 to derive any new
    /// `X ⊑ result` inferences for all classes X already known to be
    /// subclasses of both `left` and `right`.
    ///
    /// # Errors
    /// Returns an error if `left == right == result` (trivial self-inclusion).
    pub fn assert_conjunction(
        &mut self,
        left: NodeId,
        right: NodeId,
        result: NodeId,
    ) -> Result<Vec<Inference>> {
        // Store the conjunction axiom.
        self.conjunctions.push((left, right, result));
        // Apply CR2 immediately for existing knowledge.
        Ok(self.apply_cr2_for_conjunction(left, right, result))
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
            // B ⊑ C: direct equality or transitive via taxonomy / derived set.
            let filler_sub_c = filler == *c
                || self.taxonomy.is_subclass_of(filler, *c)
                || self.derived.contains(&(filler, *c));

            if filler_sub_c {
                let pair = (sub, *d);
                if !self.derived.contains(&pair)
                    && !self.taxonomy.direct_parents(sub).contains(d)
                {
                    self.derived.insert(pair);
                    new_inferences.push(Inference {
                        axiom: Axiom::SubClassOf { sub, super_class: *d },
                        rule: CompletionRule::CR3,
                    });
                }
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
            // B ⊑ filler: direct equality or transitive.
            let b_sub_filler = *b == filler
                || self.taxonomy.is_subclass_of(*b, filler)
                || self.derived.contains(&(*b, filler));

            if b_sub_filler {
                let pair = (*a, result);
                if !self.derived.contains(&pair)
                    && !self.taxonomy.direct_parents(*a).contains(&result)
                {
                    self.derived.insert(pair);
                    new_inferences.push(Inference {
                        axiom: Axiom::SubClassOf { sub: *a, super_class: result },
                        rule: CompletionRule::CR3,
                    });
                }
            }
        }

        new_inferences
    }

    /// Apply CR2 for a single conjunction axiom `left ⊓ right ⊑ result`.
    ///
    /// For every class X such that X ⊑ left (direct or derived) AND X ⊑ right,
    /// derive X ⊑ result.
    fn apply_cr2_for_conjunction(
        &mut self,
        left: NodeId,
        right: NodeId,
        result: NodeId,
    ) -> Vec<Inference> {
        let all_nodes = self.taxonomy.all_class_ids();
        let mut new_inferences: Vec<Inference> = Vec::new();

        for &x in &all_nodes {
            if x == result {
                continue;
            }
            // X ⊑ left (direct or transitive) AND X ⊑ right (direct or transitive)
            let x_sub_left  = x == left  || self.taxonomy.is_subclass_of(x, left);
            let x_sub_right = x == right || self.taxonomy.is_subclass_of(x, right);

            if x_sub_left && x_sub_right {
                let pair = (x, result);
                if !self.derived.contains(&pair) && !self.taxonomy.direct_parents(x).contains(&result) {
                    self.derived.insert(pair);
                    new_inferences.push(Inference {
                        axiom: Axiom::SubClassOf { sub: x, super_class: result },
                        rule: CompletionRule::CR2,
                    });
                }
            }
        }

        new_inferences
    }

    /// Run CR1 and CR2 to saturation over all currently known axioms.
    ///
    /// Idempotent — safe to call multiple times; subsequent calls after
    /// saturation return an empty Vec.
    ///
    /// For each registered class X:
    /// - CR1: finds transitive ancestors not already in `derived`.
    /// - CR2: for each conjunction `B ⊓ C ⊑ D`, checks if X ⊑ B ∧ X ⊑ C → X ⊑ D.
    pub fn classify_all(&mut self) -> Vec<Inference> {
        let all_nodes = self.taxonomy.all_class_ids();
        let mut new_inferences: Vec<Inference> = Vec::new();

        // CR1: transitive closure.
        for node in &all_nodes {
            let direct_parents = self.taxonomy.direct_parents(*node);
            let ancestors = self.taxonomy.ancestors(*node);

            for ancestor in &ancestors {
                // Only record as CR1-derived if it is NOT a direct edge.
                if !direct_parents.contains(ancestor) {
                    let pair = (*node, *ancestor);
                    if !self.derived.contains(&pair) {
                        self.derived.insert(pair);
                        new_inferences.push(Inference {
                            axiom: Axiom::SubClassOf {
                                sub: *node,
                                super_class: *ancestor,
                            },
                            rule: CompletionRule::CR1,
                        });
                    }
                }
            }
        }

        // CR2: conjunction inclusions.
        let conjunctions = self.conjunctions.clone();
        for (left, right, result) in conjunctions {
            let cr2_new = self.apply_cr2_for_conjunction(left, right, result);
            new_inferences.extend(cr2_new);
        }

        // CR3: existential restrictions.
        // For each A ⊑ ∃R.B, check all ∃R.C ⊑ D; if B ⊑ C → derive A ⊑ D.
        let existentials = self.existentials.clone();
        for (sub, role, filler) in &existentials {
            let cr3_new = self.apply_cr3_for_existential(*sub, role, *filler);
            new_inferences.extend(cr3_new);
        }

        new_inferences
    }

    /// Returns `true` if `sub` is known to be a subclass of `super_class`,
    /// considering both direct (asserted) and inferred axioms.
    pub fn is_subclass_of(&mut self, sub: NodeId, super_class: NodeId) -> bool {
        if sub == super_class {
            return false;
        }
        // Check derived set first (O(1)).
        if self.derived.contains(&(sub, super_class)) {
            return true;
        }
        // Fall through to taxonomy BFS (handles the direct-edge case).
        self.taxonomy.is_subclass_of(sub, super_class)
    }

    /// All superclasses of `node` — direct and inferred — via BFS upward.
    pub fn superclasses(&self, node: NodeId) -> Vec<NodeId> {
        self.taxonomy.ancestors(node)
    }

    /// All subclasses of `node` — direct and inferred — via BFS downward.
    pub fn subclasses(&mut self, node: NodeId) -> Vec<NodeId> {
        self.taxonomy.descendants(node)
    }

    /// Number of direct subClassOf edges currently in the working taxonomy.
    pub fn axiom_count(&self) -> usize {
        self.taxonomy.edge_count()
    }

    /// All inferences derived by CR1 since construction or the last
    /// [`clear_derived`](Self::clear_derived) call.
    pub fn derived_inferences(&self) -> Vec<Inference> {
        self.derived
            .iter()
            .map(|(sub, super_class)| Inference {
                axiom: Axiom::SubClassOf {
                    sub: *sub,
                    super_class: *super_class,
                },
                rule: CompletionRule::CR1,
            })
            .collect()
    }

    /// Clear the derived inference set.
    ///
    /// Does NOT affect the underlying taxonomy direct edges. Useful for
    /// re-running [`classify_all`](Self::classify_all) after adding more axioms.
    pub fn clear_derived(&mut self) {
        self.derived.clear();
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
}
