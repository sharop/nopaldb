// examples/mvcc_time_travel.rs
//! Demostracion de MVCC time-travel en NopalDB.
//!
//! Escenario: expediente medico de un paciente con historia clinica
//! que evoluciona a lo largo del tiempo. Con time-travel puedes
//! responder "que decia este registro en un momento especifico del pasado".
//!
//! Ejecutar:
//!   cargo run --example mvcc_time_travel
//!   cargo run --example mvcc_time_travel --features reasoner

use nopaldb::Graph;
use nopaldb::types::{Node, PropertyValue};

#[cfg(feature = "reasoner")]
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock never before epoch")
        .as_secs()
}

fn prop_str(node: &Node, key: &str) -> String {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => s.clone(),
        Some(other) => format!("{:?}", other),
        None => "(sin valor)".to_string(),
    }
}

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    println!("=== MVCC Time-Travel — Expediente Medico ===\n");

    // ═══════════════════════════════════════════════════════════════════
    // A. Construir la linea de tiempo
    // ═══════════════════════════════════════════════════════════════════

    println!("--- A. Construyendo la linea de tiempo ---\n");

    // Version 1: ingreso con diagnostico inicial
    let paciente_id = {
        let mut tx = graph.begin_transaction().await?;
        let nodo = Node::new("Paciente")
            .with_property("nombre", PropertyValue::String("Laura Gomez".into()))
            .with_property(
                "diagnostico",
                PropertyValue::String("sospecha viral".into()),
            )
            .with_property("estado", PropertyValue::String("activo".into()));
        let id = tx.add_node(nodo).await?;
        tx.commit().await?;
        id
    };

    println!("v1 — Ingreso: diagnostico='sospecha viral', estado='activo'");

    // Version 2: confirmacion diagnostica y tratamiento
    {
        let mut tx = graph.begin_transaction().await?;
        let mut nodo = graph.get_node(paciente_id).await?;
        nodo.properties.insert(
            "diagnostico".into(),
            PropertyValue::String("COVID-19 confirmado".into()),
        );
        nodo.properties.insert(
            "tratamiento".into(),
            PropertyValue::String("Remdesivir".into()),
        );
        tx.add_node(nodo).await?;
        tx.commit().await?;
    }

    println!("v2 — Confirmacion: diagnostico='COVID-19 confirmado', tratamiento='Remdesivir'");

    // Version 3: alta medica
    {
        let mut tx = graph.begin_transaction().await?;
        let mut nodo = graph.get_node(paciente_id).await?;
        nodo.properties
            .insert("estado".into(), PropertyValue::String("recuperado".into()));
        nodo.properties
            .insert("alta".into(), PropertyValue::String("2024-03-15".into()));
        tx.add_node(nodo).await?;
        tx.commit().await?;
    }

    println!("v3 — Alta: estado='recuperado', alta='2024-03-15'\n");

    // ═══════════════════════════════════════════════════════════════════
    // B. Historial completo
    // ═══════════════════════════════════════════════════════════════════

    println!("--- B. Historial completo (graph.history) ---\n");

    // history() retorna versiones ordenadas de mas reciente a mas antigua
    let history = graph.history(paciente_id).await?;
    println!("Versiones almacenadas: {}", history.len());

    for (i, v) in history.iter().enumerate() {
        let diagnostico = match v.node_data.properties.get("diagnostico") {
            Some(PropertyValue::String(s)) => s.as_str(),
            _ => "(sin diagnostico)",
        };
        let estado = match v.node_data.properties.get("estado") {
            Some(PropertyValue::String(s)) => s.as_str(),
            _ => "(sin estado)",
        };
        println!(
            "  [{i}] t={} version={} | diagnostico='{}' estado='{}'",
            v.timestamp, v.version, diagnostico, estado
        );
    }

    // Extraer timestamps internos del MVCC para as_of() y get_node_at()
    // history() ordena de mas reciente a mas antigua: [v3, v2, v1]
    let t_v1 = history.last().map(|v| v.timestamp).unwrap_or(0);
    let t_v2 = history
        .get(history.len().saturating_sub(2))
        .map(|v| v.timestamp)
        .unwrap_or(0);
    println!();

    // ═══════════════════════════════════════════════════════════════════
    // C. Time-travel con as_of() — estilo Datomic
    // ═══════════════════════════════════════════════════════════════════

    println!("--- C. Snapshots historicos (graph.as_of) ---\n");

    // Snapshot en t_v1: el nodo en su version inicial (ingreso)
    let snap_v1 = graph.as_of(t_v1);
    match snap_v1.get_node(paciente_id).await {
        Ok(nodo) => println!(
            "  as_of(t_v1) — diagnostico='{}' estado='{}'",
            prop_str(&nodo, "diagnostico"),
            prop_str(&nodo, "estado"),
        ),
        Err(_) => println!("  as_of(t_v1) — nodo no encontrado"),
    }

    // Snapshot en t_v2: despues de la confirmacion, antes del alta
    let snap_v2 = graph.as_of(t_v2);
    match snap_v2.get_node(paciente_id).await {
        Ok(nodo) => println!(
            "  as_of(t_v2) — diagnostico='{}' tratamiento='{}' estado='{}'",
            prop_str(&nodo, "diagnostico"),
            prop_str(&nodo, "tratamiento"),
            prop_str(&nodo, "estado"),
        ),
        Err(_) => println!("  as_of(t_v2) — nodo no encontrado"),
    }

    // Estado actual
    let actual = graph.get_node(paciente_id).await?;
    println!(
        "  actual      — estado='{}' alta='{}'",
        prop_str(&actual, "estado"),
        prop_str(&actual, "alta"),
    );
    println!();

    // ═══════════════════════════════════════════════════════════════════
    // D. Consulta directa por timestamp
    // ═══════════════════════════════════════════════════════════════════

    println!("--- D. Consulta directa (graph.get_node_at) ---\n");

    match graph.get_node_at(paciente_id, t_v1).await {
        Ok(nodo) => println!(
            "  get_node_at(t_v1) — diagnostico='{}'",
            prop_str(&nodo, "diagnostico"),
        ),
        Err(e) => println!("  get_node_at(t_v1) — no encontrado: {e}"),
    }

    match graph.get_node_at(paciente_id, t_v2).await {
        Ok(nodo) => println!(
            "  get_node_at(t_v2) — tratamiento='{}'",
            prop_str(&nodo, "tratamiento"),
        ),
        Err(e) => println!("  get_node_at(t_v2) — no encontrado: {e}"),
    }

    println!();

    // ═══════════════════════════════════════════════════════════════════
    // E. ELReasoner::from_graph_at — razonamiento en punto historico
    //    (requiere: --features reasoner)
    // ═══════════════════════════════════════════════════════════════════

    #[cfg(feature = "reasoner")]
    {
        use nopaldb::reasoner::ELReasoner;
        use nopaldb::types::NodeKind;

        println!("--- E. ELReasoner time-travel (feature = reasoner) ---\n");

        // Crear jerarquia inicial: Enfermedad <- Infeccion <- Viral
        let (enfermedad_id, viral_id) = {
            let mut tx = graph.begin_transaction().await?;

            let mut enfermedad = Node::new("Enfermedad");
            enfermedad.kind = NodeKind::Class;

            let mut infeccion = Node::new("Infeccion");
            infeccion.kind = NodeKind::Class;

            let mut viral = Node::new("Viral");
            viral.kind = NodeKind::Class;

            let eid = tx.add_node(enfermedad.clone()).await?;
            let iid = tx.add_node(infeccion.clone()).await?;
            let vid = tx.add_node(viral.clone()).await?;

            tx.add_edge(nopaldb::types::Edge {
                id: uuid::Uuid::new_v4(),
                source: iid,
                target: eid,
                edge_type: "subClassOf".into(),
                properties: Default::default(),
            })
            .unwrap();

            tx.add_edge(nopaldb::types::Edge {
                id: uuid::Uuid::new_v4(),
                source: vid,
                target: iid,
                edge_type: "subClassOf".into(),
                properties: Default::default(),
            })
            .unwrap();

            tx.commit().await?;
            (eid, vid)
        };

        println!("Jerarquia inicial: Viral ⊑ Infeccion ⊑ Enfermedad");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let _t_pre_bacteriana = now_secs();

        // Reasoner reconstruido en el timestamp actual del grafo
        // Nota: las aristas no tienen MVCC todavia (v1), por lo que
        // from_graph_at() refleja el estado de nodos en el timestamp dado
        // pero lee las aristas desde el estado actual del grafo.
        let mut reasoner = ELReasoner::from_graph_at(&graph, now_secs()).await?;
        let inferences = reasoner.classify_all();
        println!(
            "  from_graph_at(ahora) — {} inferencias derivadas",
            inferences.len(),
        );
        println!(
            "  Viral ⊑ Infeccion ⊑ Enfermedad — Viral ⊑ Enfermedad (CR1): {}",
            reasoner.is_subclass_of(viral_id, enfermedad_id),
        );
        println!();
        println!("  (time-travel del reasoner a nivel de nodos: ELReasoner::from_graph_at)");
        println!("  (aristas: MVCC implementado — graph.edge_history(id) disponible)");

        println!();
    }

    println!("=== Demo completado ===");
    Ok(())
}
