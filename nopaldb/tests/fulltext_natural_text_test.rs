// tests/fulltext_natural_text_test.rs
//
// #377: text that is not tantivy syntax is searched as plain words. Until
// 0.10.12 a question in natural language was a syntax error whenever it had
// an apostrophe (`chef's`: it opens a phrase), a colon (`field:`), quotes
// left open, loose operators or brackets: 31% of the questions of
// 2WikiMultiHopQA. Valid syntax is still a query.
//
// Fictional domain: a kitchen.

use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

const NOTES: &[(&str, &str)] = &[
    ("madre", "The mother of the chef opened her kitchen in spring"),
    ("salsa", "A salsa of roasted tomatoes and chile"),
    ("mole", "Mole negro takes three days in this kitchen"),
    ("casa", "The house cook keeps the old recipes"),
];

async fn kitchen() -> (Graph, Vec<(String, NodeId)>) {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::new();
    for (name, body) in NOTES {
        let node = Node::new("Nota").with_property("name", s(name)).with_property("body", s(body));
        ids.push((name.to_string(), g.add_node(node).await.unwrap()));
    }
    g.create_index("Nota", "body", nopaldb::index::IndexType::FullText).await.unwrap();
    (g, ids)
}

async fn search(g: &Graph, ids: &[(String, NodeId)], text: &str, label: bool) -> Vec<String> {
    let mut q = HybridQuery::new();
    q.text = Some(text.to_string());
    q.k = 10;
    if label {
        q.filter = Some(HybridFilter { label: Some("Nota".into()), props: vec![] });
    }
    let hits = g.search_hybrid(q).await.unwrap_or_else(|e| panic!("{text:?}: {e}"));
    let mut names: Vec<String> = ids
        .iter()
        .filter(|(_, id)| hits.iter().any(|h| h.node_id == *id))
        .map(|(n, _)| n.clone())
        .collect();
    names.sort();
    names
}

/// Text that tantivy rejects.
const NATURAL: &[&str] = &[
    // An apostrophe opens a phrase that never closes.
    "When did the chef's mother open her kitchen?",
    // A colon reads `of:` as a field; `NOT` would exclude `house`.
    "Who is the cook of: the AND OR NOT house?",
    // Loose operators, a caret, a tilde and brackets.
    "What's -the + salsa ^ of ~x [a TO b]?",
    // Quotes left open.
    "mole \"negro",
];

/// The notes that hold any word of `text`: what searching it as plain words
/// must find.
fn holding_any_word(text: &str) -> Vec<String> {
    let words = |t: &str| -> Vec<String> {
        t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let query = words(text);
    let mut names: Vec<String> = NOTES
        .iter()
        .filter(|(_, body)| words(body).iter().any(|w| query.contains(w)))
        .map(|(n, _)| n.to_string())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn natural_text_is_searched_as_its_words() {
    let (g, ids) = kitchen().await;
    for text in NATURAL {
        let expected = holding_any_word(text);
        assert!(!expected.is_empty(), "{text:?}");
        for label in [false, true] {
            assert_eq!(search(&g, &ids, text, label).await, expected, "{text:?}, label filter {label}");
        }
    }
    // `NOT house` is two words, not an exclusion.
    assert!(search(&g, &ids, NATURAL[1], false).await.contains(&"casa".to_string()));
}

#[tokio::test]
async fn valid_syntax_is_still_a_query() {
    let (g, ids) = kitchen().await;
    // A phrase and AND: only the note with both.
    assert_eq!(search(&g, &ids, r#"kitchen AND "mole negro""#, false).await, ["mole"]);
    // `-` excludes.
    assert_eq!(search(&g, &ids, "kitchen -mole", false).await, ["madre"]);
    // Words with no syntax: any of them.
    assert_eq!(search(&g, &ids, "salsa house", false).await, ["casa", "salsa"]);
}

#[tokio::test]
async fn nql_hybrid_takes_natural_text() {
    let (g, _) = kitchen().await;
    let r = g
        .execute_nql(r#"find n.name from (n:Nota) where hybrid(n, text = "When did the chef's mother open her kitchen?") order by n.name"#)
        .await
        .unwrap();
    let names: Vec<PropertyValue> = r.rows().iter().map(|row| row["n.name"].clone()).collect();
    assert_eq!(names, [s("casa"), s("madre"), s("mole")]);
}
