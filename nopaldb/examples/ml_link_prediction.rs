// examples/ml_link_prediction.rs
//
// Link Prediction: Predice enlaces futuros en red de citaciones

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::collections::HashSet;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          LINK PREDICTION - CITATION NETWORK               ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    // Crear red de citaciones
    println!("📊 Creando red de citaciones...");
    let papers = create_citation_network(&graph).await?;
    println!("   ✓ {} papers creados\n", papers.len());

    // Predecir siguiente citación para Paper A
    let paper_a = papers[0];
    println!("📄 Predicción de citaciones para Paper A:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let predictions = predict_links(&graph, paper_a, 3).await?;

    for (i, (paper_id, score)) in predictions.iter().enumerate() {
        let paper = graph.get_node(*paper_id).await?;
        let title = paper.properties.get("title").unwrap();
        println!("  {}. {:?} (score: {:.3})", i + 1, title, score);
    }

    println!();
    Ok(())
}

async fn create_citation_network(graph: &Graph) -> nopaldb::Result<Vec<uuid::Uuid>> {
    // Papers
    let paper_a = graph
        .add_node(
            Node::new("Paper")
                .with_property("title", PropertyValue::String("Graph Databases".into()))
                .with_property("topic", PropertyValue::String("Databases".into())),
        )
        .await?;

    let paper_b = graph
        .add_node(
            Node::new("Paper")
                .with_property("title", PropertyValue::String("NoSQL Systems".into()))
                .with_property("topic", PropertyValue::String("Databases".into())),
        )
        .await?;

    let paper_c = graph
        .add_node(
            Node::new("Paper")
                .with_property("title", PropertyValue::String("Graph Algorithms".into()))
                .with_property("topic", PropertyValue::String("Algorithms".into())),
        )
        .await?;

    let paper_d = graph
        .add_node(
            Node::new("Paper")
                .with_property("title", PropertyValue::String("Network Analysis".into()))
                .with_property("topic", PropertyValue::String("Networks".into())),
        )
        .await?;

    let paper_e = graph
        .add_node(
            Node::new("Paper")
                .with_property(
                    "title",
                    PropertyValue::String("Machine Learning on Graphs".into()),
                )
                .with_property("topic", PropertyValue::String("ML".into())),
        )
        .await?;

    // Citations: A cita B y C
    graph.add_edge(Edge::new(paper_a, paper_b, "CITES")).await?;
    graph.add_edge(Edge::new(paper_a, paper_c, "CITES")).await?;

    // B cita C y D
    graph.add_edge(Edge::new(paper_b, paper_c, "CITES")).await?;
    graph.add_edge(Edge::new(paper_b, paper_d, "CITES")).await?;

    // C cita D
    graph.add_edge(Edge::new(paper_c, paper_d, "CITES")).await?;

    // D cita E
    graph.add_edge(Edge::new(paper_d, paper_e, "CITES")).await?;

    Ok(vec![paper_a, paper_b, paper_c, paper_d, paper_e])
}

/// Predice próximos enlaces usando Adamic-Adar Index
async fn predict_links(
    graph: &Graph,
    source_paper: uuid::Uuid,
    top_n: usize,
) -> nopaldb::Result<Vec<(uuid::Uuid, f64)>> {
    // 1. Obtener papers ya citados
    let cited_papers = get_cited_papers(graph, source_paper).await?;

    // 2. Obtener candidatos (vecinos de vecinos)
    let mut candidates: HashSet<uuid::Uuid> = HashSet::new();

    for &cited_id in &cited_papers {
        let second_order = get_cited_papers(graph, cited_id).await?;
        for candidate in second_order {
            if candidate != source_paper && !cited_papers.contains(&candidate) {
                candidates.insert(candidate);
            }
        }
    }

    // 3. Calcular Adamic-Adar score para cada candidato
    let mut scores = Vec::new();

    for &candidate_id in &candidates {
        let score = adamic_adar_score(graph, source_paper, candidate_id).await?;
        scores.push((candidate_id, score));
    }

    // 4. Ordenar y retornar top N
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores.truncate(top_n);

    Ok(scores)
}

async fn get_cited_papers(
    graph: &Graph,
    paper_id: uuid::Uuid,
) -> nopaldb::Result<HashSet<uuid::Uuid>> {
    let edges = graph.edges_of(paper_id, Direction::Outgoing).await?;

    let cited: HashSet<_> = edges
        .iter()
        .filter(|e| e.edge_type == "CITES")
        .map(|e| e.target)
        .collect();

    Ok(cited)
}

/// Adamic-Adar Index: Suma de 1/log(degree) de vecinos comunes
// Reemplazar función adamic_adar_score:

/// Adamic-Adar Index mejorado
async fn adamic_adar_score(
    graph: &Graph,
    paper1: uuid::Uuid,
    paper2: uuid::Uuid,
) -> nopaldb::Result<f64> {
    let cited1 = get_cited_papers(graph, paper1).await?;
    let cited2 = get_cited_papers(graph, paper2).await?;

    let common: HashSet<_> = cited1.intersection(&cited2).collect();

    if common.is_empty() {
        // Si no hay vecinos comunes, usar preferential attachment
        let degree1 = cited1.len() as f64;
        let degree2 = cited2.len() as f64;
        return Ok(degree1 * degree2);
    }

    let mut score = 0.0;

    for &&common_paper in &common {
        let degree = graph.degree(common_paper, Direction::Outgoing).await? as f64;
        if degree > 1.0 {
            score += 1.0 / degree.log2();
        } else {
            score += 1.0; // Evitar división por cero
        }
    }

    Ok(score)
}
