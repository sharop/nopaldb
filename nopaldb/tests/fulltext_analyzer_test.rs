// tests/fulltext_analyzer_test.rs
//
// Per-index full-text analyzer (#74): language stemmer, stop words and accent
// folding, configured at `create index` and applied to documents and queries
// alike; persisted next to the tantivy files; default untouched.
// Fictional Spanish corpus: the card catalogue of a cookbook library.

use nopaldb::index::{FullTextAnalyzer, IndexOptions, IndexType};
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

const FICHAS: &[(&str, &str)] = &[
    ("f1", "Clasificación de los catálogos de recetas por región"),
    ("f2", "El catálogo de postres de la biblioteca"),
    ("f3", "Préstamos y devoluciones de libros prestados"),
    ("f4", "Inventario de utensilios de cocina"),
];

async fn corpus(graph: &Graph) -> std::collections::HashMap<String, NodeId> {
    let mut ids = std::collections::HashMap::new();
    for (name, body) in FICHAS {
        let node = Node::new("Ficha").with_property("name", s(name)).with_property("body", s(body));
        ids.insert(name.to_string(), graph.add_node(node).await.unwrap());
    }
    ids
}

/// The text branch of hybrid search, the path users query full-text through.
async fn fulltext(graph: &Graph, text: &str) -> Vec<NodeId> {
    let mut q = HybridQuery::new();
    q.text = Some(text.to_string());
    q.k = 50;
    graph.search_hybrid(q).await.unwrap().into_iter().map(|h| h.node_id).collect()
}

fn names(ids: &std::collections::HashMap<String, NodeId>, hits: &[NodeId]) -> Vec<String> {
    let mut out: Vec<String> = ids.iter().filter(|(_, id)| hits.contains(id)).map(|(n, _)| n.clone()).collect();
    out.sort();
    out
}

#[tokio::test]
async fn spanish_analyzer_folds_accents_and_stems() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let ids = corpus(&graph).await;
    let options = IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) };
    graph.create_index_with("Ficha", "body", IndexType::FullText, options).await.unwrap();

    // Typed without the accent, singular vs plural, different inflection.
    assert_eq!(names(&ids, &fulltext(&graph, "clasificacion").await), vec!["f1"]);
    assert_eq!(names(&ids, &fulltext(&graph, "catalogos").await), vec!["f1", "f2"]);
    assert_eq!(names(&ids, &fulltext(&graph, "catálogo").await), vec!["f1", "f2"]);
    // Stemming is heuristic (Snowball reads `préstamos` as a verb form and
    // `préstamo` as a noun, so those two do NOT share a stem); a regular
    // plural does.
    assert_eq!(names(&ids, &fulltext(&graph, "libro").await), vec!["f3"]);
    // Stop words never reach the index: a query made only of them finds nothing.
    assert!(fulltext(&graph, "de").await.is_empty());
    assert!(fulltext(&graph, "los").await.is_empty());
}

#[tokio::test]
async fn default_analyzer_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let ids = corpus(&graph).await;
    graph.create_index("Ficha", "body", IndexType::FullText).await.unwrap();

    // Exactly what 0.5.12 did: accents and inflection matter, stop words are indexed.
    assert!(fulltext(&graph, "clasificacion").await.is_empty());
    assert_eq!(names(&ids, &fulltext(&graph, "clasificación").await), vec!["f1"]);
    assert_eq!(names(&ids, &fulltext(&graph, "catalogos").await), Vec::<String>::new());
    assert_eq!(names(&ids, &fulltext(&graph, "catálogos").await), vec!["f1"]);
    assert_eq!(fulltext(&graph, "de").await.len(), 4);

    let info = graph.describe_index("Ficha_body").await.unwrap();
    assert_eq!(info.analyzer, Some(FullTextAnalyzer::default()));
}

#[tokio::test]
async fn folding_without_language_only_folds() {
    let graph = Graph::in_memory().await.unwrap();
    let ids = corpus(&graph).await;
    let analyzer = FullTextAnalyzer { ascii_folding: true, ..Default::default() };
    graph
        .create_index_with("Ficha", "body", IndexType::FullText, IndexOptions { analyzer: Some(analyzer) })
        .await
        .unwrap();
    assert_eq!(names(&ids, &fulltext(&graph, "clasificacion").await), vec!["f1"]);
    // No stemming: the plural does not find the singular.
    assert_eq!(names(&ids, &fulltext(&graph, "catalogos").await), vec!["f1"]);
    // No stop words: they are indexed.
    assert_eq!(fulltext(&graph, "de").await.len(), 4);
}

#[tokio::test]
async fn analyzer_survives_reopen_via_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        corpus(&graph).await;
        let options = IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) };
        graph.create_index_with("Ficha", "body", IndexType::FullText, options).await.unwrap();
        graph.close().await.unwrap();
    }
    let sidecar = dir.path().join("indexes").join("fulltext_Ficha_body").join("analyzer.json");
    let sidecar = if sidecar.exists() {
        sidecar
    } else {
        // Locate it wherever the index manager keeps its base path.
        walkdir(dir.path()).into_iter().find(|p| p.ends_with("analyzer.json")).expect("sidecar written")
    };
    assert!(std::fs::read_to_string(&sidecar).unwrap().contains("\"spanish\""));

    let graph = Graph::open(dir.path()).await.unwrap();
    let ids: std::collections::HashMap<String, NodeId> = graph
        .get_nodes_by_label("Ficha")
        .await
        .unwrap()
        .into_iter()
        .map(|n| (n.properties["name"].as_str().unwrap().to_string(), n.id))
        .collect();
    assert_eq!(names(&ids, &fulltext(&graph, "clasificacion").await), vec!["f1"]);
    let info = graph.describe_index("Ficha_body").await.unwrap();
    assert_eq!(info.analyzer, Some(FullTextAnalyzer::for_language("spanish")));
}

#[tokio::test]
async fn index_without_sidecar_opens_as_default() {
    // A database written before 0.5.13 has a tantivy directory and no
    // analyzer.json. Simulate it by deleting the sidecar.
    let dir = tempfile::tempdir().unwrap();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        corpus(&graph).await;
        graph.create_index("Ficha", "body", IndexType::FullText).await.unwrap();
        graph.close().await.unwrap();
    }
    for p in walkdir(dir.path()) {
        if p.ends_with("analyzer.json") {
            std::fs::remove_file(p).unwrap();
        }
    }
    let graph = Graph::open(dir.path()).await.unwrap();
    let info = graph.describe_index("Ficha_body").await.unwrap();
    assert_eq!(info.analyzer, Some(FullTextAnalyzer::default()));
    assert_eq!(fulltext(&graph, "clasificación").await.len(), 1);
}

#[tokio::test]
async fn changing_the_analyzer_means_drop_and_create() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let ids = corpus(&graph).await;
    graph.create_index("Ficha", "body", IndexType::FullText).await.unwrap();

    let options = IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) };
    let err = graph
        .create_index_with("Ficha", "body", IndexType::FullText, options.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("drop index Ficha_body"), "{err}");

    // Until 0.5.13 the tantivy directory outlived the index and the re-create
    // reopened the old schema. Now drop removes it and the new analyzer applies.
    graph.drop_index("Ficha_body").await.unwrap();
    graph.create_index_with("Ficha", "body", IndexType::FullText, options).await.unwrap();
    assert_eq!(names(&ids, &fulltext(&graph, "clasificacion").await), vec!["f1"]);
    let info = graph.describe_index("Ficha_body").await.unwrap();
    assert_eq!(info.analyzer, Some(FullTextAnalyzer::for_language("spanish")));

    // Dropping removes the directory; nothing of the index is left on disk.
    graph.drop_index("Ficha_body").await.unwrap();
    assert!(!walkdir(dir.path()).iter().any(|p| p.to_string_lossy().contains("fulltext_Ficha_body")));
}

#[tokio::test]
async fn analyzer_is_rejected_for_other_index_types_and_bad_options() {
    let graph = Graph::in_memory().await.unwrap();
    let options = IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) };
    let err = graph.create_index_with("Ficha", "name", IndexType::Hash, options).await.unwrap_err().to_string();
    assert!(err.contains("only applies to full-text"), "{err}");

    let bad = IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("klingon")) };
    let err = graph.create_index_with("Ficha", "body", IndexType::FullText, bad).await.unwrap_err().to_string();
    assert!(err.contains("unknown language"), "{err}");
    // Nothing half-created.
    assert!(graph.describe_index("Ficha_body").await.is_none());
}

#[tokio::test]
async fn nql_with_clause_configures_the_analyzer() {
    let graph = Graph::in_memory().await.unwrap();
    let ids = corpus(&graph).await;
    graph
        .execute_nql(r#"create index on Ficha(body) type fulltext with (language = "spanish", stopwords = false)"#)
        .await
        .unwrap();
    let info = graph.describe_index("Ficha_body").await.unwrap();
    assert_eq!(
        info.analyzer,
        Some(FullTextAnalyzer { language: Some("spanish".into()), stemming: true, stopwords: false, ascii_folding: true })
    );
    assert_eq!(names(&ids, &fulltext(&graph, "catalogos").await), vec!["f1", "f2"]);
    // stopwords = false: they are indexed.
    assert_eq!(fulltext(&graph, "de").await.len(), 4);

    // The clause is only for full-text, and typos are errors, not silence.
    let err = graph.execute_nql(r#"create index on Ficha(name) type hash with (language = "spanish")"#).await.unwrap_err();
    assert!(err.to_string().contains("only applies to `type fulltext`"), "{err}");
    let err = graph
        .execute_nql(r#"create index on Ficha(name) type fulltext with (ascii_fold = true)"#)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown option `ascii_fold`"), "{err}");
}

/// Every file under `root`, recursively.
fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
    }
    out
}
