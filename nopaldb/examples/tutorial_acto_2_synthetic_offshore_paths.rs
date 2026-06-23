// Tutorial NopalDB - Acto 2: Synthetic Offshore Network (sintetico)
//
// Acompana a docs/tutorial/acto_2_synthetic_offshore/README.md.
//
// Este ejemplo cubre los pasos 1, 2, 4 y 5 del Acto 2 (los que NO requieren
// embeddings). Los pasos 3 y 6 (HNSW + PathAnomaly) viven en el notebook
// Python porque el binding actual de NopalDB para Rust no expone una API
// idiomatica para cargar embeddings desde sentence-transformers (eso
// pertenece naturalmente al ecosistema Python). El ejemplo Rust se enfoca
// en queries puramente estructurales: schema, indices, paths, reducers.
//
// Uso:
//   # Generar la DB:
//   python3 tutorials/shared/synthetic_offshore_dataset.py \
//     --db test_dbs/synthetic_offshore.db --reset
//
//   # Ejecutar:
//   cargo run --example tutorial_acto_2_synthetic_offshore_paths -- test_dbs/synthetic_offshore.db
//
// Gate: top-1 de path_sum debe ser "Pinnacle International #038" (cadena
// de 3 hops desde el flagship con flujo agregado de ~11.48M).

use nopaldb::Graph;
use std::error::Error;

const Q_SCHEMA: &str =
    include_str!("../../docs/tutorial/acto_2_synthetic_offshore/queries/01_schema_discovery.nql");
const Q_INDEX: &str =
    include_str!("../../docs/tutorial/acto_2_synthetic_offshore/queries/02_indices.nql");
const Q_PATHS: &str =
    include_str!("../../docs/tutorial/acto_2_synthetic_offshore/queries/04_paths_quantifier.nql");
const Q_PATH_SUM: &str =
    include_str!("../../docs/tutorial/acto_2_synthetic_offshore/queries/05_path_minivm.nql");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test_dbs/synthetic_offshore.db".to_string());

    println!("=== Tutorial NopalDB - Acto 2: Synthetic Offshore Network ===");
    println!("DB: {}", db_path);
    println!();

    let graph = Graph::open(&db_path).await?;

    paso_1_schema(&graph).await?;
    paso_2_indice(&graph).await?;
    paso_4_paths(&graph).await?;
    let top1_destino = paso_5_path_sum(&graph).await?;
    gate(&top1_destino)?;

    graph.close().await?;
    println!("\n=== Acto 2 (estructural) completado ===");
    println!("Para HNSW + PathAnomaly, ejecuta el notebook 02_synthetic_offshore.ipynb.");
    Ok(())
}

async fn paso_1_schema(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 1: schema discovery ---");
    let result = graph.execute_nql(Q_SCHEMA).await?;
    for row in &result.rows {
        let label = row
            .values
            .get("n.label")
            .map(prop_to_string)
            .unwrap_or_default();
        let total = row.values.get("total").map(prop_to_i64).unwrap_or(-1);
        println!("  {:<18} {}", label, total);
    }
    println!();
    Ok(())
}

async fn paso_2_indice(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 2: lookup con hash index ---");
    let result = graph.execute_nql(Q_INDEX).await?;
    println!("matches: {}", result.len());
    for row in &result.rows {
        let name = row
            .values
            .get("e.name")
            .map(prop_to_string)
            .unwrap_or_default();
        let industry = row
            .values
            .get("e.industry")
            .map(prop_to_string)
            .unwrap_or_default();
        let inc = row
            .values
            .get("e.incorporated")
            .map(prop_to_string)
            .unwrap_or_default();
        println!("  {} ({}, incorporated {})", name, industry, inc);
    }
    println!();
    Ok(())
}

async fn paso_4_paths(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 4: path queries cuantificadas (1..=3 hops) ---");
    let result = graph.execute_nql(Q_PATHS).await?;
    println!("paths encontrados: {}", result.len());
    let mut by_depth: std::collections::BTreeMap<i64, usize> = Default::default();
    for row in &result.rows {
        let depth = row.values.get("hops").map(prop_to_i64).unwrap_or(-1);
        *by_depth.entry(depth).or_insert(0) += 1;
    }
    for (d, c) in by_depth {
        println!("  hops={}: {} paths", d, c);
    }
    println!();
    Ok(())
}

/// Paso 5: top-1 destino por path_sum. Devuelve el nombre del destino #1
/// para el gate cruzado.
async fn paso_5_path_sum(graph: &Graph) -> Result<String, Box<dyn Error>> {
    println!("--- Paso 5: path_sum reducer ---");
    let result = graph.execute_nql(Q_PATH_SUM).await?;
    println!("Top-3 destinos por flujo agregado:");
    let mut top1 = String::new();
    for (i, row) in result.rows.iter().take(3).enumerate() {
        let destino = row
            .values
            .get("destino")
            .map(prop_to_string)
            .unwrap_or_default();
        let hops = row.values.get("hops").map(prop_to_i64).unwrap_or(-1);
        let flujo = row
            .values
            .get("flujo_total")
            .map(prop_to_f64)
            .unwrap_or(0.0);
        println!(
            "  {} {}  ({} hops, flujo {:.2})",
            i + 1,
            destino,
            hops,
            flujo
        );
        if i == 0 {
            top1 = destino;
        }
    }
    println!();
    Ok(top1)
}

fn gate(top1_destino: &str) -> Result<(), Box<dyn Error>> {
    println!("--- Gate Acto 2: top-1 path_sum ---");
    println!("Top-1 destino: {:?}", top1_destino);
    let expected = "Pinnacle International #038";
    if top1_destino != expected {
        return Err(format!(
            "Gate fallido: esperaba {:?}, obtuve {:?}",
            expected, top1_destino
        )
        .into());
    }
    println!("GATE OK - cadena CONTROLS de mayor flujo identificada.");
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
