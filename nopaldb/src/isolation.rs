// src/isolation.rs

/// Niveles de aislamiento ACID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Permite dirty reads, non-repeatable reads, phantom reads
    /// Más rápido, menos garantías
    ReadUncommitted,

    /// Previene dirty reads
    /// Permite non-repeatable reads, phantom reads
    /// Default en muchas DBs (PostgreSQL, MySQL)
    ReadCommitted,

    /// Previene dirty reads y non-repeatable reads
    /// Permite phantom reads
    /// Usa snapshots
    RepeatableRead,

    /// Previene todos los problemas
    /// Máxima consistencia, mínima performance
    /// Usa detección de conflictos
    Serializable,
}

impl Default for IsolationLevel {
    fn default() -> Self {
        IsolationLevel::ReadCommitted
    }
}

impl IsolationLevel {
    /// ¿Este nivel permite ver cambios no commiteados?
    pub fn allows_dirty_reads(&self) -> bool {
        matches!(self, IsolationLevel::ReadUncommitted)
    }

    /// ¿Este nivel requiere snapshot?
    pub fn requires_snapshot(&self) -> bool {
        matches!(
            self,
            IsolationLevel::RepeatableRead | IsolationLevel::Serializable
        )
    }

    /// ¿Este nivel requiere tracking de conflictos?
    pub fn requires_conflict_detection(&self) -> bool {
        matches!(self, IsolationLevel::Serializable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isolation_level_properties() {
        assert!(IsolationLevel::ReadUncommitted.allows_dirty_reads());
        assert!(!IsolationLevel::ReadCommitted.allows_dirty_reads());

        assert!(IsolationLevel::RepeatableRead.requires_snapshot());
        assert!(!IsolationLevel::ReadCommitted.requires_snapshot());

        assert!(IsolationLevel::Serializable.requires_conflict_detection());
        assert!(!IsolationLevel::RepeatableRead.requires_conflict_detection());
    }
}



#[tokio::test]
async fn test_read_committed_basic() {
    let graph = Graph::in_memory().await.unwrap();

    // Tx1: Insert Alice
    let mut tx1 = graph.begin_transaction().await.unwrap();
    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(1000));
    let alice_id = tx1.add_node(alice).unwrap();
    tx1.commit().await.unwrap();

    // Tx2: Read Alice
    let tx2 = graph.begin_transaction()
        .await.unwrap()
        .with_isolation(IsolationLevel::ReadCommitted);

    let node = tx2.get_node(alice_id).await.unwrap();
    assert_eq!(node.properties.get("balance"), Some(&PropertyValue::Int(1000)));
}

#[tokio::test]
async fn test_repeatable_read_prevents_modifications() {
    let graph = Graph::in_memory().await.unwrap();

    // Setup: Create Alice
    let mut tx_setup = graph.begin_transaction().await.unwrap();
    let alice = Node::new("Person")
        .with_property("balance", PropertyValue::Int(1000));
    let alice_id = tx_setup.add_node(alice).unwrap();
    tx_setup.commit().await.unwrap();

    // Tx1: Start with RepeatableRead
    let tx1 = graph.begin_transaction()
        .await.unwrap()
        .with_isolation(IsolationLevel::RepeatableRead);

    // Read snapshot timestamp
    let snapshot = tx1.timestamp;

    // First read
    let node1 = tx1.get_node(alice_id).await.unwrap();
    assert_eq!(node1.properties.get("balance"), Some(&PropertyValue::Int(1000)));

    // Tx2: Modify Alice and commit
    let mut tx2 = graph.begin_transaction().await.unwrap();
    let mut alice_modified = Node::new("Person")
        .with_property("balance", PropertyValue::Int(500));
    alice_modified.id = alice_id;
    tx2.add_node(alice_modified).unwrap();
    tx2.commit().await.unwrap();

    // Tx1: Try to read again - should fail (node modified after snapshot)
    let result = tx1.get_node(alice_id).await;
    assert!(result.is_err(), "RepeatableRead should detect modification");
}

#[tokio::test]
async fn test_serializable_detects_write_conflict() {
    let graph = Graph::in_memory().await.unwrap();

    // Setup
    let mut tx_setup = graph.begin_transaction().await.unwrap();
    let alice = Node::new("Person")
        .with_property("balance", PropertyValue::Int(1000));
    let alice_id = tx_setup.add_node(alice).unwrap();
    tx_setup.commit().await.unwrap();

    // Tx1: Read with Serializable
    let mut tx1 = graph.begin_transaction()
        .await.unwrap()
        .with_isolation(IsolationLevel::Serializable);

    let _node = tx1.get_node(alice_id).await.unwrap();

    // Tx2: Modify and commit
    let mut tx2 = graph.begin_transaction().await.unwrap();
    let mut alice_modified = Node::new("Person")
        .with_property("balance", PropertyValue::Int(500));
    alice_modified.id = alice_id;
    tx2.add_node(alice_modified).unwrap();
    tx2.commit().await.unwrap();

    // Tx1: Try to commit - should fail (read-write conflict)
    let mut alice_tx1 = Node::new("Person")
        .with_property("balance", PropertyValue::Int(800));
    alice_tx1.id = alice_id;
    tx1.add_node(alice_tx1).unwrap();

    let result = tx1.commit().await;
    assert!(result.is_err(), "Serializable should detect conflict");
}