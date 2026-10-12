//! The universe a retrieval runs over (#214).
//!
//! A [`CandidateSet`] is **not another filter**: it is the definition of the
//! universe the ranking is computed in. Two invariants follow, and every
//! consumer in this crate holds them:
//!
//! 1. **It changes the scorable universe, never the score.** A search over a
//!    set is `top-k(Q, C)`, not `top-k(Q, D) ∩ C`: BM25 scores and cosine
//!    distances are the ones the same document gets without a set (the text
//!    branch scores the set in tantivy with the index-wide IDF, #213; the
//!    vector branch ranks the set's vectors exactly, 0.5.5).
//! 2. **It is never widened.** A set given by the caller is only kept or
//!    reduced by a `{label, props}` filter: every result is a member of the
//!    caller's set. That is what makes it usable as an ACL.
//!
//! The representation of an explicit set is private ([`CandidateIds`]): a
//! `HashSet<NodeId>` today, which a bitmap or a sorted vector could replace
//! without touching any consumer. Neither is done now — the sets callers pass
//! are small by nature (a document, an ACL, a tenant ∩ case: median 37 and
//! p95 102 nodes in the one measured deployment) — and a bigger
//! representation would only be worth it with a measurement asking for it.

use std::collections::HashSet;

use crate::types::NodeId;

/// The universe a retrieval runs over. See the module docs for the two
/// invariants.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CandidateSet {
    /// The whole graph, or whatever the query's filter leaves of it.
    #[default]
    All,
    /// An explicit set from the caller: a document's chunks, an ACL, a
    /// snapshot. Ids that do not exist simply never score.
    Ids(CandidateIds),
}

/// An explicit candidate set with a private representation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CandidateIds(HashSet<NodeId>);

impl CandidateIds {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, id: &NodeId) -> bool {
        self.0.contains(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodeId> {
        self.0.iter()
    }

    /// The ids as a set, for the paths that score a set directly. Crate-only
    /// so the representation stays an implementation detail of this module;
    /// gated like its single consumer (`graph::hybrid`) so a build without
    /// the vector index does not see dead code.
    #[cfg(feature = "embeddings-index")]
    pub(crate) fn to_hash_set(&self) -> HashSet<NodeId> {
        self.0.clone()
    }
}

impl From<HashSet<NodeId>> for CandidateSet {
    fn from(ids: HashSet<NodeId>) -> Self {
        CandidateSet::Ids(CandidateIds(ids))
    }
}

impl FromIterator<NodeId> for CandidateSet {
    fn from_iter<I: IntoIterator<Item = NodeId>>(iter: I) -> Self {
        CandidateSet::Ids(CandidateIds(iter.into_iter().collect()))
    }
}

impl CandidateSet {
    /// An explicit set from any collection of ids.
    pub fn ids<I: IntoIterator<Item = NodeId>>(ids: I) -> Self {
        ids.into_iter().collect()
    }

    pub fn is_all(&self) -> bool {
        matches!(self, CandidateSet::All)
    }

    /// `None` for [`CandidateSet::All`]: the whole graph has no size the set
    /// knows about.
    pub fn len(&self) -> Option<usize> {
        match self {
            CandidateSet::All => None,
            CandidateSet::Ids(ids) => Some(ids.len()),
        }
    }

    /// `true` only for an explicit set with no ids: nothing can score.
    pub fn is_empty(&self) -> bool {
        matches!(self, CandidateSet::Ids(ids) if ids.is_empty())
    }

    /// Whether `id` may score. `All` admits everything.
    pub fn contains(&self, id: &NodeId) -> bool {
        match self {
            CandidateSet::All => true,
            CandidateSet::Ids(ids) => ids.contains(id),
        }
    }

    /// `self ∩ other`. `All` is the identity, an empty set absorbs, two
    /// explicit sets intersect. This is the only operation a planner needs
    /// to combine the caller's set with whatever a filter produces, and it
    /// can only ever keep or shrink a set (invariant 2).
    pub fn intersect(self, other: CandidateSet) -> CandidateSet {
        match (self, other) {
            (CandidateSet::All, x) | (x, CandidateSet::All) => x,
            (CandidateSet::Ids(a), CandidateSet::Ids(b)) => {
                let (small, big) = if a.len() <= b.len() { (a, b) } else { (b, a) };
                CandidateSet::Ids(CandidateIds(small.0.into_iter().filter(|id| big.0.contains(id)).collect()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: &[u128]) -> CandidateSet {
        CandidateSet::ids(n.iter().map(|&x| NodeId::from_u128(x)))
    }

    #[test]
    fn all_is_the_identity_of_intersection() {
        let x = ids(&[1, 2, 3]);
        assert_eq!(CandidateSet::All.intersect(x.clone()), x);
        assert_eq!(x.clone().intersect(CandidateSet::All), x);
        assert_eq!(CandidateSet::All.intersect(CandidateSet::All), CandidateSet::All);
    }

    #[test]
    fn an_empty_set_absorbs() {
        let empty = ids(&[]);
        assert_eq!(empty.clone().intersect(ids(&[1, 2])), empty);
        assert_eq!(ids(&[1, 2]).intersect(empty.clone()), empty);
        assert!(empty.is_empty());
        assert!(!CandidateSet::All.is_empty(), "All is not empty: it is unbounded");
    }

    #[test]
    fn two_sets_intersect_commutatively_and_never_grow() {
        let a = ids(&[1, 2, 3, 4]);
        let b = ids(&[3, 4, 5]);
        let ab = a.clone().intersect(b.clone());
        assert_eq!(ab, ids(&[3, 4]));
        assert_eq!(b.intersect(a.clone()), ab);
        assert!(ab.len().unwrap() <= a.len().unwrap());
        for x in [1u128, 2, 3, 4, 5] {
            let id = NodeId::from_u128(x);
            assert_eq!(ab.contains(&id), a.contains(&id) && ids(&[3, 4, 5]).contains(&id));
        }
    }

    #[test]
    fn all_admits_everything_and_has_no_len() {
        assert!(CandidateSet::All.contains(&NodeId::from_u128(99)));
        assert_eq!(CandidateSet::All.len(), None);
        assert_eq!(ids(&[7]).len(), Some(1));
        assert!(CandidateSet::default().is_all());
    }
}
