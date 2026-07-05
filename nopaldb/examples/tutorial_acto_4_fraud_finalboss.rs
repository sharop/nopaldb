// Tutorial NopalDB - Acto 4: Synthetic Fraud (final boss)
//
// Acompana a docs/tutorial/acto_4_synthetic_fraud/README.md.
//
// Este ejemplo recorre el dataset sintetico (generado por el script
// Python tutorials/shared/synthetic_fraud_dataset.py) y verifica que las
// queries clave - topologia, top inbound transfers, communities, paths -
// retornan resultados consistentes con la planta del ring.
//
// Gate: las 5 cuentas con mas transfers inbound deben tener cada una
// >=14 inbound (vs <=4 de las cuentas no-ring). Este invariante es
// estable sobre seed=42.
//
// Uso:
//   python3 tutorials/shared/synthetic_fraud_dataset.py \
//     --db test_dbs/synthetic_fraud.db --reset
//   cargo run --example tutorial_acto_4_fraud_finalboss \
//     -- test_dbs/synthetic_fraud.db

use nopaldb::Graph;
use std::error::Error;

const Q_TOPOLOGY: &str = include_str!(
    "../../docs/tutorial/acto_4_synthetic_fraud/queries/01_topology.nql"
);
const Q_TOP_INBOUND: &str = include_str!(
    "../../docs/tutorial/acto_4_synthetic_fraud/queries/02_top_inbound.nql"
);
const Q_RING_TRANSFERS: &str = include_str!(
    "../../docs/tutorial/acto_4_synthetic_fraud/queries/03_ring_transfers.nql"
);
const Q_PATHS: &str = include_str!(
    "../../docs/tutorial/acto_4_synthetic_fraud/queries/05_path_chains.nql"
);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test_dbs/synthetic_fraud.db".to_string());

    println!("=== Tutorial NopalDB - Acto 4: Synthetic Fraud (final boss) ===");
    println!("DB: {}", db_path);
    println!();

    let graph = Graph::open(&db_path).await?;

    paso_1_topology(&graph).await?;
    let min_inbound = paso_2_top_inbound(&graph).await?;
    paso_3_ring_transfers(&graph).await?;
    paso_5_path_chains(&graph).await?;

    gate(min_inbound)?;

    graph.close().await?;
    println!("\n=== Acto 4 (estructural) completado ===");
    println!("Para community detection, embeddings, path-anomaly y Arrow export, ejecuta el notebook 04_synthetic_fraud.ipynb.");
    Ok(())
}

async fn paso_1_topology(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 1: schema (counts por label) ---");
    let result = graph.execute_nql(Q_TOPOLOGY).await?;
    for row in result.rows() {
        let label = row.get("etiqueta").map(prop_to_string).unwrap_or_default();
        let total = row.get("total").map(prop_to_i64).unwrap_or(-1);
        if !label.is_empty() {
            println!("  {:<18} {}", label, total);
        }
    }
    println!();
    Ok(())
}

/// Retorna el min inbound del top-5 cuentas con más transfers inbound (gate).
async fn paso_2_top_inbound(graph: &Graph) -> Result<i64, Box<dyn Error>> {
    println!("--- Paso 2: top-5 cuentas por transfers inbound ---");
    let result = graph.execute_nql(Q_TOP_INBOUND).await?;
    let mut min_inbound: i64 = i64::MAX;
    for row in result.rows().iter().take(5) {
        let id = row.get("b.id").map(prop_to_string).unwrap_or_default();
        let n = row.get("inbound").map(prop_to_i64).unwrap_or(0);
        println!("  {}  {}", &id[..8.min(id.len())], n);
        if n < min_inbound {
            min_inbound = n;
        }
    }
    println!();
    Ok(min_inbound)
}

async fn paso_3_ring_transfers(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 3: transfers de alto monto (>900k) ---");
    let result = graph.execute_nql(Q_RING_TRANSFERS).await?;
    let amounts: Vec<f64> = result.rows().iter()
        .filter_map(|r| r.get("e.amount").map(prop_to_f64))
        .collect();
    println!("transfers > 900k: {}", amounts.len());
    if !amounts.is_empty() {
        let min = amounts.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = amounts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let avg = amounts.iter().sum::<f64>() / amounts.len() as f64;
        println!("  amount min/max/avg: {:.0} / {:.0} / {:.0}", min, max, avg);
    }
    println!();
    Ok(())
}

async fn paso_5_path_chains(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 5: cadenas TRANSFERS (2-3 hops) por flujo total ---");
    let result = graph.execute_nql(Q_PATHS).await?;
    println!("paths encontrados: {}", result.len());
    for row in result.rows().iter().take(3) {
        let hops = row.get("hops").map(prop_to_i64).unwrap_or(-1);
        let flujo = row.get("flujo").map(prop_to_f64).unwrap_or(0.0);
        println!("  hops={} flujo={:.0}", hops, flujo);
    }
    println!();
    Ok(())
}

fn gate(min_inbound: i64) -> Result<(), Box<dyn Error>> {
    println!("--- Gate Acto 4 ---");
    println!("min inbound del top-5 = {} (esperado >= 14)", min_inbound);
    if min_inbound < 14 {
        return Err(format!(
            "Gate fallido: el top-5 inbound deberia tener >=14 transfers. Min real: {}",
            min_inbound
        )
        .into());
    }
    println!("GATE OK - el ring fraudulento se delata por densidad inbound.");
    Ok(())
}

fn prop_to_string(p: &nopaldb::types::PropertyValue) -> String {
    use nopaldb::types::PropertyValue::*;
    match p {
        String(s) => s.clone(),
        Int(n) => n.to_string(),
        Float(f) => f.to_string(),
        Bool(b) => b.to_string(),
        _ => format!("{:?}", p),
    }
}

fn prop_to_f64(p: &nopaldb::types::PropertyValue) -> f64 {
    use nopaldb::types::PropertyValue::*;
    match p {
        Float(f) => *f,
        Int(n) => *n as f64,
        _ => 0.0,
    }
}

fn prop_to_i64(p: &nopaldb::types::PropertyValue) -> i64 {
    use nopaldb::types::PropertyValue::*;
    match p {
        Int(n) => *n,
        Float(f) => *f as i64,
        _ => -1,
    }
}
