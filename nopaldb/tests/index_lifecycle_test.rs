// Ciclo de vida del nodo indexado: sobrescribir o borrar un nodo debe dejar
// consistentes sus propiedades, su documento full-text y su embedding.
//
// El caso de uso es el índice derivado y reconstruible (fragmentos de
// documentos, notas, catálogos): se re-ingesta la misma fuente una y otra vez,
// y cada re-ingesta sobrescribe nodos existentes. Si la sobrescritura no
// retira lo viejo, el índice acumula entradas fantasma que siguen matcheando
// consultas — el nodo aparece por un valor que ya no tiene.

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::graph::upsert::UpsertRequest;
use nopaldb::{Graph, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn graph() -> (Graph, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let g = Graph::open(dir.path()).await.unwrap();
    (g, dir)
}

/// Full-text por la vía que usa el usuario (la rama de texto de la búsqueda
/// híbrida). Devuelve los NodeId en orden de relevancia.
async fn fulltext(g: &Graph, text: &str) -> Vec<NodeId> {
    let mut q = HybridQuery::new();
    q.text = Some(text.to_string());
    q.k = 50;
    g.search_hybrid(q).await.unwrap().into_iter().map(|h| h.node_id).collect()
}

/// Sobrescribir el valor de una propiedad indexada retira el valor anterior
/// del índice de propiedades: buscar por el valor viejo no puede devolver el
/// nodo, porque el nodo ya no lo tiene.
#[tokio::test]
async fn overwrite_retracts_old_value_from_property_index() {
    let (g, _dir) = graph().await;

    let id = NodeId::new_v4();
    g.add_node(Node::with_id(id, "Doc").with_property("status", s("draft")))
        .await
        .unwrap();

    let found = g.get_all_nodes_by_property("status", &s("draft")).await.unwrap();
    assert_eq!(found, vec![id], "el valor inicial se indexa");

    // Re-ingesta del mismo nodo con otro estado.
    g.add_node(Node::with_id(id, "Doc").with_property("status", s("published")))
        .await
        .unwrap();

    let by_new = g.get_all_nodes_by_property("status", &s("published")).await.unwrap();
    assert_eq!(by_new, vec![id], "el valor nuevo se indexa");

    let by_old = g.get_all_nodes_by_property("status", &s("draft")).await.unwrap();
    assert!(
        by_old.is_empty(),
        "el valor VIEJO debe salir del índice; sigue devolviendo {:?}",
        by_old
    );
}

/// Una propiedad que desaparece por completo en la sobrescritura también debe
/// salir del índice — no solo las que cambian de valor.
#[tokio::test]
async fn overwrite_retracts_dropped_property() {
    let (g, _dir) = graph().await;

    let id = NodeId::new_v4();
    g.add_node(
        Node::with_id(id, "Doc")
            .with_property("status", s("draft"))
            .with_property("owner", s("ana")),
    )
    .await
    .unwrap();

    // La re-ingesta ya no trae `owner`.
    g.add_node(Node::with_id(id, "Doc").with_property("status", s("draft")))
        .await
        .unwrap();

    let by_owner = g.get_all_nodes_by_property("owner", &s("ana")).await.unwrap();
    assert!(
        by_owner.is_empty(),
        "una propiedad eliminada debe salir del índice; sigue devolviendo {:?}",
        by_owner
    );
}

/// Sobrescribir el texto de un nodo deja UN documento full-text, no dos: el
/// texto viejo no puede seguir matcheando.
#[tokio::test]
async fn overwrite_replaces_fulltext_document() {
    let (g, _dir) = graph().await;
    g.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let id = NodeId::new_v4();
    g.add_node(Node::with_id(id, "Doc").with_property("body", s("cactus nopal")))
        .await
        .unwrap();

    let hits = fulltext(&g, "cactus").await;
    assert_eq!(hits, vec![id], "el texto inicial se indexa");

    // Re-ingesta con contenido distinto.
    g.add_node(Node::with_id(id, "Doc").with_property("body", s("agave maguey")))
        .await
        .unwrap();

    let by_new = fulltext(&g, "agave").await;
    assert_eq!(by_new, vec![id], "el texto nuevo se indexa");

    let by_old = fulltext(&g, "cactus").await;
    assert!(
        by_old.is_empty(),
        "el texto VIEJO debe salir del índice; sigue devolviendo {:?}",
        by_old
    );
}


/// Borrar un nodo lo saca del índice de propiedades y del full-text.
#[tokio::test]
async fn delete_node_clears_property_and_fulltext_indexes() {
    let (g, _dir) = graph().await;
    g.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let id = NodeId::new_v4();
    g.add_node(
        Node::with_id(id, "Doc")
            .with_property("body", s("cactus nopal"))
            .with_property("status", s("draft")),
    )
    .await
    .unwrap();

    g.delete_node(id).await.unwrap();

    let by_prop = g.get_all_nodes_by_property("status", &s("draft")).await.unwrap();
    assert!(by_prop.is_empty(), "nodo borrado sigue en el índice de propiedades: {:?}", by_prop);

    let by_text = fulltext(&g, "cactus").await;
    assert!(by_text.is_empty(), "nodo borrado sigue en el full-text: {:?}", by_text);
}

/// Un nodo borrado no puede seguir siendo recuperable por su vector.
///
/// El índice HNSW se reconstruye desde los embeddings del storage, así que un
/// embedding huérfano no solo ocupa espacio: **resucita el nodo** en la
/// siguiente reconstrucción del índice.
#[tokio::test]
async fn delete_node_removes_its_embedding_from_vector_search() {
    let (g, _dir) = graph().await;

    let keep = NodeId::new_v4();
    let doomed = NodeId::new_v4();
    g.add_node(Node::with_id(keep, "Doc").with_property("name", s("keep")))
        .await
        .unwrap();
    g.add_node(Node::with_id(doomed, "Doc").with_property("name", s("doomed")))
        .await
        .unwrap();
    g.add_node_embedding(keep, vec![0.0, 1.0, 0.0], "m").await.unwrap();
    g.add_node_embedding(doomed, vec![1.0, 0.0, 0.0], "m").await.unwrap();

    g.delete_node(doomed).await.unwrap();

    // Consulta pegada al vector del nodo borrado: si sigue indexado, gana.
    let mut q = HybridQuery::new();
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.k = 10;
    let hits: Vec<NodeId> = g
        .search_hybrid(q)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.node_id)
        .collect();

    assert!(
        !hits.contains(&doomed),
        "el nodo borrado sigue apareciendo en la búsqueda vectorial: {:?}",
        hits
    );
    assert!(hits.contains(&keep), "el nodo vivo debe seguir apareciendo");

    // El embedding tampoco debe quedar huérfano en el storage.
    assert!(
        g.get_node_embedding(doomed, "m").await.is_err(),
        "el embedding del nodo borrado sigue en el storage"
    );
}

/// Un nodo escrito por transacción debe entrar al full-text igual que uno
/// escrito directo. El commit indexa sus nodos al final, en un solo paso; si
/// ese paso solo toca una de las dos capas de índice, el nodo queda invisible
/// a la búsqueda de texto hasta reabrir la base.
#[tokio::test]
async fn transactional_writes_reach_the_fulltext_index() {
    let (g, _dir) = graph().await;
    g.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let direct = NodeId::new_v4();
    g.add_node(Node::with_id(direct, "Doc").with_property("body", s("alfa")))
        .await
        .unwrap();

    let via_tx = NodeId::new_v4();
    let mut tx = g.begin_transaction().await.unwrap();
    tx.add_node(Node::with_id(via_tx, "Doc").with_property("body", s("beta")))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(fulltext(&g, "alfa").await, vec![direct], "escritura directa indexada");
    assert_eq!(
        fulltext(&g, "beta").await,
        vec![via_tx],
        "la escritura transaccional también debe quedar indexada"
    );
}

/// `upsert_node` es la API de re-ingesta idempotente: es exactamente la que
/// sobrescribe nodos una y otra vez, así que su ciclo completo —indexar al
/// crear, retirar lo viejo al actualizar— tiene que cerrar.
#[tokio::test]
async fn upsert_indexes_on_create_and_retracts_on_update() {
    use std::collections::HashMap;

    let (g, _dir) = graph().await;
    g.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let mut props = HashMap::new();
    props.insert("slug".to_string(), s("nopal"));
    props.insert("body".to_string(), s("cactus nopal"));
    let req = UpsertRequest {
        label: "Doc".into(),
        key: "slug".into(),
        props: props.clone(),
        embedding: None,
        links: vec![],
    };
    let (_, created) = g.upsert_node(req.clone()).await.unwrap();

    assert_eq!(
        fulltext(&g, "cactus").await,
        vec![created],
        "upsert de creación debe indexar el texto"
    );

    // Segunda ingesta de la misma fuente con el contenido corregido.
    props.insert("body".to_string(), s("agave maguey"));
    let (_, updated) = g
        .upsert_node(UpsertRequest { props, ..req })
        .await
        .unwrap();
    assert_eq!(updated, created, "misma clave de negocio, mismo nodo");

    assert_eq!(fulltext(&g, "agave").await, vec![created], "texto nuevo indexado");
    assert!(
        fulltext(&g, "cactus").await.is_empty(),
        "el texto viejo debe salir del índice tras el upsert de actualización"
    );
}

/// Re-ingestar un nodo sin cambiar una propiedad indexada no debe duplicarlo
/// en el índice. La retracción solo retira lo que cambió —a propósito, para no
/// dejar un instante sin indexar—, así que el valor estable se re-inserta en
/// cada escritura: la inserción tiene que ser idempotente por su cuenta.
#[tokio::test]
async fn repeated_reingest_does_not_duplicate_index_entries() {
    let (g, _dir) = graph().await;
    g.create_index("Doc", "status", IndexType::Hash).await.unwrap();

    let id = NodeId::new_v4();
    for i in 0..5 {
        g.add_node(
            Node::with_id(id, "Doc")
                .with_property("status", s("draft"))
                .with_property("rev", PropertyValue::Int(i)),
        )
        .await
        .unwrap();
    }

    let indexed = g
        .find_nodes_indexed("Doc", "status", s("draft"))
        .await
        .unwrap();
    assert_eq!(
        indexed.len(),
        1,
        "el índice de usuario debe tener el nodo UNA vez tras 5 re-ingestas"
    );

    let by_prop = g.get_all_nodes_by_property("status", &s("draft")).await.unwrap();
    assert_eq!(by_prop, vec![id], "el índice de propiedades tampoco duplica");
}
