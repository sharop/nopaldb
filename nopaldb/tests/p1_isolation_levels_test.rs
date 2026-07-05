#![cfg(feature = "full-isolation")]

use nopaldb::{Edge, Graph, IsolationLevel, Node, PropertyValue};

#[tokio::test]
async fn test_read_committed_sees_latest_committed_value() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let alice_id = setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;
    setup.commit().await?;

    let tx_rc = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::ReadCommitted);
    let first_read = tx_rc.get_node(alice_id).await?;
    assert_eq!(
        first_read.properties.get("balance"),
        Some(&PropertyValue::Int(1000))
    );

    let mut tx_update = graph.begin_transaction().await?;
    let mut alice_updated = Node::new("Person").with_property("balance", PropertyValue::Int(500));
    alice_updated.id = alice_id;
    tx_update.add_node(alice_updated).await?;
    tx_update.commit().await?;

    let second_read = tx_rc.get_node(alice_id).await?;
    assert_eq!(
        second_read.properties.get("balance"),
        Some(&PropertyValue::Int(500))
    );

    Ok(())
}

#[tokio::test]
async fn test_repeatable_read_returns_stable_snapshot() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let alice_id = setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;
    setup.commit().await?;

    let tx_rr = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::RepeatableRead);

    let first_read = tx_rr.get_node(alice_id).await?;
    assert_eq!(
        first_read.properties.get("balance"),
        Some(&PropertyValue::Int(1000))
    );

    let mut tx_update = graph.begin_transaction().await?;
    let mut alice_updated = Node::new("Person").with_property("balance", PropertyValue::Int(500));
    alice_updated.id = alice_id;
    tx_update.add_node(alice_updated).await?;
    tx_update.commit().await?;

    let second_read = tx_rr.get_node(alice_id).await?;
    assert_eq!(
        second_read.properties.get("balance"),
        Some(&PropertyValue::Int(1000))
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_conflict_at_commit() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let alice_id = setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;
    setup.commit().await?;

    let mut tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let _ = tx_serializable.get_node(alice_id).await?;

    let mut tx_update = graph.begin_transaction().await?;
    let mut alice_updated = Node::new("Person").with_property("balance", PropertyValue::Int(500));
    alice_updated.id = alice_id;
    tx_update.add_node(alice_updated).await?;
    tx_update.commit().await?;

    let mut alice_local = Node::new("Person").with_property("balance", PropertyValue::Int(800));
    alice_local.id = alice_id;
    tx_serializable.add_node(alice_local).await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit after concurrent modification"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_delete_conflict() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let alice_id = setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;
    setup.commit().await?;

    let mut tx_delete = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let _ = tx_delete.get_node(alice_id).await?;

    let mut tx_update = graph.begin_transaction().await?;
    let mut alice_updated = Node::new("Person").with_property("balance", PropertyValue::Int(500));
    alice_updated.id = alice_id;
    tx_update.add_node(alice_updated).await?;
    tx_update.commit().await?;

    tx_delete.delete_node(alice_id)?;
    let commit_result = tx_delete.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable delete should fail after concurrent modification"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_phantom_insert_by_label() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into())),
        )
        .await?;
    setup.commit().await?;

    let tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let baseline = tx_serializable.get_nodes_by_label("Person").await?;
    assert_eq!(baseline.len(), 1);

    let mut tx_insert = graph.begin_transaction().await?;
    tx_insert
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Bob".into())),
        )
        .await?;
    tx_insert.commit().await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit when predicate result changes (phantom insert)"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_phantom_insert_on_global_scan() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into())),
        )
        .await?;
    setup.commit().await?;

    let tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let baseline = tx_serializable.get_all_nodes().await?;
    assert_eq!(baseline.len(), 1);

    let mut tx_insert = graph.begin_transaction().await?;
    tx_insert
        .add_node(
            Node::new("Company")
                .with_property("name", PropertyValue::String("Acme".into())),
        )
        .await?;
    tx_insert.commit().await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit when global scan result changes (phantom insert)"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_phantom_insert_on_single_hop_pattern() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let person_a = setup
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let company = setup
        .add_node(Node::new("Company").with_property("name", PropertyValue::String("Acme".into())))
        .await?;
    setup.add_edge(Edge::new(person_a, company, "OWNS"))?;
    setup.commit().await?;

    let tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let baseline = tx_serializable
        .get_pattern_pairs("Person", "OWNS", "Company")
        .await?;
    assert_eq!(baseline.len(), 1);

    let mut tx_insert = graph.begin_transaction().await?;
    let person_b = tx_insert
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    tx_insert.add_edge(Edge::new(person_b, company, "OWNS"))?;
    tx_insert.commit().await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit when single-hop pattern result changes"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_phantom_insert_on_two_hop_pattern() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    let person_a = setup
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let company_a = setup
        .add_node(Node::new("Company").with_property("name", PropertyValue::String("Acme".into())))
        .await?;
    let country = setup
        .add_node(Node::new("Country").with_property("name", PropertyValue::String("MX".into())))
        .await?;
    setup.add_edge(Edge::new(person_a, company_a, "OWNS"))?;
    setup.add_edge(Edge::new(company_a, country, "LOCATED_IN"))?;
    setup.commit().await?;

    let tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let baseline = tx_serializable
        .get_pattern_triples_two_hop("Person", "OWNS", "Company", "LOCATED_IN", "Country")
        .await?;
    assert_eq!(baseline.len(), 1);

    let mut tx_insert = graph.begin_transaction().await?;
    let person_b = tx_insert
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    tx_insert.add_edge(Edge::new(person_b, company_a, "OWNS"))?;
    tx_insert.commit().await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit when two-hop pattern result changes"
    );

    Ok(())
}

#[tokio::test]
async fn test_serializable_detects_phantom_insert_on_label_property_predicate() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let mut setup = graph.begin_transaction().await?;
    setup
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("status", PropertyValue::String("active".into())),
        )
        .await?;
    setup.commit().await?;

    let tx_serializable = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let baseline = tx_serializable
        .get_nodes_by_label_and_property(
            "Person",
            "status",
            &PropertyValue::String("active".into()),
        )
        .await?;
    assert_eq!(baseline.len(), 1);

    let mut tx_insert = graph.begin_transaction().await?;
    tx_insert
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Bob".into()))
                .with_property("status", PropertyValue::String("active".into())),
        )
        .await?;
    tx_insert.commit().await?;

    let commit_result = tx_serializable.commit().await;
    assert!(
        commit_result.is_err(),
        "Serializable should fail commit when label+property predicate result changes"
    );

    Ok(())
}
