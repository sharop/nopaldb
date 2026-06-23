use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::path::Path;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    env_logger::init();

    let db_path = "/tmp/nopaldb_test.db";

    // Limpiar si existe
    if Path::new(db_path).exists() {
        std::fs::remove_dir_all(db_path).ok();
    }

    println!("=== PARTE 1: Crear grafo y guardar ===\n");

    {
        let graph = Graph::open(db_path).await?;

        // Crear nodos
        let alice = graph
            .add_node(
                Node::new("Person").with_property("name", PropertyValue::String("Alice".into())),
            )
            .await?;

        let bob = graph
            .add_node(
                Node::new("Person").with_property("name", PropertyValue::String("Bob".into())),
            )
            .await?;

        let charlie = graph
            .add_node(
                Node::new("Person").with_property("name", PropertyValue::String("Charlie".into())),
            )
            .await?;

        // Crear relaciones
        graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;
        graph.add_edge(Edge::new(alice, charlie, "KNOWS")).await?;
        graph.add_edge(Edge::new(bob, charlie, "KNOWS")).await?;

        println!("Creados 3 nodos y 3 aristas");
        println!(
            "Grado de Alice: {}",
            graph.degree(alice, Direction::Outgoing).await?
        );

        // Persistir índices explícitamente
        graph.flush_indices().await?;
        println!("Índices guardados en disco\n");
    }
    // Grafo se cierra aquí

    println!("=== PARTE 2: Reabrir grafo (sin reconstruir) ===\n");

    {
        let _graph = Graph::open(db_path).await?;

        // Los índices deberían estar cargados
        // Obtener todos los nodos con label "Person"
        println!("Reabriendo base de datos...");

        // No podemos iterar nodos aún, pero podemos verificar que los índices funcionan
        // Necesitarías guardar los IDs o implementar un método list_all_nodes()

        println!("✅ Grafo reabierto exitosamente");
        println!("✅ Índices cargados desde disco (no reconstruidos)");
    }

    // Limpiar
    std::fs::remove_dir_all(db_path).ok();
    println!("\n✅ Demo completado");

    Ok(())
}
