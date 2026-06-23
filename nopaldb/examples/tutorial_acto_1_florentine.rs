// Tutorial NopalDB — Acto 1: Florentine Families
//
// Acompaña a docs/tutorial/acto_1_florentine/README.md.
//
// Las queries NQL se cargan desde docs/tutorial/acto_1_florentine/queries/*.nql
// vía `include_str!`. Esto garantiza que el ejemplo Rust, el notebook Python
// y la documentación Markdown ejecutan exactamente la misma query — si una
// se actualiza, el archivo .nql cambia y los tres medios siguen alineados.
//
// Uso:
//   cargo run --example tutorial_acto_1_florentine -- test_dbs/florentine_families.db
//
// La DB debe existir. Generala con:
//   python nopaldb/examples/florentine_families_dataset.py \
//     --db test_dbs/florentine_families.db --reset

use nopaldb::Graph;
use std::error::Error;

const ACTO_DIR: &str = "../docs/tutorial/acto_1_florentine/queries";

const Q_MODELO: &str = include_str!("../../docs/tutorial/acto_1_florentine/queries/01_modelo.nql");
const Q_PATTERN: &str =
    include_str!("../../docs/tutorial/acto_1_florentine/queries/02_pattern_matching.nql");
const Q_CENTRALIDAD: &str =
    include_str!("../../docs/tutorial/acto_1_florentine/queries/03_centralidad.nql");
const Q_COMMUNITIES: &str =
    include_str!("../../docs/tutorial/acto_1_florentine/queries/04_communities.nql");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test_dbs/florentine_families.db".to_string());

    println!("=== Tutorial NopalDB — Acto 1: Florentine Families ===");
    println!("DB: {}", db_path);
    println!("Queries cargadas desde: {}\n", ACTO_DIR);

    let graph = Graph::open(&db_path).await?;

    paso_1_modelo(&graph).await?;
    paso_2_pattern_matching(&graph).await?;
    let top3 = paso_3_centralidad(&graph).await?;
    paso_4_communities(&graph).await?;
    gate(&top3)?;

    graph.close().await?;
    println!("\n=== Acto 1 completado ===");
    Ok(())
}

/// Paso 1 — Validar el modelo (15 familias).
async fn paso_1_modelo(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 1: modelo ---");
    let result = graph.execute_nql(Q_MODELO).await?;
    println!("Filas: {}", result.len());
    if result.len() != 15 {
        eprintln!(
            "  WARN: esperaba 15 familias, obtuve {}. ¿La DB se generó completa?",
            result.len()
        );
    }
    println!("Columnas: {:?}", result.columns);
    for row in result.rows.iter().take(3) {
        println!("  {:?}", row.values);
    }
    println!();
    Ok(())
}

/// Paso 2 — Vecinas directas de Medici.
async fn paso_2_pattern_matching(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 2: pattern matching (vecinas de Medici) ---");
    let result = graph.execute_nql(Q_PATTERN).await?;
    println!("Aliadas directas de Medici: {} familias", result.len());
    for row in &result.rows {
        let aliada = row
            .values
            .get("aliada")
            .map(|v| format!("{:?}", v))
            .unwrap_or_else(|| "?".into());
        let faccion = row
            .values
            .get("faccion_aliada")
            .map(|v| format!("{:?}", v))
            .unwrap_or_else(|| "?".into());
        println!("  {} ({})", aliada, faccion);
    }
    println!();
    Ok(())
}

/// Paso 3 — Centralidad. Devuelve los nombres del top-3 PageRank para el gate.
async fn paso_3_centralidad(graph: &Graph) -> Result<Vec<String>, Box<dyn Error>> {
    println!("--- Paso 3: centralidad (4 métricas) ---");
    let result = graph.execute_nql(Q_CENTRALIDAD).await?;
    println!(
        "{:>4}  {:<14} {:>10} {:>10} {:>10} {:>10}",
        "#", "familia", "pagerank", "btw", "cc", "deg"
    );
    let mut top3: Vec<String> = Vec::new();
    for (i, row) in result.rows.iter().enumerate() {
        let name = row
            .values
            .get("f.name")
            .map(prop_to_string)
            .unwrap_or_default();
        let pr = row.values.get("pr").map(prop_to_f64).unwrap_or(0.0);
        let btw = row.values.get("btw").map(prop_to_f64).unwrap_or(0.0);
        let cc = row.values.get("cc").map(prop_to_f64).unwrap_or(0.0);
        let deg = row.values.get("deg").map(prop_to_f64).unwrap_or(0.0);
        println!(
            "{:>4}  {:<14} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            i + 1,
            name,
            pr,
            btw,
            cc,
            deg
        );
        if top3.len() < 3 {
            top3.push(name);
        }
    }
    println!();
    Ok(top3)
}

/// Paso 4 — Comparar Louvain vs Leiden.
async fn paso_4_communities(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 4: communities (Louvain vs Leiden) ---");
    let result = graph.execute_nql(Q_COMMUNITIES).await?;
    let mut louvain: std::collections::BTreeMap<i64, Vec<String>> = Default::default();
    let mut leiden: std::collections::BTreeMap<i64, Vec<String>> = Default::default();
    for row in &result.rows {
        let name = row
            .values
            .get("f.name")
            .map(prop_to_string)
            .unwrap_or_default();
        let l = row.values.get("louvain").map(prop_to_i64).unwrap_or(-1);
        let le = row.values.get("leiden").map(prop_to_i64).unwrap_or(-1);
        louvain.entry(l).or_default().push(name.clone());
        leiden.entry(le).or_default().push(name);
    }
    println!("Louvain communities:");
    for (cid, names) in &louvain {
        let mut sorted = names.clone();
        sorted.sort();
        println!("  {}: {:?}", cid, sorted);
    }
    println!("Leiden communities:");
    for (cid, names) in &leiden {
        let mut sorted = names.clone();
        sorted.sort();
        println!("  {}: {:?}", cid, sorted);
    }
    println!();
    Ok(())
}

/// Gate cruzado — el invariante del Acto 1: Medici primero en PageRank.
fn gate(top3: &[String]) -> Result<(), Box<dyn Error>> {
    println!("--- Gate: top-3 PageRank ---");
    println!("Top-3: {:?}", top3);
    if top3.first().map(|s| s.as_str()) != Some("Medici") {
        return Err(format!(
            "Gate fallido: esperaba Medici en #1, obtuve {:?}",
            top3.first()
        )
        .into());
    }
    println!("GATE OK — Medici domina por posición estructural.");
    Ok(())
}

// --- helpers para extraer PropertyValue como tipos Rust ---

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
