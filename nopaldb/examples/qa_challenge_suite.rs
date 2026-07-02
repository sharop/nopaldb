use nopaldb::{Direction, Edge, Graph, Node, PropertyValue, Result};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

fn s(value: impl Into<String>) -> PropertyValue {
    PropertyValue::String(value.into())
}

fn i(value: i64) -> PropertyValue {
    PropertyValue::Int(value)
}

fn b(value: bool) -> PropertyValue {
    PropertyValue::Bool(value)
}

fn parse_args() -> (PathBuf, bool) {
    let mut db_path = PathBuf::from("./nopaldb/test_dbs/qa_challenge_db");
    let mut reset = true;

    let args: Vec<String> = env::args().collect();
    let mut idx = 1;
    while idx < args.len() {
        match args[idx].as_str() {
            "--db" => {
                if idx + 1 < args.len() {
                    db_path = PathBuf::from(&args[idx + 1]);
                    idx += 1;
                }
            }
            "--no-reset" => {
                reset = false;
            }
            _ => {}
        }
        idx += 1;
    }

    (db_path, reset)
}

fn maybe_reset(path: &Path, reset: bool) -> Result<()> {
    if reset && path.exists() {
        std::fs::remove_dir_all(path)?;
    }

    std::fs::create_dir_all(path)?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let (db_path, reset) = parse_args();
    maybe_reset(&db_path, reset)?;

    println!("\\n=== QA Challenge Suite (Rust) ===");
    println!("Database path: {}", db_path.display());
    println!("Reset: {}", reset);

    let graph = Graph::open(&db_path).await?;

    // 1) Seed a challenging graph with bulk loader (multiple labels/types/properties)
    let mut loader = graph.bulk_loader(256);
    let mut users = Vec::new();
    let mut projects = Vec::new();
    let mut services = Vec::new();
    let mut alerts = Vec::new();

    for p in 0..20 {
        let project = Node::new("Project")
            .with_property("name", s(format!("project-{:02}", p)))
            .with_property("priority", i((p % 5 + 1) as i64))
            .with_property("status", s(if p % 3 == 0 { "paused" } else { "active" }));
        projects.push(project.id);
        loader.add_node(project).await?;
    }

    for svc in 0..10 {
        let service = Node::new("Service")
            .with_property("name", s(format!("svc-{:02}", svc)))
            .with_property("tier", s(if svc % 2 == 0 { "critical" } else { "standard" }))
            .with_property("region", s(match svc % 3 {
                0 => "us-east",
                1 => "us-west",
                _ => "eu-central",
            }));
        services.push(service.id);
        loader.add_node(service).await?;
    }

    for u in 0..120 {
        let user = Node::new("User")
            .with_property("username", s(format!("user_{:03}", u)))
            .with_property("team", s(format!("team_{}", u % 6)))
            .with_property("score", i((40 + (u % 61)) as i64))
            .with_property("active", b(u % 11 != 0))
            .with_property("bio", s(format!(
                "Engineer {} builds resilient graph pipelines and incident tooling",
                u
            )));
        users.push(user.id);
        loader.add_node(user).await?;
    }

    for a in 0..48 {
        let alert = Node::new("Alert")
            .with_property("name", s(format!("alert-{:03}", a)))
            .with_property("severity", s(match a % 4 {
                0 => "critical",
                1 => "high",
                2 => "medium",
                _ => "low",
            }))
            .with_property("open", b(a % 5 != 0));
        alerts.push(alert.id);
        loader.add_node(alert).await?;
    }

    for (idx, user_id) in users.iter().enumerate() {
        let project_id = projects[idx % projects.len()];
        let service_id = services[idx % services.len()];

        loader
            .add_edge(
                Edge::new(*user_id, project_id, "MEMBER_OF")
                    .with_property("since", i((2018 + (idx % 8)) as i64)),
            )
            .await?;

        loader
            .add_edge(
                Edge::new(*user_id, service_id, "USES_SERVICE")
                    .with_property("calls_per_day", i((50 + (idx % 450)) as i64)),
            )
            .await?;

        if idx + 1 < users.len() {
            loader
                .add_edge(Edge::new(*user_id, users[idx + 1], "MENTORS"))
                .await?;
        }
    }

    for (idx, alert_id) in alerts.iter().enumerate() {
        let service_id = services[idx % services.len()];
        loader
            .add_edge(
                Edge::new(*alert_id, service_id, "ON_SERVICE")
                    .with_property("window_min", i((idx % 60) as i64 + 1)),
            )
            .await?;
    }

    let bulk_stats = loader.finish().await?;
    println!(
        "Seeded with BulkLoader: {} nodes, {} edges in {:.2}s",
        bulk_stats.nodes_inserted,
        bulk_stats.edges_inserted,
        bulk_stats.duration.as_secs_f64()
    );

    // 2) Transaction commit and rollback paths
    {
        let mut tx = graph.begin_transaction().await?;
        let incident = Node::new("Incident")
            .with_property("title", s("latency spike"))
            .with_property("status", s("open"));
        tx.add_node(incident).await?;
        tx.commit().await?;
    }

    {
        let mut tx = graph.begin_transaction().await?;
        let transient = Node::new("Transient")
            .with_property("name", s("should_rollback"));
        tx.add_node(transient).await?;
        tx.rollback()?;
    }

    // 3) NQL write statements (ADD/UPDATE/DELETE) and EXPLAIN
    graph
        .execute_statement(r#"add (ops:OpsRun {name: "nightly-qa", ok: true})"#)
        .await?;

    graph
        .execute_statement(r#"update (u:User) set u.status = "inactive" where u.active = false"#)
        .await?;

    graph
        .execute_statement(r#"delete (a:Alert) where a.severity = "low" limit 5"#)
        .await?;

    let explain = graph
        .execute_statement("explain find u.username from (u:User) where u.score > 80")
        .await?;
    println!("Explain summary: {}", explain.summary());

    // 4) NQL export
    let export_csv = graph
        .execute_statement("find u.username, u.team from (u:User) limit 10 export csv")
        .await?;
    println!("CSV export summary: {}", export_csv.summary());

    let export_json = graph
        .execute_statement("find s.name, s.region from (s:Service) export json")
        .await?;
    println!("JSON export summary: {}", export_json.summary());

    // 5) Index lifecycle
    graph
        .execute_statement("create index on User(username) type hash")
        .await?;
    graph
        .execute_statement("create index on User(score) type btree")
        .await?;
    graph
        .execute_statement("create index on User(bio) type fulltext")
        .await?;
    graph
        .execute_statement("create index on Service(name) type hash")
        .await?;

    let indexes = graph.list_indexes().await;
    println!("Indexes created: {}", indexes.len());

    graph.execute_statement("drop index Service_name").await?;
    graph
        .execute_statement("create index on Service(name) type hash")
        .await?;

    // 6) Query patterns, aggregations, algorithms
    let q1 = graph
        .execute_nql("find u.team, count(u) as total from (u:User) group by u.team order by u.team asc")
        .await?;
    println!("Team aggregation rows: {}", q1.len());

    let q2 = graph
        .execute_nql("find u.username, p.name from (u:User) -> [:MEMBER_OF] -> (p:Project) limit 12")
        .await?;
    println!("Pattern match rows: {}", q2.len());

    let q3 = graph
        .execute_nql("find degree(u) as deg, pagerank(u) as pr from (u:User) limit 5")
        .await?;
    println!("Algorithms rows: {}", q3.len());

    // 7) Schema and stats
    let schema = graph.get_schema().await?;
    let stats = graph.get_stats().await?;
    let labels: HashSet<String> = schema.node_labels.into_iter().collect();

    assert!(labels.contains("User"), "schema missing User label");
    assert!(labels.contains("Project"), "schema missing Project label");
    assert!(stats.total_nodes > 0, "stats total_nodes should be > 0");
    assert!(stats.total_edges > 0, "stats total_edges should be > 0");

    // 8) Basic graph API checks
    let out_degree = graph.degree(users[0], Direction::Outgoing).await?;
    let neighbors = graph.neighbors(users[0], Direction::Outgoing).await?;
    println!("User[0] outgoing degree: {} (neighbors: {})", out_degree, neighbors.len());

    let rolled_back = graph
        .execute_nql(r#"find t.name from (t:Transient) where t.name = "should_rollback""#)
        .await?;
    assert_eq!(rolled_back.len(), 0, "rollback test failed");

    #[cfg(feature = "analytics")]
    {
        let rb = graph.to_arrow_with_label(Some("User")).await?;
        println!("Arrow export rows for User: {}", rb.num_rows());
    }

    graph.close().await?;

    println!("\\n✅ QA challenge DB ready at {}", db_path.display());
    println!("Run next: python nopaldb/examples/qa_challenge_python_checks.py --db {}", db_path.display());
    println!("Open in ndbstudio: ndbstudio {}", db_path.display());

    Ok(())
}
