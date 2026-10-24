//! The logical definition of a candidate universe, and its resolution to a
//! [`CandidateSet`] (#214 phase 2b).
//!
//! A [`CandidateSet`](crate::CandidateSet) is a *materialized* universe: the ids a search may
//! rank. Many of the universes a GraphRAG query wants are defined through
//! the graph instead: "the members of this community", "the chunks that
//! mention a member of this community", "the neighbours of these seeds".
//! [`Universe`] says *what* the universe is; `Graph::resolve_ids` turns it
//! into ids, choosing the cheap way to read them. The search
//! planner never has to know where a set came from. This is the "seed
//! constraint" of `docs/NQL_SEMANTICS.md` § 2, in a form that later NQL
//! syntax (`in_community`, several patterns in the FROM) compiles to.
//!
//! Deliberately not here (yet):
//! - **Label and property filters.** The search planner already resolves
//!   them without materializing (checked path, bounded probe, or set;
//!   `graph::hybrid`). Materializing them here would make a broad filter
//!   ×200 slower, as measured in phase 2a.
//! - **Union and difference.** No syntax asks for them yet. They are added
//!   when one does, not before.
//! - **Explicit ids and intersections.** They come with their first
//!   consumer (NQL `in_community` intersected with `c.id in […]`).

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use crate::error::Result;
use crate::types::NodeId;

use super::communities::IN_COMMUNITY;
use super::{Direction, Graph};

/// What a candidate universe is, independently of how it is read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Universe {
    /// The members of a materialized community (`IN_COMMUNITY` edges into
    /// the `Community` node with that `key`).
    Community(String),
    /// The nodes one edge away from `of`, through edges of `edge_type` (any
    /// type with `None`), in `direction` *as seen from `of`*. With
    /// [`Direction::Incoming`], the nodes that have an edge pointing at a
    /// member of `of`: the chunks that `MENTIONS` an entity.
    Neighbors {
        of: Box<Universe>,
        edge_type: Option<String>,
        direction: Direction,
    },
}

impl Graph {
    /// The ids a [`Universe`] denotes.
    ///
    /// Every step reads only what it needs: a community reads the incoming
    /// edges of its `Community` node; a neighbourhood reads the adjacency of
    /// the ids it starts from. Nothing scans the whole edge set.
    fn resolve_ids<'a>(
        &'a self,
        universe: &'a Universe,
    ) -> Pin<Box<dyn Future<Output = Result<HashSet<NodeId>>> + Send + 'a>> {
        Box::pin(async move {
            match universe {
                Universe::Community(key) => {
                    let community = self.community_by_key(key).await?;
                    Ok(self
                        .get_incoming_edges(community.id)
                        .await?
                        .into_iter()
                        .filter(|e| e.edge_type == IN_COMMUNITY)
                        .map(|e| e.source)
                        .collect())
                }
                Universe::Neighbors { of, edge_type, direction } => {
                    let start = self.resolve_ids(of).await?;
                    let mut out = HashSet::new();
                    for id in start {
                        let edge_ids = self.adjacent_edge_ids(id, *direction, None).await;
                        for edge in self.get_edges(&edge_ids).await?.into_iter().flatten() {
                            if edge_type.as_ref().is_some_and(|t| *t != edge.edge_type) {
                                continue;
                            }
                            out.insert(if edge.source == id { edge.target } else { edge.source });
                        }
                    }
                    Ok(out)
                }
            }
        })
    }

    /// The members of the materialized community `key` (see
    /// [`Self::materialize_communities`]), sorted by id. The key carries its
    /// partition and level (`"leiden/L0/…"`), so it is unique. An unknown
    /// key is an error.
    ///
    /// Reads only the incoming edges of that community's node.
    pub async fn community_members(&self, key: &str) -> Result<Vec<NodeId>> {
        self.sorted(Universe::Community(key.to_string())).await
    }

    /// The nodes that have a `via` edge pointing at a member of the
    /// community `key`, sorted by id: with `via = "MENTIONS"`, the chunks
    /// that mention one of its entities. Communities are usually computed on
    /// entities while searches run on chunks; this is the hop between them,
    /// and the result is a ready universe for
    /// [`HybridQuery::candidates`](super::hybrid::HybridQuery).
    pub async fn community_members_via(&self, key: &str, via: &str) -> Result<Vec<NodeId>> {
        self.sorted(Universe::Neighbors {
            of: Box::new(Universe::Community(key.to_string())),
            edge_type: Some(via.to_string()),
            direction: Direction::Incoming,
        })
        .await
    }

    async fn sorted(&self, universe: Universe) -> Result<Vec<NodeId>> {
        let mut ids: Vec<NodeId> = self.resolve_ids(&universe).await?.into_iter().collect();
        ids.sort();
        Ok(ids)
    }
}
