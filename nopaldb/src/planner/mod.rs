// ==========================================
// src/planner/mod.rs
// ==========================================

//! Query Planner - Cost-based optimizer for NopalDB
//!
//! The planner analyzes queries and chooses the optimal execution strategy
//! based on graph statistics and available indexes.

use std::collections::HashMap;

/// Graph statistics for cost estimation
#[derive(Debug, Clone, Default)]
pub struct GraphStats {
    /// Total number of nodes in the graph
    pub total_nodes: usize,

    /// Total number of edges in the graph
    pub total_edges: usize,

    /// Number of nodes per label
    pub nodes_per_label: HashMap<String, usize>,

    /// Number of edges per type
    pub edges_per_type: HashMap<String, usize>,

    /// Average degree (edges per node)
    pub avg_degree: f64,

    /// Property cardinality (unique values per property)
    pub property_cardinality: HashMap<String, usize>,
}

impl GraphStats {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Execution plan node
#[derive(Debug, Clone)]
pub enum PlanNode {
    /// Full scan of all nodes with a label
    LabelScan {
        label: String,
        estimated_rows: usize,
        cost: f64,
    },

    /// Index seek using hash or btree index
    IndexSeek {
        index_name: String,
        label: String,
        property: String,
        estimated_rows: usize,
        cost: f64,
    },

    /// Filter operation
    Filter {
        predicate: String,
        input: Box<PlanNode>,
        selectivity: f64,
        estimated_rows: usize,
        cost: f64,
    },

    /// Expand edges
    Expand {
        edge_type: String,
        direction: String,
        input: Box<PlanNode>,
        estimated_rows: usize,
        cost: f64,
    },
}

impl PlanNode {
    pub fn estimated_rows(&self) -> usize {
        match self {
            PlanNode::LabelScan { estimated_rows, .. } => *estimated_rows,
            PlanNode::IndexSeek { estimated_rows, .. } => *estimated_rows,
            PlanNode::Filter { estimated_rows, .. } => *estimated_rows,
            PlanNode::Expand { estimated_rows, .. } => *estimated_rows,
        }
    }

    pub fn cost(&self) -> f64 {
        match self {
            PlanNode::LabelScan { cost, .. } => *cost,
            PlanNode::IndexSeek { cost, .. } => *cost,
            PlanNode::Filter { cost, .. } => *cost,
            PlanNode::Expand { cost, .. } => *cost,
        }
    }
}

/// Cost constants (calibrated for typical workloads)
const SCAN_COST_PER_NODE: f64 = 1.0;
const INDEX_SEEK_BASE_COST: f64 = 10.0;
const INDEX_SEEK_PER_ROW: f64 = 0.1;
const FILTER_COST_PER_ROW: f64 = 0.1;
const _EXPAND_COST_PER_EDGE: f64 = 0.5;

/// Query Planner - Cost-based optimizer
pub struct QueryPlanner {
    stats: GraphStats,
}

impl QueryPlanner {
    /// Create a new query planner with the given statistics
    pub fn new(stats: GraphStats) -> Self {
        Self { stats }
    }

    /// Estimate cost of a label scan
    pub fn estimate_label_scan(&self, label: &str) -> (usize, f64) {
        let estimated_rows = self.stats.nodes_per_label
            .get(label)
            .copied()
            .unwrap_or(self.stats.total_nodes);

        let cost = estimated_rows as f64 * SCAN_COST_PER_NODE;

        (estimated_rows, cost)
    }

    /// Estimate cost of an index seek
    pub fn estimate_index_seek(&self, label: &str, property: &str) -> (usize, f64) {
        let key = format!("{}_{}", label, property);
        let cardinality = self.stats.property_cardinality
            .get(&key)
            .copied()
            .unwrap_or(1);

        let total_nodes = self.stats.nodes_per_label
            .get(label)
            .copied()
            .unwrap_or(self.stats.total_nodes);

        // For equality, estimate 1 row per unique value
        let estimated_rows = total_nodes.checked_div(cardinality).unwrap_or(1);

        let cost = INDEX_SEEK_BASE_COST + (estimated_rows as f64 * INDEX_SEEK_PER_ROW);

        (estimated_rows, cost)
    }

    /// Choose best plan: IndexSeek vs LabelScan
    pub fn choose_best_plan(
        &self,
        label: &str,
        property: Option<&str>,
        has_index: bool,
    ) -> PlanNode {
        // Try index seek if available
        if let Some(prop) = property {
            if has_index {
                let (index_rows, index_cost) = self.estimate_index_seek(label, prop);
                let (_scan_rows, scan_cost) = self.estimate_label_scan(label);

                if index_cost < scan_cost {
                    log::info!(
                        "🚀 Planner: Using INDEX SEEK (cost: {:.2} vs {:.2})",
                        index_cost, scan_cost
                    );

                    return PlanNode::IndexSeek {
                        index_name: format!("{}_{}", label, prop),
                        label: label.to_string(),
                        property: prop.to_string(),
                        estimated_rows: index_rows,
                        cost: index_cost,
                    };
                }

                log::info!(
                    "⚠️  Planner: INDEX exists but SCAN is faster (index: {:.2}, scan: {:.2})",
                    index_cost, scan_cost
                );
            } else {
                log::warn!("⚠️  Planner: No index on {}.{}, using SCAN", label, prop);
            }
        }

        // Fallback to label scan
        let (rows, cost) = self.estimate_label_scan(label);
        log::info!("📊 Planner: Using LABEL SCAN (cost: {:.2})", cost);

        PlanNode::LabelScan {
            label: label.to_string(),
            estimated_rows: rows,
            cost,
        }
    }

    /// Add filter to existing plan
    pub fn add_filter(
        &self,
        input: PlanNode,
        selectivity: f64,
        predicate: String,
    ) -> PlanNode {
        let input_rows = input.estimated_rows();
        let estimated_rows = (input_rows as f64 * selectivity).max(1.0) as usize;
        let filter_cost = input_rows as f64 * FILTER_COST_PER_ROW;
        let total_cost = input.cost() + filter_cost;

        PlanNode::Filter {
            predicate,
            input: Box::new(input),
            selectivity,
            estimated_rows,
            cost: total_cost,
        }
    }

    /// Format plan as string (for EXPLAIN)
    pub fn format_plan(&self, plan: &PlanNode, indent: usize) -> String {
        let prefix = "│ ".repeat(indent);

        match plan {
            PlanNode::LabelScan { label, estimated_rows, cost } => {
                format!(
                    "{}→ LABEL SCAN: {} | rows=~{} | cost={:.2}",
                    prefix, label, estimated_rows, cost
                )
            }
            PlanNode::IndexSeek { index_name, estimated_rows, cost, .. } => {
                format!(
                    "{}→ INDEX SEEK: {} | rows=~{} | cost={:.2}",
                    prefix, index_name, estimated_rows, cost
                )
            }
            PlanNode::Filter { predicate, input, estimated_rows, cost, selectivity } => {
                let mut s = format!(
                    "{}→ FILTER: {} | selectivity={:.2} | rows=~{} | cost={:.2}\n",
                    prefix, predicate, selectivity, estimated_rows, cost
                );
                s.push_str(&self.format_plan(input, indent + 1));
                s
            }
            PlanNode::Expand { edge_type, direction, input, estimated_rows, cost } => {
                let mut s = format!(
                    "{}→ EXPAND: {} ({}) | rows=~{} | cost={:.2}\n",
                    prefix, edge_type, direction, estimated_rows, cost
                );
                s.push_str(&self.format_plan(input, indent + 1));
                s
            }
        }
    }

    /// Format plan in a fancy box (for EXPLAIN output)
    pub fn explain(&self, plan: &PlanNode) -> String {
        let plan_str = self.format_plan(plan, 0);
        let total_cost = plan.cost();
        let total_rows = plan.estimated_rows();

        format!(
            "┌─────────────────────────────────────────┐\n\
             │ Query Execution Plan                    │\n\
             ├─────────────────────────────────────────┤\n\
             {}\n\
             ├─────────────────────────────────────────┤\n\
             │ Total Cost: {:.2}                        │\n\
             │ Estimated Rows: ~{}                     │\n\
             │ Estimated Time: {:.2}ms                 │\n\
             └─────────────────────────────────────────┘",
            plan_str, total_cost, total_rows, total_cost / 1000.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planner_chooses_index_when_better() {
        let mut stats = GraphStats::new();
        stats.total_nodes = 1_000_000;
        stats.nodes_per_label.insert("Person".to_string(), 1_000_000);
        stats.property_cardinality.insert("Person_email".to_string(), 500_000);

        let planner = QueryPlanner::new(stats);

        // With index: should choose IndexSeek
        let plan = planner.choose_best_plan("Person", Some("email"), true);
        assert!(matches!(plan, PlanNode::IndexSeek { .. }));
        assert!(plan.cost() < 100.0); // Much cheaper than scan
    }

    #[test]
    fn test_planner_chooses_scan_without_index() {
        let mut stats = GraphStats::new();
        stats.total_nodes = 1000;
        stats.nodes_per_label.insert("Person".to_string(), 1000);

        let planner = QueryPlanner::new(stats);

        // Without index: should choose LabelScan
        let plan = planner.choose_best_plan("Person", Some("name"), false);
        assert!(matches!(plan, PlanNode::LabelScan { .. }));
    }

    #[test]
    fn test_filter_reduces_rows() {
        let stats = GraphStats::new();
        let planner = QueryPlanner::new(stats);

        let scan = PlanNode::LabelScan {
            label: "Person".to_string(),
            estimated_rows: 1000,
            cost: 1000.0,
        };

        // Filter with 10% selectivity
        let filtered = planner.add_filter(scan, 0.1, "age > 30".to_string());

        assert_eq!(filtered.estimated_rows(), 100); // 10% of 1000
        assert!(filtered.cost() > 1000.0); // Cost increased
    }
}