// Copied from hnsw_rs 0.3.4 (Jean-Pierre Both; MIT OR Apache-2.0, see LICENSE-MIT
// and LICENSE-APACHE next to this file). Changes for nopaldb are marked
// "nopaldb (#288)"; see mod.rs.

//! defines a trait for filtering requests.  
//! See examples in tests/filtertest.rs

use super::hnsw::DataId;

/// Only queries returning true are taken into account along the search
pub trait FilterT {
    fn hnsw_filter(&self, id: &DataId) -> bool;
}

impl FilterT for Vec<usize> {
    fn hnsw_filter(&self, id: &DataId) -> bool {
        self.binary_search(id).is_ok()
    }
}

impl<F> FilterT for F
where
    F: Fn(&DataId) -> bool,
{
    fn hnsw_filter(&self, id: &DataId) -> bool {
        self(id)
    }
}
