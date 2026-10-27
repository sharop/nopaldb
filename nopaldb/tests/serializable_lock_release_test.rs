// tests/serializable_lock_release_test.rs
//
// #296: bajo Serializable, toda forma de terminar una transacción sin commit
// (Drop, rollback() síncrono, un `?` que propaga un error) debe soltar sus
// candados. Antes solo los soltaban un commit fallido y rollback_async(): el
// siguiente escritor sobre esos nodos esperaba y fallaba con "Lock timeout"
// a los 5 s.
//
// Cada test envuelve al siguiente escritor en un timeout de 2 s: muy por
// debajo del timeout del lock manager (5 s), así una regresión falla rápido
// en vez de colgarse.

#![cfg(feature = "full-isolation")]

use nopaldb::{Graph, IsolationLevel, Node, NodeId, PropertyValue, Result, Transaction};
use std::sync::Arc;
use std::time::Duration;

const LIMITE: Duration = Duration::from_secs(2);

async fn sembrar_planta(graph: &Graph, nombre: &str) -> NodeId {
    let mut tx = graph.begin_transaction().await.unwrap();
    let id = tx
        .add_node(
            Node::new("Planta")
                .with_property("nombre", PropertyValue::String(nombre.into()))
                .with_property("riegos", PropertyValue::Int(0)),
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

fn planta_con_riegos(id: NodeId, riegos: i64) -> Node {
    let mut n = Node::new("Planta").with_property("riegos", PropertyValue::Int(riegos));
    n.id = id;
    n
}

async fn serializable(graph: &Graph) -> Transaction {
    graph
        .begin_transaction()
        .await
        .unwrap()
        .with_isolation(IsolationLevel::Serializable)
}

/// El siguiente escritor Serializable sobre `id` debe tomar el candado y
/// commitear dentro de `LIMITE`.
async fn siguiente_escritor_commitea(graph: &Graph, id: NodeId, riegos: i64) {
    let resultado = tokio::time::timeout(LIMITE, async {
        let mut tx = serializable(graph).await;
        tx.add_node(planta_con_riegos(id, riegos)).await?;
        tx.commit().await
    })
    .await;

    match resultado {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("el siguiente escritor falló: {e}"),
        Err(_) => panic!(
            "el siguiente escritor no obtuvo el candado en {LIMITE:?}: \
             la tx anterior lo dejó tomado"
        ),
    }

    let leido = graph.get_node(id).await.unwrap();
    assert_eq!(leido.properties.get("riegos"), Some(&PropertyValue::Int(riegos)));
}

#[tokio::test]
async fn drop_sin_commit_suelta_el_candado_de_escritura() {
    let graph = Graph::in_memory().await.unwrap();
    let nopal = sembrar_planta(&graph, "nopal").await;

    {
        let mut tx = serializable(&graph).await;
        tx.add_node(planta_con_riegos(nopal, 1)).await.unwrap();
        // Se descarta sin commit ni rollback.
    }

    siguiente_escritor_commitea(&graph, nopal, 2).await;
}

#[tokio::test]
async fn drop_sin_commit_suelta_el_candado_de_lectura() {
    let graph = Graph::in_memory().await.unwrap();
    let maguey = sembrar_planta(&graph, "maguey").await;

    {
        let tx = serializable(&graph).await;
        tx.get_node(maguey).await.unwrap();
    }

    siguiente_escritor_commitea(&graph, maguey, 3).await;
}

#[tokio::test]
async fn rollback_sincrono_suelta_el_candado() {
    let graph = Graph::in_memory().await.unwrap();
    let agave = sembrar_planta(&graph, "agave").await;

    let mut tx = serializable(&graph).await;
    tx.add_node(planta_con_riegos(agave, 1)).await.unwrap();
    tx.rollback().unwrap();

    siguiente_escritor_commitea(&graph, agave, 4).await;
}

#[tokio::test]
async fn rollback_async_suelta_el_candado() {
    let graph = Graph::in_memory().await.unwrap();
    let biznaga = sembrar_planta(&graph, "biznaga").await;

    let mut tx = serializable(&graph).await;
    tx.add_node(planta_con_riegos(biznaga, 1)).await.unwrap();
    tx.rollback_async().await.unwrap();

    siguiente_escritor_commitea(&graph, biznaga, 5).await;
}

#[tokio::test]
async fn commit_exitoso_suelta_el_candado_y_el_drop_posterior_no_estorba() {
    let graph = Graph::in_memory().await.unwrap();
    let pitaya = sembrar_planta(&graph, "pitaya").await;

    let mut tx = serializable(&graph).await;
    tx.add_node(planta_con_riegos(pitaya, 1)).await.unwrap();
    tx.commit().await.unwrap();

    siguiente_escritor_commitea(&graph, pitaya, 6).await;
    siguiente_escritor_commitea(&graph, pitaya, 7).await;
}

/// Mueve agua de `origen` a `destino` con el patrón habitual: `?` en cada
/// paso. Si `add_node` devuelve `Deadlock`, la tx se descarta al salir.
async fn trasvasar(mut tx: Transaction, destino: NodeId) -> Result<()> {
    tx.add_node(planta_con_riegos(destino, 10)).await?;
    tx.commit().await
}

#[tokio::test]
async fn deadlock_propagado_con_interrogacion_suelta_los_candados() {
    let graph = Arc::new(Graph::in_memory().await.unwrap());
    let nopal = sembrar_planta(&graph, "nopal").await;
    let maguey = sembrar_planta(&graph, "maguey").await;

    // tx_vieja (id menor) toma nopal; tx_nueva (id mayor) toma maguey.
    let mut tx_vieja = serializable(&graph).await;
    tx_vieja.add_node(planta_con_riegos(nopal, 1)).await.unwrap();
    let mut tx_nueva = serializable(&graph).await;
    tx_nueva.add_node(planta_con_riegos(maguey, 1)).await.unwrap();

    // tx_vieja espera maguey (sin ciclo todavía).
    let espera = tokio::spawn(async move {
        tx_vieja.add_node(planta_con_riegos(maguey, 2)).await?;
        tx_vieja.commit().await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // tx_nueva pide nopal: cierra el ciclo y es la víctima (la más reciente).
    let err = trasvasar(tx_nueva, nopal)
        .await
        .expect_err("tx_nueva debía ser la víctima del deadlock");
    assert!(err.to_string().contains("Deadlock"), "error inesperado: {err}");

    // Al propagarse el error, tx_nueva se descartó: tx_vieja debe obtener
    // maguey y commitear sin esperar el timeout del lock manager.
    match tokio::time::timeout(LIMITE, espera).await {
        Ok(join) => join.unwrap().expect("tx_vieja debía commitear"),
        Err(_) => panic!("tx_vieja siguió esperando el candado de la víctima"),
    }

    siguiente_escritor_commitea(&graph, nopal, 20).await;
    siguiente_escritor_commitea(&graph, maguey, 21).await;
}
