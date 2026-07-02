// examples/benchmark_community_dual.rs
//
// Benchmark dual mode for NQL community aggregation:
// - community(n): exact global (Louvain), cache-enabled.
// - community_fast(n): approximate local.
//
// Reports p50/p95 latency (ms) for:
// 1) first exact run (cold)
// 2) repeated exact run (cache hit)
// 3) repeated fast run

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    println!("==========================================================");
    println!("NopalDB community() vs community_fast() benchmark");
    println!("==========================================================");

    let scenarios = [
        ("small", 300usize, 2usize),
        ("medium", 1_200usize, 4usize),
        ("large", 4_000usize, 6usize),
    ];

    for (name, node_count, span) in scenarios {
        println!();
        println!("Scenario: {name} | nodes={node_count} | local links={span}");

        let graph = Graph::in_memory().await?;
        seed_graph(&graph, node_count, span).await?;

        // Warm parser path minimally.
        let _ = graph.execute_nql("find count(n) as total from (n)").await?;

        let exact_cold_ms = run_once_ms(
            &graph,
            "find community(n) as cluster from (n) limit 1",
        )
        .await?;

        let exact_cached_ms = run_n_times_ms(
            &graph,
            "find community(n) as cluster from (n) limit 1",
            12,
        )
        .await?;

        let fast_ms = run_n_times_ms(
            &graph,
            "find community_fast(n) as cluster_fast from (n) limit 1",
            12,
        )
        .await?;

        print_stats("community exact (cold)", &[exact_cold_ms]);
        print_stats("community exact (cached)", &exact_cached_ms);
        print_stats("community_fast", &fast_ms);
    }

    println!();
    println!("Done.");
    Ok(())
}

async fn seed_graph(graph: &Graph, node_count: usize, span: usize) -> Result<()> {
    let mut tx = graph.begin_transaction().await?;
    let mut ids = Vec::with_capacity(node_count);

    for i in 0..node_count {
        let node = Node::new("Entity")
            .with_property("group", PropertyValue::String((i % 8).to_string()));
        ids.push(node.id);
        tx.add_node(node).await?;
    }

    // Ring backbone + local neighborhood edges to create community-like structure.
    for i in 0..node_count {
        let source = ids[i];
        let target_ring = ids[(i + 1) % node_count];
        tx.add_edge(Edge::new(source, target_ring, "LINK"))?;

        for hop in 2..=span {
            let target = ids[(i + hop) % node_count];
            tx.add_edge(Edge::new(source, target, "LINK"))?;
        }
    }

    tx.commit().await?;
    Ok(())
}

async fn run_once_ms(graph: &Graph, query: &str) -> Result<f64> {
    let start = Instant::now();
    let _ = graph.execute_nql(query).await?;
    Ok(start.elapsed().as_secs_f64() * 1_000.0)
}

async fn run_n_times_ms(graph: &Graph, query: &str, runs: usize) -> Result<Vec<f64>> {
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        times.push(run_once_ms(graph, query).await?);
    }
    Ok(times)
}

fn print_stats(label: &str, samples: &[f64]) {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);

    let p50 = percentile(&sorted, 0.50);
    let p95 = percentile(&sorted, 0.95);
    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;

    println!(
        "  {:26} | runs={:<2} avg={:>8.2} ms p50={:>8.2} ms p95={:>8.2} ms",
        label,
        sorted.len(),
        avg,
        p50,
        p95
    );
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
