// Tutorial NopalDB - Acto 3: Biomedical OWL + reasoner
//
// Acompana a docs/tutorial/acto_3_biomedical_owl/README.md.
//
// Este ejemplo:
//   1. Importa tutorials/data/biomedical.ttl via Graph::import_turtle().
//      El metodo instala el snapshot en la TaxonomyIndex del grafo.
//   2. Ejecuta queries NQL estructurales (schema, jerarquia subClassOf).
//   3. Verifica que los nodos Class existen via la API publica del grafo.
//   4. Demuestra instanceOf NQL con inferencia transitiva (fix #56, v0.4.20).
//
// Uso:
//   cargo run --example tutorial_acto_3_biomedical \
//     --no-default-features \
//     --features storage-sled,reasoner,owl-import,algorithms,analytics,hypergraph,ml \
//     -- test_dbs/biomedical.db tutorials/data/biomedical.ttl

use nopaldb::Graph;
use std::error::Error;
use std::fs;

const Q_CLASSES: &str =
    include_str!("../../docs/tutorial/acto_3_biomedical_owl/queries/01_classes.nql");
const Q_INSTANCES: &str =
    include_str!("../../docs/tutorial/acto_3_biomedical_owl/queries/02_instances_check.nql");
const Q_SUBCLASS_EDGES: &str =
    include_str!("../../docs/tutorial/acto_3_biomedical_owl/queries/03_subclassof_edges.nql");
const Q_INSTANCEOF_NQL: &str =
    include_str!("../../docs/tutorial/acto_3_biomedical_owl/queries/04_instanceof_nql.nql");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let db_path = args
        .next()
        .unwrap_or_else(|| "test_dbs/biomedical.db".to_string());
    let ttl_path = args
        .next()
        .unwrap_or_else(|| "tutorials/data/biomedical.ttl".to_string());

    println!("=== Tutorial NopalDB - Acto 3: Biomedical OWL ===");
    println!("DB:  {}", db_path);
    println!("TTL: {}", ttl_path);
    println!();

    // Limpiar DB previa para idempotencia.
    let _ = std::fs::remove_dir_all(&db_path);

    let graph = Graph::open(&db_path).await?;

    let ttl_source = fs::read_to_string(&ttl_path)?;
    let report = graph.import_turtle(&ttl_source).await?;
    println!("Import report:");
    println!("  classes_added:        {}", report.classes_added);
    println!("  subclass_edges_added: {}", report.subclass_edges_added);
    println!("  instances_added:      {}", report.instances_added);
    println!("  triples_skipped:      {}", report.triples_skipped);
    println!();

    paso_1_classes(&graph).await?;
    paso_2_instances(&graph).await?;
    paso_3_subclass_edges(&graph).await?;
    paso_4_taxonomy_check(&graph).await?;
    let instance_count = paso_5_instanceof_nql(&graph).await?;

    gate(&report, instance_count)?;

    graph.close().await?;
    println!("\n=== Acto 3 completado ===");
    println!("Para reasoner CR1+CR2+CR3 standalone, ejecuta el notebook 03_biomedical_owl.ipynb.");
    Ok(())
}

async fn paso_1_classes(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 1: clases OWL importadas ---");
    let result = graph.execute_nql(Q_CLASSES).await?;
    let mut seen = std::collections::BTreeSet::new();
    for row in result.rows() {
        if let Some(c) = row.get("clase") {
            seen.insert(prop_to_string(c));
        }
    }
    println!("clases unicas: {}", seen.len());
    for c in &seen {
        println!("  {}", c);
    }
    println!();
    Ok(())
}

async fn paso_2_instances(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 2: instancias por clase directa (con count) ---");
    let result = graph.execute_nql(Q_INSTANCES).await?;
    for row in result.rows() {
        let clase = row.get("clase").map(prop_to_string).unwrap_or_default();
        let total = row.get("total").map(prop_to_i64).unwrap_or(-1);
        println!("  {:<22} {}", clase, total);
    }
    println!();
    Ok(())
}

async fn paso_3_subclass_edges(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 3: edges subClassOf (jerarquia explicita) ---");
    let result = graph.execute_nql(Q_SUBCLASS_EDGES).await?;
    for row in result.rows() {
        let sub = row.get("subclase").map(prop_to_string).unwrap_or_default();
        let sup = row
            .get("superclase")
            .map(prop_to_string)
            .unwrap_or_default();
        println!("  {:<22} subClassOf {}", sub, sup);
    }
    println!();
    Ok(())
}

async fn paso_4_taxonomy_check(graph: &Graph) -> Result<(), Box<dyn Error>> {
    println!("--- Paso 4: verificacion de class nodes via API publica ---");
    let expected_classes = [
        "Disease",
        "Infection",
        "ViralInfection",
        "BacterialInfection",
        "Treatment",
        "Antiviral",
        "Antibiotic",
    ];
    for cls in expected_classes {
        let nodes = graph.get_nodes_by_label(cls).await?;
        if nodes.is_empty() {
            return Err(format!("Falta la clase {} en el grafo", cls).into());
        }
        println!("  {:<22} nodos con ese label: {}", cls, nodes.len());
    }
    println!();
    Ok(())
}

/// Demuestra instanceOf NQL con inferencia transitiva sobre la TaxonomyIndex.
/// Retorna el numero de individuos encontrados para el gate.
async fn paso_5_instanceof_nql(graph: &Graph) -> Result<usize, Box<dyn Error>> {
    println!("--- Paso 5: instanceOf NQL — instancias transitivas de Disease ---");
    let result = graph.execute_nql(Q_INSTANCEOF_NQL).await?;
    let count = result.len();
    for row in result.rows() {
        let clase = row.get("clase").map(prop_to_string).unwrap_or_default();
        let nombre = row.get("nombre").map(prop_to_string).unwrap_or_default();
        println!("  {:<22} {}", clase, nombre);
    }
    println!("total: {} individuos", count);
    println!();
    Ok(count)
}

fn gate(
    report: &nopaldb::rdf_owl::importer::ImportReport,
    instance_count: usize,
) -> Result<(), Box<dyn Error>> {
    println!("--- Gate Acto 3 ---");
    println!("Esperado: 7 classes_added, 5 subclass_edges_added, 9 instances_added");
    println!(
        "Real:     {} / {} / {}",
        report.classes_added, report.subclass_edges_added, report.instances_added
    );
    if report.classes_added != 7 || report.subclass_edges_added != 5 || report.instances_added != 9
    {
        return Err("Gate fallido: import_turtle reporto conteos inesperados".into());
    }
    println!(
        "instanceOf(n, \"Disease\") retorno {} individuos (esperado 5)",
        instance_count
    );
    if instance_count != 5 {
        return Err(format!(
            "Gate fallido: instanceOf deberia retornar 5 individuos Disease, got {}",
            instance_count
        )
        .into());
    }
    println!("GATE OK - import + taxonomy + instanceOf NQL transitivo verificados.");
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

fn prop_to_i64(p: &nopaldb::types::PropertyValue) -> i64 {
    use nopaldb::types::PropertyValue::*;
    match p {
        Int(n) => *n,
        Float(f) => *f as i64,
        _ => -1,
    }
}
