// tests/synthetic_offshore_e2e_test.rs
//
// Step 12 — E2E: Synthetic Offshore Network pipeline
//
// Pipeline completo:
//   Turtle OWL → import_turtle() → Graph
//   → add_edge() para relaciones entre individuos
//   → NQL multi-hop + property filters
//   → ELReasoner CR1 classify_all() → transitividad ShellCompany ⊑ LegalEntity
//
// NOTA sobre label matching en NQL:
//   NQL filtra por label exacto. `(e:OffshoreEntity)` NO incluye subclases.
//   La inferencia transitiva es responsabilidad del ELReasoner, no del executor.
//   Las queries NQL aquí reflejan el comportamiento real del sistema.
//
// Run: cargo test --features core,owl-import,reasoner --test synthetic_offshore_e2e_test

#[cfg(all(feature = "owl-import", feature = "reasoner"))]
mod tests {
    use nopaldb::graph::Graph;
    use nopaldb::reasoner::ELReasoner;
    use nopaldb::types::{Edge, NodeKind, PropertyValue};

    static TTL: &str = include_str!("fixtures/synthetic_offshore.ttl");

    /// Importa el TTL y agrega aristas entre individuos.
    async fn build_graph() -> Graph {
        let graph = Graph::in_memory().await.unwrap();

        let report = graph.import_turtle(TTL).await.unwrap();

        // Clases: LegalEntity, OffshoreEntity, ShellCompany, Officer, Intermediary, Jurisdiction
        assert_eq!(report.classes_added, 6);
        // OffshoreEntity ⊑ LegalEntity, ShellCompany ⊑ OffshoreEntity
        assert_eq!(report.subclass_edges_added, 2);
        // 4 jurisdicciones + 2 intermediarios + 4 entidades + 4 oficiales = 14
        assert_eq!(report.instances_added, 14);

        let find = |label: &str, name_val: &str| {
            let graph = graph.clone();
            let label = label.to_string();
            let name_val = name_val.to_string();
            async move {
                graph
                    .get_nodes_by_label(&label)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|n| {
                        n.kind == NodeKind::Individual
                            && n.properties.get("name")
                                == Some(&PropertyValue::String(name_val.clone()))
                    })
                    .unwrap_or_else(|| panic!("Individual '{name_val}' not found"))
                    .id
            }
        };

        let bvi = find("Jurisdiction", "British Virgin Islands").await;
        let harbor_cay = find("Jurisdiction", "Harbor Cay").await;
        let seychelles = find("Jurisdiction", "Seychelles").await;

        let atlas = find("Intermediary", "Atlas Fiduciary Group").await;
        let alpha = find("Intermediary", "Alpha Services Ltd").await;

        let sunrise = find("OffshoreEntity", "Sunrise Holdings BVI").await;
        let bluewater = find("OffshoreEntity", "Bluewater Capital SA").await;
        let redmond = find("OffshoreEntity", "Redmond International Ltd").await;
        let zephyr = find("OffshoreEntity", "Zephyr Trust Seychelles").await;

        let alice = find("Officer", "Alice Novak").await;
        let bob = find("Officer", "Bob Okafor").await;
        let carol = find("Officer", "Carol Svensson").await;
        let dave = find("Officer", "Dave Muller").await;

        // registeredIn: entidad → jurisdicción (con propiedad risk)
        graph
            .add_edge(
                Edge::new(sunrise, bvi, "registeredIn")
                    .with_property("risk", PropertyValue::String("high".into())),
            )
            .await
            .unwrap();
        graph
            .add_edge(
                Edge::new(bluewater, harbor_cay, "registeredIn")
                    .with_property("risk", PropertyValue::String("high".into())),
            )
            .await
            .unwrap();
        graph
            .add_edge(
                Edge::new(redmond, bvi, "registeredIn")
                    .with_property("risk", PropertyValue::String("high".into())),
            )
            .await
            .unwrap();
        graph
            .add_edge(
                Edge::new(zephyr, seychelles, "registeredIn")
                    .with_property("risk", PropertyValue::String("medium".into())),
            )
            .await
            .unwrap();

        // intermediaryOf: intermediario → entidad
        graph
            .add_edge(Edge::new(atlas, sunrise, "intermediaryOf"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(atlas, bluewater, "intermediaryOf"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(atlas, redmond, "intermediaryOf"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(alpha, zephyr, "intermediaryOf"))
            .await
            .unwrap();

        // hasOfficer: entidad → oficial
        graph
            .add_edge(Edge::new(sunrise, alice, "hasOfficer"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(bluewater, alice, "hasOfficer"))
            .await
            .unwrap(); // alice en 2 entidades
        graph
            .add_edge(Edge::new(bluewater, bob, "hasOfficer"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(redmond, carol, "hasOfficer"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(zephyr, dave, "hasOfficer"))
            .await
            .unwrap();

        graph
    }

    // ── Test 1: import básico ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_import_creates_classes_and_individuals() {
        let graph = build_graph().await;

        // Clase OffshoreEntity existe como nodo Class
        let nodes = graph.get_nodes_by_label("OffshoreEntity").await.unwrap();
        assert!(
            nodes.iter().any(|n| n.kind == NodeKind::Class),
            "OffshoreEntity como Class"
        );

        // 4 individuos OffshoreEntity (todos tipados como OffshoreEntity en el TTL)
        let individuals: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Individual)
            .collect();
        assert_eq!(individuals.len(), 4, "4 entidades offshore");

        // Propiedades importadas
        let alice = graph
            .get_nodes_by_label("Officer")
            .await
            .unwrap()
            .into_iter()
            .find(|n| {
                n.kind == NodeKind::Individual
                    && n.properties.get("name")
                        == Some(&PropertyValue::String("Alice Novak".into()))
            })
            .unwrap();
        assert_eq!(
            alice.properties.get("country"),
            Some(&PropertyValue::String("Russia".into()))
        );
    }

    // ── Test 2: NQL — entidades en jurisdicciones de riesgo alto ──────────────

    #[tokio::test]
    async fn test_nql_high_risk_registrations() {
        let graph = build_graph().await;

        // Aristas registeredIn con risk="high" → Sunrise, Bluewater, Redmond (3)
        let r = graph
            .execute_nql(
                r#"find e.name, j.name
               from (e:OffshoreEntity) -[r:registeredIn {risk: "high"}]-> (j:Jurisdiction)
               order by e.name"#,
            )
            .await
            .unwrap();

        assert_eq!(r.rows.len(), 3, "3 registros con risk=high");
        let names: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| {
                if let Some(PropertyValue::String(s)) = row.get("e.name") {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(names.contains(&"Sunrise Holdings BVI".into()));
        assert!(names.contains(&"Bluewater Capital SA".into()));
        assert!(names.contains(&"Redmond International Ltd".into()));
    }

    // ── Test 3: NQL — multi-hop intermediario → entidad → jurisdicción ────────

    #[tokio::test]
    async fn test_nql_multihop_intermediary_to_jurisdiction() {
        let graph = build_graph().await;

        // Atlas Fiduciary Group gestiona 3 entidades; cada una en su jurisdicción
        let r = graph.execute_nql(
            r#"find i.name, e.name, j.name
               from (i:Intermediary) -[:intermediaryOf]-> (e:OffshoreEntity) -[:registeredIn]-> (j:Jurisdiction)
               where i.name = "Atlas Fiduciary Group"
               order by e.name"#
        ).await.unwrap();

        assert_eq!(r.rows.len(), 3, "Atlas gestiona 3 entidades");

        let jurisdictions: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| {
                if let Some(PropertyValue::String(s)) = row.get("j.name") {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(jurisdictions.contains(&"British Virgin Islands".into()));
        assert!(jurisdictions.contains(&"Harbor Cay".into()));
    }

    // ── Test 4: NQL — oficial con presencia en múltiples entidades ────────────

    #[tokio::test]
    async fn test_nql_officer_in_multiple_entities() {
        let graph = build_graph().await;

        // Alice aparece en Sunrise y Bluewater (cnt = 2)
        let r = graph
            .execute_nql(
                r#"find o.name, count(*) as cnt
               from (e:OffshoreEntity) -[:hasOfficer]-> (o:Officer)
               group by o.name
               having count(*) > 1"#,
            )
            .await
            .unwrap();

        assert!(
            !r.rows.is_empty(),
            "Al menos un oficial en múltiples entidades"
        );
        let names: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| {
                if let Some(PropertyValue::String(s)) = row.get("o.name") {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            names.contains(&"Alice Novak".into()),
            "Alice debe tener cnt > 1"
        );
    }

    // ── Test 5: NQL — count de entidades por jurisdicción ────────────────────

    #[tokio::test]
    async fn test_nql_count_by_jurisdiction() {
        let graph = build_graph().await;

        let r = graph
            .execute_nql(
                r#"find j.name, count(*) as cnt
               from (e:OffshoreEntity) -[:registeredIn]-> (j:Jurisdiction)
               group by j.name
               order by cnt"#,
            )
            .await
            .unwrap();

        // BVI: 2 (Sunrise, Redmond), Harbor Cay: 1, Seychelles: 1
        assert_eq!(r.rows.len(), 3, "3 jurisdicciones distintas");

        let bvi = r
            .rows
            .iter()
            .find(|row| {
                row.get("j.name") == Some(&PropertyValue::String("British Virgin Islands".into()))
            })
            .expect("BVI debe aparecer");

        // BVI tiene 2 entidades
        assert_eq!(bvi.get("cnt"), Some(&PropertyValue::Int(2)));
    }

    // ── Test 6: NQL — entidades activas con filtro de propiedad ──────────────

    #[tokio::test]
    async fn test_nql_active_offshore_entities() {
        let graph = build_graph().await;

        let r = graph
            .execute_nql(
                r#"find e.name, e.incorporated
               from (e:OffshoreEntity)
               where e.status = "active"
               order by e.name"#,
            )
            .await
            .unwrap();

        // Sunrise, Bluewater, Zephyr son active; Redmond es inactive
        assert_eq!(r.rows.len(), 3, "3 entidades activas");
        let names: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| {
                if let Some(PropertyValue::String(s)) = row.get("e.name") {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !names.contains(&"Redmond International Ltd".into()),
            "Redmond es inactive"
        );
    }

    // ── Test 7: NQL — aristas filtradas por propiedad (shell=true) ───────────

    #[tokio::test]
    async fn test_nql_shell_entities_in_high_risk_jurisdictions() {
        let graph = build_graph().await;

        // Entidades shell (shell=true) en jurisdicciones de riesgo alto
        // Nota: el importador TTL convierte "true"/"false" a PropertyValue::Bool
        let r = graph
            .execute_nql(
                r#"find e.name, j.name
               from (e:OffshoreEntity) -[r:registeredIn {risk: "high"}]-> (j:Jurisdiction)
               where e.shell = true
               order by e.name"#,
            )
            .await
            .unwrap();

        // Sunrise (shell=true, BVI, risk=high) — sola cumpla ambas condiciones
        assert_eq!(
            r.rows.len(),
            1,
            "Solo Sunrise es shell en jurisdiccion high-risk"
        );
        assert_eq!(
            r.rows[0].get("e.name"),
            Some(&PropertyValue::String("Sunrise Holdings BVI".into()))
        );
    }

    // ── Test 8: ELReasoner — transitividad ShellCompany ⊑ LegalEntity ────────

    #[tokio::test]
    async fn test_reasoner_class_hierarchy_transitivity() {
        let graph = build_graph().await;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1;

        let mut reasoner = ELReasoner::from_graph_at(&graph, ts).await.unwrap();
        reasoner.classify_all();

        let get_class = |label: &str| {
            let graph = graph.clone();
            let label = label.to_string();
            async move {
                graph
                    .get_nodes_by_label(&label)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|n| n.kind == NodeKind::Class)
                    .unwrap_or_else(|| panic!("Class '{label}' not found"))
                    .id
            }
        };

        let shell_id = get_class("ShellCompany").await;
        let offshore_id = get_class("OffshoreEntity").await;
        let legal_id = get_class("LegalEntity").await;

        // Declarado: ShellCompany ⊑ OffshoreEntity
        assert!(
            reasoner.is_subclass_of(shell_id, offshore_id),
            "ShellCompany ⊑ OffshoreEntity"
        );
        // Declarado: OffshoreEntity ⊑ LegalEntity
        assert!(
            reasoner.is_subclass_of(offshore_id, legal_id),
            "OffshoreEntity ⊑ LegalEntity"
        );
        // Inferido por CR1: ShellCompany ⊑ LegalEntity
        assert!(
            reasoner.is_subclass_of(shell_id, legal_id),
            "ShellCompany ⊑ LegalEntity (transitivo)"
        );
    }
}
