// tests/nql_string_escapes_test.rs
//
// #372: the escapes of an NQL string are read. The grammar accepted `\"`,
// `\'`, `\\`, `\n`, `\r` and `\t`, but the parser only removed the quotes:
// until 0.10.11 `"dice \"hola\""` was stored as `dice \"hola\"`, `"C:\\tmp"`
// as `C:\\tmp`, and `export parquet "..."` reported a path other than the
// one written.
//
// Fictional domain: a kitchen.

use nopaldb::{Graph, NqlResult, PropertyValue, Result};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Every escape, written in NQL and the value it stands for.
const ESCAPES: [(&str, &str, &str); 6] = [
    ("comillas", r#""dice \"hola\"""#, "dice \"hola\""),
    ("apostrofo", r"'it\'s'", "it's"),
    ("barra", r#""C:\\cocina""#, "C:\\cocina"),
    ("salto", r#""una\ndos""#, "una\ndos"),
    ("retorno", r#""una\rdos""#, "una\rdos"),
    ("tab", r#""una\tdos""#, "una\tdos"),
];

async fn one(g: &Graph, q: &str, column: &str) -> PropertyValue {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let rows = r.rows();
    assert_eq!(rows.len(), 1, "{q}");
    rows[0].get(column).cloned().unwrap_or(PropertyValue::Null)
}

#[tokio::test]
async fn add_stores_the_value_the_escape_stands_for() -> Result<()> {
    for (name, literal, value) in ESCAPES {
        let g = Graph::in_memory().await?;
        g.execute_nql(&format!("add (r:Receta {{nota: {literal}}})")).await?;
        assert_eq!(one(&g, "find r.nota from (r:Receta)", "r.nota").await, s(value), "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn where_compares_with_the_value_the_escape_stands_for() -> Result<()> {
    for (name, literal, value) in ESCAPES {
        let g = Graph::in_memory().await?;
        // Stored through the API: the value itself, no NQL in between.
        g.add_node(nopaldb::Node::new("Receta").with_property("nota", s(value))).await?;
        let q = format!("find r.nota from (r:Receta) where r.nota = {literal}");
        assert_eq!(one(&g, &q, "r.nota").await, s(value), "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn set_writes_the_value_the_escape_stands_for() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.execute_nql(r#"add (r:Receta {nombre: "mole"})"#).await?;
    g.execute_nql(r#"update (r:Receta) set r.nota = 'it\'s "picante"\n'"#).await?;
    assert_eq!(
        one(&g, "find r.nota from (r:Receta)", "r.nota").await,
        s("it's \"picante\"\n")
    );
    Ok(())
}

/// A path inside an NQL single-quoted string: `\` and `'` are escaped.
fn nql_single_quoted(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "\\\\").replace('\'', "\\'")
}

#[tokio::test]
async fn export_paths_and_options_are_read_with_their_escapes() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.execute_nql(r#"add (r:Receta {nombre: "mole", porciones: 6})"#).await?;
    let dir = tempfile::tempdir()?;

    // The path of `export parquet`, with an apostrophe in the file name.
    let path = dir.path().join("it's.parquet");
    let q = format!("find r.nombre from (r:Receta) export parquet '{}'", nql_single_quoted(&path));
    match g.execute_statement(&q).await? {
        NqlResult::Export { data, .. } => assert_eq!(data, path.display().to_string()),
        other => panic!("expected Export, got: {}", other.summary()),
    }
    assert!(path.exists(), "{}", path.display());

    // An option of `export csv`: a tab separator, written `\t`.
    let path = dir.path().join("o'k.csv");
    let q = format!(
        r#"find r.nombre, r.porciones from (r:Receta) export csv with path='{}', separator="\t""#,
        nql_single_quoted(&path)
    );
    g.execute_statement(&q).await?;
    assert_eq!(std::fs::read_to_string(&path)?, "r.nombre\tr.porciones\nmole\t6\n");
    Ok(())
}
