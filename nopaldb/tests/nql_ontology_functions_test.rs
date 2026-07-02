// tests/nql_ontology_functions_test.rs
//
// Integration tests for NQL ontology predicates:
//   instanceOf(var, "ClassName") — Step 7

use nopaldb::{PropertyValue, Result};
use nopaldb::index::{IndexManager, IndexType, TaxonomyIndex};

// ---------------------------------------------------------------------------
// Test 1 — instanceOf filters by taxonomy (happy path)
//
// Since the NQL predicate requires a registered taxonomy index, we test
// the component functions directly (TaxonomyIndex + evaluate logic)
// rather than firing full NQL queries (which require taxonomy persistence
// wired end-to-end with the graph's index_manager).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_taxonomy_is_subclass_of_label() -> Result<()> {
    let mut tax = TaxonomyIndex::new();

    let fe_id   = uuid::Uuid::new_v4();
    let acc_id  = uuid::Uuid::new_v4();
    let sa_id   = uuid::Uuid::new_v4();

    tax.register_class(fe_id, "FinancialEntity");
    tax.register_class(acc_id, "Account");
    tax.register_class(sa_id, "SavingsAccount");

    // Account ⊑ FinancialEntity
    tax.add_subclass(fe_id, acc_id)?;
    // SavingsAccount ⊑ Account
    tax.add_subclass(acc_id, sa_id)?;

    // SavingsAccount ⊑ FinancialEntity (transitively)
    let fe_id_found = tax.find_by_label("FinancialEntity").unwrap();
    assert!(tax.is_subclass_of_label("SavingsAccount", fe_id_found),
        "SavingsAccount must be subclass of FinancialEntity");

    // Account ⊑ FinancialEntity (directly)
    assert!(tax.is_subclass_of_label("Account", fe_id_found),
        "Account must be subclass of FinancialEntity");

    // Document is NOT in taxonomy → false
    assert!(!tax.is_subclass_of_label("Document", fe_id_found),
        "Document is not in taxonomy, must return false");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2 — subClassOf check on Class nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_subclassof_class_node_check() -> Result<()> {
    let mut tax = TaxonomyIndex::new();

    let entity_id = uuid::Uuid::new_v4();
    let person_id = uuid::Uuid::new_v4();
    let employee_id = uuid::Uuid::new_v4();

    tax.register_class(entity_id, "Entity");
    tax.register_class(person_id, "Person");
    tax.register_class(employee_id, "Employee");

    tax.add_subclass(entity_id, person_id)?;   // Person ⊑ Entity
    tax.add_subclass(person_id, employee_id)?; // Employee ⊑ Person

    let entity = tax.find_by_label("Entity").unwrap();

    // Employee ⊑ Entity (transitively)
    assert!(tax.is_subclass_of_label("Employee", entity));
    // Person ⊑ Entity
    assert!(tax.is_subclass_of_label("Person", entity));
    // Entity is NOT a subclass of itself
    assert!(!tax.is_subclass_of_label("Entity", entity));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3 — unknown class returns false (no panic)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_unknown_class_returns_false() -> Result<()> {
    let mut tax = TaxonomyIndex::new();
    let known_id = uuid::Uuid::new_v4();
    tax.register_class(known_id, "KnownClass");

    // Parent class that doesn't exist → find_by_label returns None
    assert!(!tax.is_subclass_of_label("AnyLabel", uuid::Uuid::new_v4()));

    // Child label not in taxonomy
    let parent = tax.find_by_label("KnownClass").unwrap();
    assert!(!tax.is_subclass_of_label("Unknown", parent));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4 — as_taxonomy() downcast on TaxonomyIndex via Index trait
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_as_taxonomy_downcast() -> Result<()> {
    use nopaldb::index::{HashIndex, Index as IndexTrait};

    let tax = TaxonomyIndex::new();
    let boxed: Box<dyn IndexTrait> = Box::new(tax);

    // Should downcast successfully
    assert!(boxed.as_taxonomy().is_some(), "TaxonomyIndex must return Some from as_taxonomy()");

    // HashIndex should return None
    let hash: Box<dyn IndexTrait> = Box::new(HashIndex::new());
    assert!(hash.as_taxonomy().is_none(), "HashIndex must return None from as_taxonomy()");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5 — get_taxonomy_sync returns a clone when taxonomy is registered
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_get_taxonomy_sync_via_index_manager() -> Result<()> {
    let manager = IndexManager::new(None);
    manager.create_index("FinancialEntity", "label", IndexType::Taxonomy).await?;

    // Insert a class
    manager.insert(
        "FinancialEntity_label",
        PropertyValue::String("Account".to_string()),
        uuid::Uuid::new_v4(),
    ).await?;

    // get_taxonomy_sync should return Some
    let tax = manager.get_taxonomy_sync();
    assert!(tax.is_some(), "get_taxonomy_sync must return Some after taxonomy index created");

    let tax = tax.unwrap();
    assert!(tax.find_by_label("Account").is_some());

    Ok(())
}
