// examples/ml_recommendation.rs
//
// Sistema de Recomendaciones usando Collaborative Filtering
// Dataset: MovieLens-style (Users → Movies → Genres)

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::collections::{HashMap, HashSet};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║     SISTEMA DE RECOMENDACIONES - COLLABORATIVE FILTERING  ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    // 1. Crear dataset
    println!("📊 Creando dataset...");
    let (users, movies) = create_movie_dataset(&graph).await?;
    println!(
        "   ✓ {} usuarios, {} películas\n",
        users.len(),
        movies.len()
    );

    // 2. Recomendar para Bob
    let bob_id = users[1];
    println!("🎬 Recomendaciones para Bob:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let recommendations = recommend_movies(&graph, bob_id, 5).await?;

    for (i, (movie_id, score)) in recommendations.iter().enumerate() {
        let movie = graph.get_node(*movie_id).await?;
        let title = movie.properties.get("title").unwrap();
        println!("  {}. {:?} (score: {:.2})", i + 1, title, score);
    }

    println!();
    Ok(())
}

/// Crea dataset de películas
async fn create_movie_dataset(
    graph: &Graph,
) -> nopaldb::Result<(Vec<uuid::Uuid>, Vec<uuid::Uuid>)> {
    // Usuarios
    let alice = graph
        .add_node(Node::new("User").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = graph
        .add_node(Node::new("User").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let charlie = graph
        .add_node(Node::new("User").with_property("name", PropertyValue::String("Charlie".into())))
        .await?;
    let diana = graph
        .add_node(Node::new("User").with_property("name", PropertyValue::String("Diana".into())))
        .await?;

    // Películas
    let inception = graph
        .add_node(
            Node::new("Movie")
                .with_property("title", PropertyValue::String("Inception".into()))
                .with_property("genre", PropertyValue::String("Sci-Fi".into())),
        )
        .await?;

    let matrix = graph
        .add_node(
            Node::new("Movie")
                .with_property("title", PropertyValue::String("The Matrix".into()))
                .with_property("genre", PropertyValue::String("Sci-Fi".into())),
        )
        .await?;

    let interstellar = graph
        .add_node(
            Node::new("Movie")
                .with_property("title", PropertyValue::String("Interstellar".into()))
                .with_property("genre", PropertyValue::String("Sci-Fi".into())),
        )
        .await?;

    let titanic = graph
        .add_node(
            Node::new("Movie")
                .with_property("title", PropertyValue::String("Titanic".into()))
                .with_property("genre", PropertyValue::String("Romance".into())),
        )
        .await?;

    let notebook = graph
        .add_node(
            Node::new("Movie")
                .with_property("title", PropertyValue::String("The Notebook".into()))
                .with_property("genre", PropertyValue::String("Romance".into())),
        )
        .await?;

    // Ratings: Alice le gustan Sci-Fi
    graph
        .add_edge(
            Edge::new(alice, inception, "RATED").with_property("rating", PropertyValue::Int(5)),
        )
        .await?;
    graph
        .add_edge(Edge::new(alice, matrix, "RATED").with_property("rating", PropertyValue::Int(5)))
        .await?;

    // Bob también Sci-Fi + Interstellar
    graph
        .add_edge(Edge::new(bob, inception, "RATED").with_property("rating", PropertyValue::Int(5)))
        .await?;
    graph
        .add_edge(Edge::new(bob, matrix, "RATED").with_property("rating", PropertyValue::Int(4)))
        .await?;
    graph
        .add_edge(
            Edge::new(bob, interstellar, "RATED").with_property("rating", PropertyValue::Int(5)),
        )
        .await?;

    // Charlie: Sci-Fi + Romance
    graph
        .add_edge(
            Edge::new(charlie, matrix, "RATED").with_property("rating", PropertyValue::Int(5)),
        )
        .await?;
    graph
        .add_edge(
            Edge::new(charlie, interstellar, "RATED")
                .with_property("rating", PropertyValue::Int(4)),
        )
        .await?;
    graph
        .add_edge(
            Edge::new(charlie, titanic, "RATED").with_property("rating", PropertyValue::Int(3)),
        )
        .await?;

    // Diana: Romance
    graph
        .add_edge(Edge::new(diana, titanic, "RATED").with_property("rating", PropertyValue::Int(5)))
        .await?;
    graph
        .add_edge(
            Edge::new(diana, notebook, "RATED").with_property("rating", PropertyValue::Int(5)),
        )
        .await?;

    Ok((
        vec![alice, bob, charlie, diana],
        vec![inception, matrix, interstellar, titanic, notebook],
    ))
}

/// Algoritmo de recomendación: Collaborative Filtering
async fn recommend_movies(
    graph: &Graph,
    user_id: uuid::Uuid,
    top_n: usize,
) -> nopaldb::Result<Vec<(uuid::Uuid, f64)>> {
    // 1. Obtener películas que el usuario YA vio
    let watched_movies = get_watched_movies(graph, user_id).await?;

    // 2. Encontrar usuarios similares (que vieron las mismas películas)
    let similar_users = find_similar_users(graph, user_id, &watched_movies).await?;

    // 3. Obtener películas que vieron usuarios similares (pero no el target)
    let mut candidate_scores: HashMap<uuid::Uuid, f64> = HashMap::new();

    for (similar_user_id, similarity) in similar_users {
        let their_movies = get_watched_movies(graph, similar_user_id).await?;

        for movie_id in their_movies {
            if !watched_movies.contains(&movie_id) {
                *candidate_scores.entry(movie_id).or_insert(0.0) += similarity;
            }
        }
    }

    // 4. Ordenar por score y retornar top N
    let mut recommendations: Vec<_> = candidate_scores.into_iter().collect();
    recommendations.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    recommendations.truncate(top_n);

    Ok(recommendations)
}

/// Obtiene películas vistas por un usuario
async fn get_watched_movies(
    graph: &Graph,
    user_id: uuid::Uuid,
) -> nopaldb::Result<HashSet<uuid::Uuid>> {
    let edges = graph.edges_of(user_id, Direction::Outgoing).await?;

    let movie_ids: HashSet<_> = edges
        .iter()
        .filter(|e| e.edge_type == "RATED")
        .map(|e| e.target)
        .collect();

    Ok(movie_ids)
}

/// Encuentra usuarios similares usando Jaccard similarity
async fn find_similar_users(
    graph: &Graph,
    user_id: uuid::Uuid,
    user_movies: &HashSet<uuid::Uuid>,
) -> nopaldb::Result<Vec<(uuid::Uuid, f64)>> {
    let mut similarities = Vec::new();

    // Para cada película vista, encontrar otros usuarios que también la vieron
    let mut other_users: HashSet<uuid::Uuid> = HashSet::new();

    for &movie_id in user_movies {
        let incoming_edges = graph.edges_of(movie_id, Direction::Incoming).await?;

        for edge in incoming_edges {
            if edge.edge_type == "RATED" && edge.source != user_id {
                other_users.insert(edge.source);
            }
        }
    }

    // Calcular Jaccard similarity con cada usuario
    for other_user_id in other_users {
        let other_movies = get_watched_movies(graph, other_user_id).await?;

        let intersection: HashSet<_> = user_movies.intersection(&other_movies).collect();
        let union: HashSet<_> = user_movies.union(&other_movies).collect();

        let jaccard = intersection.len() as f64 / union.len() as f64;

        if jaccard > 0.0 {
            similarities.push((other_user_id, jaccard));
        }
    }

    // Ordenar por similaridad
    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    Ok(similarities)
}
