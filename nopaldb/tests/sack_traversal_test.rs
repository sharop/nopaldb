// Pruebas de integración del acumulador por traverser (sack).
//
// Fixture principal: la explosión de materiales de una mesa de taller — un
// árbol con cantidades por arista donde la cantidad pedida se multiplica al
// bajar por cada componente.

use std::collections::HashMap;

use nopaldb::{CycleMode, Edge, Graph, Node, NodeId, PropertyValue, Result, Truncation};

fn int(v: i64) -> PropertyValue {
    PropertyValue::Int(v)
}

fn float(v: f64) -> PropertyValue {
    PropertyValue::Float(v)
}

fn name(n: &str) -> PropertyValue {
    PropertyValue::String(n.into())
}

/// Mesa ──ContieneComponente{8}──▶ Tornillo   (ensamble)
///      ──ContieneComponente{2}──▶ Tornillo   (niveladores, arista paralela)
///      ──ContieneComponente{1}──▶ Barniz
///      ──ContieneComponente{4}──▶ Pata ──ContieneComponente{1}──▶ Regatón
///      ──RequiereOperacion{...}──▶ Lijado    (debe ignorarse)
struct BomFixture {
    graph: Graph,
    mesa: NodeId,
    tornillo: NodeId,
    barniz: NodeId,
    pata: NodeId,
    regaton: NodeId,
}

async fn bom_fixture() -> Result<BomFixture> {
    let graph = Graph::in_memory().await?;

    let mesa = graph.add_node(Node::new("Producto").with_property("name", name("Mesa"))).await?;
    let tornillo = graph.add_node(Node::new("Material").with_property("name", name("Tornillo"))).await?;
    let barniz = graph.add_node(Node::new("Material").with_property("name", name("Barniz"))).await?;
    let pata = graph.add_node(Node::new("Material").with_property("name", name("Pata"))).await?;
    let regaton = graph.add_node(Node::new("Material").with_property("name", name("Regatón"))).await?;
    let lijado = graph.add_node(Node::new("Operacion").with_property("name", name("Lijado"))).await?;

    graph.add_edge(Edge::new(mesa, tornillo, "ContieneComponente").with_property("cantidad", int(8))).await?;
    graph.add_edge(Edge::new(mesa, tornillo, "ContieneComponente").with_property("cantidad", int(2))).await?;
    graph.add_edge(Edge::new(mesa, barniz, "ContieneComponente").with_property("cantidad", int(1))).await?;
    graph.add_edge(Edge::new(mesa, pata, "ContieneComponente").with_property("cantidad", int(4))).await?;
    graph.add_edge(Edge::new(pata, regaton, "ContieneComponente").with_property("cantidad", int(1))).await?;
    graph.add_edge(
        Edge::new(mesa, lijado, "RequiereOperacion")
            .with_property("setup", int(15))
            .with_property("run", int(3)),
    ).await?;

    Ok(BomFixture { graph, mesa, tornillo, barniz, pata, regaton })
}

/// Agrupa (nodo → cantidades emitidas, ordenadas) para aserciones estables.
fn by_node(items: &[nopaldb::SackItem<f64>]) -> HashMap<NodeId, Vec<f64>> {
    let mut map: HashMap<NodeId, Vec<f64>> = HashMap::new();
    for item in items {
        map.entry(item.node).or_default().push(item.sack);
    }
    for v in map.values_mut() {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    map
}

#[tokio::test]
async fn test_bom_explosion_batch_of_10() -> Result<()> {
    let f = bom_fixture().await?;

    let r = f.graph
        .traverse(f.mesa)
        .sack(10.0)
        .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
        .emit()
        .await?;

    assert!(r.is_complete());
    assert_eq!(r.cycles_skipped, 0);

    let q = by_node(&r.items);
    // Tornillo aparece DOS veces: multiplicidad por camino (dos aristas paralelas)
    assert_eq!(q[&f.tornillo], vec![20.0, 80.0]);
    assert_eq!(q[&f.barniz], vec![10.0]);
    assert_eq!(q[&f.pata], vec![40.0]);
    // Regatón hereda el multiplicador acumulado del camino: 10 × 4 × 1
    assert_eq!(q[&f.regaton], vec![40.0]);
    // La operación de lijado NO entra: out_e filtra por tipo de arista
    assert_eq!(r.items.len(), 5);

    Ok(())
}

#[tokio::test]
async fn test_diamond_is_not_a_cycle() -> Result<()> {
    // A ──{2}──▶ B ──{4}──▶ D
    //   ──{3}──▶ C ──{5}──▶ D    → D dos veces: 8.0 y 15.0
    let graph = Graph::in_memory().await?;
    let a = graph.add_node(Node::new("N").with_property("name", name("A"))).await?;
    let b = graph.add_node(Node::new("N").with_property("name", name("B"))).await?;
    let c = graph.add_node(Node::new("N").with_property("name", name("C"))).await?;
    let d = graph.add_node(Node::new("N").with_property("name", name("D"))).await?;
    graph.add_edge(Edge::new(a, b, "E").with_property("q", float(2.0))).await?;
    graph.add_edge(Edge::new(a, c, "E").with_property("q", float(3.0))).await?;
    graph.add_edge(Edge::new(b, d, "E").with_property("q", float(4.0))).await?;
    graph.add_edge(Edge::new(c, d, "E").with_property("q", float(5.0))).await?;

    let r = graph
        .traverse(a)
        .sack(1.0)
        .repeat(|blk| blk.out_e("E").sack_mul_by("q"))
        .emit()
        .await?;

    assert!(r.is_complete());
    let q = by_node(&r.items);
    assert_eq!(q[&d], vec![8.0, 15.0]);

    Ok(())
}

#[tokio::test]
async fn test_merma_formula_with_closure() -> Result<()> {
    // q_hijo = q_padre × cantidad / (1 − merma)
    let graph = Graph::in_memory().await?;
    let a = graph.add_node(Node::new("N").with_property("name", name("A"))).await?;
    let b = graph.add_node(Node::new("N").with_property("name", name("B"))).await?;
    graph.add_edge(
        Edge::new(a, b, "E")
            .with_property("cantidad", int(18))
            .with_property("merma", float(0.1)),
    ).await?;

    let r = graph
        .traverse(a)
        .sack(10.0)
        .repeat(|blk| {
            blk.out_e("E").sack_by(|edge, q| {
                let cantidad = edge.properties.get("cantidad").and_then(|v| v.as_number()).unwrap_or(1.0);
                let merma = edge.properties.get("merma").and_then(|v| v.as_number()).unwrap_or(0.0);
                q * cantidad / (1.0 - merma)
            })
        })
        .emit()
        .await?;

    assert_eq!(r.items.len(), 1);
    assert!((r.items[0].sack - 200.0).abs() < 1e-9, "sack = {}", r.items[0].sack);

    Ok(())
}

#[tokio::test]
async fn test_emit_leaves_only() -> Result<()> {
    let f = bom_fixture().await?;

    let r = f.graph
        .traverse(f.mesa)
        .sack(10.0)
        .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
        .emit_leaves()
        .await?;

    assert!(r.is_complete());
    let q = by_node(&r.items);
    // Pata NO es hoja (tiene su propio componente); Regatón sí
    assert!(!q.contains_key(&f.pata));
    assert_eq!(q[&f.tornillo], vec![20.0, 80.0]);
    assert_eq!(q[&f.barniz], vec![10.0]);
    assert_eq!(q[&f.regaton], vec![40.0]);
    assert_eq!(r.items.len(), 4);

    Ok(())
}

#[tokio::test]
async fn test_cycle_errors_by_default() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let agenda = graph.add_node(Node::new("N").with_property("name", name("agenda"))).await?;
    let cuerpo = graph.add_node(Node::new("N").with_property("name", name("cuerpo"))).await?;
    graph.add_edge(Edge::new(agenda, cuerpo, "E").with_property("q", float(1.0))).await?;
    graph.add_edge(Edge::new(cuerpo, agenda, "E").with_property("q", float(1.0))).await?;

    let err = graph
        .traverse(agenda)
        .sack(1.0)
        .repeat(|b| b.out_e("E").sack_mul_by("q"))
        .emit()
        .await
        .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("cycle"), "mensaje: {msg}");
    assert!(msg.contains("agenda → cuerpo → agenda"), "mensaje: {msg}");

    Ok(())
}

#[tokio::test]
async fn test_cycle_skip_mode() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let a = graph.add_node(Node::new("N").with_property("name", name("A"))).await?;
    let b = graph.add_node(Node::new("N").with_property("name", name("B"))).await?;
    graph.add_edge(Edge::new(a, b, "E").with_property("q", float(2.0))).await?;
    graph.add_edge(Edge::new(b, a, "E").with_property("q", float(3.0))).await?;

    let r = graph
        .traverse(a)
        .sack(1.0)
        .on_cycle(CycleMode::Skip)
        .repeat(|blk| blk.out_e("E").sack_mul_by("q"))
        .emit()
        .await?;

    // El ciclo se descarta, no trunca: el recorrido terminó completo
    assert!(r.is_complete());
    assert_eq!(r.cycles_skipped, 1);
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].node, b);
    assert_eq!(r.items[0].sack, 2.0);

    Ok(())
}

async fn chain_of_5() -> Result<(Graph, NodeId)> {
    let graph = Graph::in_memory().await?;
    let names = ["n1", "n2", "n3", "n4", "n5"];
    let mut prev: Option<NodeId> = None;
    let mut first = None;
    for n in names {
        let node = graph.add_node(Node::new("N").with_property("name", name(n))).await?;
        if let Some(p) = prev {
            graph.add_edge(Edge::new(p, node, "E").with_property("q", float(2.0))).await?;
        } else {
            first = Some(node);
        }
        prev = Some(node);
    }
    Ok((graph, first.unwrap()))
}

#[tokio::test]
async fn test_max_depth_truncation_is_reported() -> Result<()> {
    let (graph, start) = chain_of_5().await?;

    let r = graph
        .traverse(start)
        .sack(1.0)
        .max_depth(2)
        .repeat(|b| b.out_e("E").sack_mul_by("q"))
        .emit()
        .await?;

    assert_eq!(r.truncated, Some(Truncation::MaxDepth));
    assert!(!r.is_complete());
    assert_eq!(r.items.len(), 2);
    assert!(r.items.iter().all(|i| i.depth <= 2));

    Ok(())
}

#[tokio::test]
async fn test_max_nodes_truncation_is_reported() -> Result<()> {
    let (graph, start) = chain_of_5().await?;

    let r = graph
        .traverse(start)
        .sack(1.0)
        .max_nodes(2)
        .repeat(|b| b.out_e("E").sack_mul_by("q"))
        .emit()
        .await?;

    assert_eq!(r.truncated, Some(Truncation::MaxNodes));
    assert!(!r.is_complete());
    assert_eq!(r.items.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_generic_sack_type() -> Result<()> {
    // El acumulador es genérico: (cantidad, saltos) en una tupla
    let (graph, start) = chain_of_5().await?;

    let r = graph
        .traverse(start)
        .sack((1.0f64, 0usize))
        .repeat(|b| {
            b.out_e("E").sack_by(|edge, (q, hops)| {
                let factor = edge.properties.get("q").and_then(|v| v.as_number()).unwrap_or(1.0);
                (q * factor, hops + 1)
            })
        })
        .emit()
        .await?;

    assert!(r.is_complete());
    assert_eq!(r.items.len(), 4);
    let last = r.items.iter().max_by_key(|i| i.depth).unwrap();
    assert_eq!(last.sack, (16.0, 4));

    Ok(())
}

#[tokio::test]
async fn test_parent_links_rebuild_tree() -> Result<()> {
    let f = bom_fixture().await?;

    let r = f.graph
        .traverse(f.mesa)
        .sack(10.0)
        .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
        .emit()
        .await?;

    // Reconstrucción del árbol por índices (el snippet del doc del módulo)
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); r.items.len()];
    let mut roots = Vec::new();
    for (i, item) in r.items.iter().enumerate() {
        match item.parent {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }

    // Raíces = hijos directos de Mesa: Tornillo×2, Barniz, Pata
    assert_eq!(roots.len(), 4);
    let pata_idx = r.items.iter().position(|i| i.node == f.pata).unwrap();
    let regaton_idx = r.items.iter().position(|i| i.node == f.regaton).unwrap();

    // Regatón cuelga exactamente del ítem Pata, no de un NodeId ambiguo
    assert_eq!(r.items[regaton_idx].parent, Some(pata_idx));
    assert_eq!(children[pata_idx], vec![regaton_idx]);

    // Invariante con bloques de un salto: el padre está un nivel arriba
    for item in &r.items {
        if let Some(p) = item.parent {
            assert_eq!(r.items[p].depth, item.depth - 1);
        }
    }

    // via_edge distingue las dos aristas paralelas Mesa→Tornillo
    let mut cantidades = Vec::new();
    for item in r.items.iter().filter(|i| i.node == f.tornillo) {
        let edge = f.graph.get_edge(item.via_edge.unwrap()).await?;
        cantidades.push(edge.properties.get("cantidad").and_then(|v| v.as_number()).unwrap());
    }
    cantidades.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(cantidades, vec![2.0, 8.0]);

    Ok(())
}

#[tokio::test]
async fn test_emit_leaves_parent_is_none() -> Result<()> {
    let f = bom_fixture().await?;

    let r = f.graph
        .traverse(f.mesa)
        .sack(10.0)
        .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
        .emit_leaves()
        .await?;

    // Reporte plano: ningún intermedio se emite, así que no hay padres
    assert!(r.items.iter().all(|i| i.parent.is_none()));
    // Pero via_edge sí identifica por dónde llegó cada hoja
    assert!(r.items.iter().all(|i| i.via_edge.is_some()));

    Ok(())
}

#[tokio::test]
async fn test_edge_property_filter() -> Result<()> {
    // filter_edge permite seleccionar por propiedad de arista, no solo tipo
    let f = bom_fixture().await?;

    let r = f.graph
        .traverse(f.mesa)
        .sack(1.0)
        .repeat(|b| {
            b.out_e("ContieneComponente")
                .filter_edge(|e| e.properties.get("cantidad").and_then(|v| v.as_number()).unwrap_or(0.0) > 1.0)
                .sack_mul_by("cantidad")
        })
        .emit()
        .await?;

    let q = by_node(&r.items);
    // Solo sobreviven las aristas con cantidad > 1: Tornillo{8}, Tornillo{2}
    // y Pata{4}; Barniz{1} y Regatón{1} quedan fuera
    assert_eq!(q[&f.tornillo], vec![2.0, 8.0]);
    assert_eq!(q[&f.pata], vec![4.0]);
    assert!(!q.contains_key(&f.barniz));
    assert!(!q.contains_key(&f.regaton));
    assert_eq!(r.items.len(), 3);

    Ok(())
}