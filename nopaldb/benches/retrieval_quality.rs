//! Calidad de recuperación de punta a punta con texto real (#212, parte 2).
//!
//! `retrieval_dims` mide si el índice HNSW devuelve los vecinos exactos. Este
//! bench mide otra cosa: si la búsqueda encuentra los **párrafos que
//! responden** cada pregunta, y cuánto añade expandir por el grafo.
//!
//! Datos: un directorio de `nopaldb/python/scripts/prepare_2wiki.py`
//! (`NOPALDB_BENCH_DATA`), el split `dev` de 2WikiMultiHopQA: ~55k párrafos y
//! ~12.6k preguntas de 2 a 4 saltos, cada una con los títulos de sus párrafos
//! de apoyo (el oro, 2 o 4) y sus evidencias como tripletas
//! (sujeto, relación, objeto).
//!
//! **Grafo.** `Chunk` por párrafo (texto con índice full-text y embedding del
//! modelo); `Entity` por título de párrafo y por cada extremo de tripleta
//! que no sea un título; `Chunk -[ABOUT]-> Entity` de su título; y por cada
//! tripleta `Entity -[relación]-> Entity`. Un extremo se enlaza al título
//! igual, o al título normalizado (minúsculas, sin sufijo `(…)`) si es
//! único. El camino de un salto del razonamiento es `Chunk → Entity →
//! Entity → Chunk`: profundidad 3, en las dos direcciones.
//!
//! **El grafo sale de las tripletas del dataset, no de un extractor.** Son
//! los saltos de razonamiento de las preguntas, así que con todas ellas
//! (`KG completo`) el grafo contiene el camino de cada pregunta evaluada:
//! es el techo de lo que la expansión puede añadir con un grafo perfecto.
//! `KG sin la muestra` usa solo las tripletas que aparecen en preguntas
//! fuera de la muestra evaluada: ningún camino evaluado se filtra, y solo
//! queda lo que el resto del dataset sabe de las mismas entidades (el 24 %
//! de las tripletas de una muestra de 1000 aparece en otras preguntas). Las
//! tripletas solo de la muestra van con el tipo de arista `EVAL_<relación>`
//! y la expansión sin la muestra no las recorre.
//!
//! **Estrategias**, cada una da una lista ordenada de párrafos:
//!
//! - `vector`: KNN de la pregunta.
//! - `híbrida`: `search_hybrid` con el texto de la pregunta y su vector (RRF).
//! - `híbrida+BFS`: las primeras `s` de la híbrida como semillas
//!   (`NOPALDB_BENCH_SEEDS`, default 5), en su orden, y después los párrafos
//!   de su vecindario en orden de BFS. Con `s ≥ 5` el recall@5 es el de la
//!   híbrida por construcción: la expansión solo llena los puestos 6–10.
//! - `híbrida+PPR`: igual, con la expansión ordenada por Personalized
//!   PageRank sembrado en las semillas (peso = score de la híbrida).
//! - `PPR reordena semillas`: el vecindario entero, semillas incluidas,
//!   ordenado por PPR. Los scores de RRF son casi iguales entre sí
//!   (1/(60 + rango)), así que como peso apenas distinguen a las semillas y
//!   PPR pierde el orden de la híbrida; esta fila mide cuánto cuesta.
//!
//! Toda lista se completa con el resto de la híbrida hasta 10, para que una
//! expansión que encuentra poco no pierda los puestos que la híbrida sí
//! llenaba.
//!
//! Vecindario: profundidad 3, `Both`, 200 nodos como máximo y 64 aristas por
//! nodo (los países y las fechas son supernodos).
//!
//! **Métricas**, sobre una muestra fija de preguntas (`NOPALDB_BENCH_QUESTIONS`,
//! default 1000, semilla 212): recall@k = |top-k ∩ oro| / |oro|, promediado;
//! MRR = 1 / posición del primer párrafo del oro en el top 10 (0 si no
//! está). Y el recall@10 por tipo de pregunta.
//!
//! Correr: `NOPALDB_BENCH_DATA=<dir> make bench BENCH=retrieval_quality`.
//! `NOPALDB_BENCH_MODEL` (default `minilm`), `NOPALDB_BENCH_EF` (default
//! `30,100`). La construcción del HNSW en paralelo no es determinista:
//! corre el bench varias veces para ver la dispersión.
//!
//! **Gate** (job `retrieval-quality`, nightly). `NOPALDB_BENCH_JSON=<archivo>`
//! escribe las cifras de cada fila (solo agregados: ni texto, ni títulos, ni
//! ids del dataset). `NOPALDB_BENCH_BASELINE=<archivo>` compara el recall@10
//! de cada fila contra ese archivo y termina con error si alguna cae más de
//! `NOPALDB_BENCH_TOLERANCE` (default 0.02); si alguna sube más, lo avisa
//! para que se suba la línea base. Una fila que falta en un lado también es
//! error. Actualizar la línea base es copiar el JSON de una corrida. Las dos
//! rutas, absolutas: `cargo bench` corre desde `nopaldb/`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{Direction, ExpandOptions, Graph, HybridFilter, HybridQuery, Rank};

const K: usize = 10;
const MODEL: &str = "m";
const DEPTH: usize = 3;
const MAX_NODES: usize = 200;
const MAX_EDGES_PER_NODE: usize = 64;
const TYPES: [&str; 4] = ["compositional", "comparison", "inference", "bridge_comparison"];

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_or(name, &default.to_string()).parse().unwrap_or_else(|_| panic!("{name} no es un número"))
}

struct Question {
    text: String,
    kind: String,
    gold: Vec<String>,
    evidences: Vec<(String, String, String)>,
}

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON line"))
        .collect()
}

fn read_f32(path: &Path, dim: usize) -> Vec<Vec<f32>> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(bytes.len() % (4 * dim), 0, "{}: not rows of {dim} float32", path.display());
    bytes
        .chunks_exact(4 * dim)
        .map(|row| row.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
        .collect()
}

/// `n` distinct indices out of `len`, the same for a seed.
fn sample(len: usize, n: usize, seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..len).collect();
    let mut x = seed.max(1);
    let n = n.min(len);
    for i in 0..n {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let r = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) % (len - i) as u64) as usize;
        idx.swap(i, i + r);
    }
    idx.truncate(n);
    idx
}

fn normalized(title: &str) -> String {
    let t = title.trim();
    let t = match (t.ends_with(')'), t.rfind(" (")) {
        (true, Some(i)) => &t[..i],
        _ => t,
    };
    t.trim().to_lowercase()
}

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

struct Corpus {
    graph: Graph,
    _dir: tempfile::TempDir,
    chunk_of_title: HashMap<String, NodeId>,
    title_of_chunk: HashMap<NodeId, String>,
    /// Edge types of the triples that also appear outside the sample.
    shared_types: Vec<String>,
    /// Edge types of every triple.
    all_types: Vec<String>,
    eval_triples: usize,
    shared_triples: usize,
}

async fn build(dir: &Path, model: &str, questions: &[Question], evaluated: &HashSet<usize>) -> Corpus {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("manifest.json")).expect("manifest");
    let dim = manifest["models"][model]["dim"].as_u64().unwrap_or_else(|| panic!("model {model} not in manifest")) as usize;
    let chunks = read_jsonl(&dir.join("chunks.jsonl"));
    let vectors = read_f32(&dir.join(format!("{model}.chunks.f32")), dim);
    assert_eq!(chunks.len(), vectors.len());

    let tmp = tempfile::tempdir().expect("tempdir");
    let graph = Graph::open(tmp.path()).await.expect("open");

    // Entities: one per title, plus the endpoints that match no title.
    let titles: Vec<String> = chunks.iter().map(|c| c["title"].as_str().unwrap().to_string()).collect();
    let mut by_norm: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in titles.iter().enumerate() {
        by_norm.entry(normalized(t)).or_default().push(i);
    }
    let title_index: HashMap<&str, usize> = titles.iter().enumerate().map(|(i, t)| (t.as_str(), i)).collect();
    let entity_key = |name: &str| -> String {
        if title_index.contains_key(name) {
            return name.to_string();
        }
        match by_norm.get(&normalized(name)) {
            Some(v) if v.len() == 1 => titles[v[0]].clone(),
            _ => format!("\u{0}{name}"),
        }
    };

    // Triples, split by whether a question outside the sample has them.
    let mut outside: HashSet<(String, String, String)> = HashSet::new();
    for (i, q) in questions.iter().enumerate() {
        if !evaluated.contains(&i) {
            outside.extend(q.evidences.iter().cloned());
        }
    }
    let mut triples: HashMap<(String, String, String), bool> = HashMap::new();
    for q in questions {
        for e in &q.evidences {
            let shared = outside.contains(e);
            let key = (entity_key(&e.0), e.1.clone(), entity_key(&e.2));
            let slot = triples.entry(key).or_insert(false);
            *slot |= shared;
        }
    }

    let t = Instant::now();
    let mut chunk_of_title = HashMap::new();
    let mut title_of_chunk = HashMap::new();
    let mut entity: HashMap<String, NodeId> = HashMap::new();
    let mut shared_types = HashSet::new();
    let mut all_types = HashSet::new();
    let (mut eval_triples, mut shared_triples) = (0, 0);
    {
        let mut loader = graph.bulk_loader(10_000);
        for c in &chunks {
            let title = c["title"].as_str().unwrap();
            let node = Node::new("Chunk").with_property("title", s(title)).with_property("text", s(c["text"].as_str().unwrap()));
            let id = node.id;
            loader.add_node(node).await.expect("chunk");
            let e = Node::new("Entity").with_property("name", s(title));
            let eid = e.id;
            loader.add_node(e).await.expect("entity");
            loader.add_edge(Edge::new(id, eid, "ABOUT")).await.expect("about");
            chunk_of_title.insert(title.to_string(), id);
            title_of_chunk.insert(id, title.to_string());
            entity.insert(title.to_string(), eid);
        }
        for ((a, rel, b), shared) in &triples {
            let mut id_of = |k: &str| -> Option<(NodeId, Node)> {
                if entity.contains_key(k) {
                    return None;
                }
                let n = Node::new("Entity").with_property("name", s(k.trim_start_matches('\u{0}')));
                entity.insert(k.to_string(), n.id);
                Some((n.id, n))
            };
            for k in [a.as_str(), b.as_str()] {
                if let Some((_, n)) = id_of(k) {
                    loader.add_node(n).await.expect("entity");
                }
            }
            let ty = if *shared { rel.clone() } else { format!("EVAL_{rel}") };
            if *shared {
                shared_types.insert(ty.clone());
                shared_triples += 1;
            } else {
                eval_triples += 1;
            }
            all_types.insert(ty.clone());
            loader.add_edge(Edge::new(entity[a.as_str()], entity[b.as_str()], ty)).await.expect("rel");
        }
        loader.finish().await.expect("finish");
    }
    let ids: Vec<NodeId> = titles.iter().map(|t| chunk_of_title[t]).collect();
    for (id_chunk, vec_chunk) in ids.chunks(10_000).zip(vectors.chunks(10_000)) {
        let items = id_chunk.iter().copied().zip(vec_chunk.iter().cloned()).collect();
        graph.add_node_embeddings(MODEL, items).await.expect("embeddings");
    }
    graph.create_index("Chunk", "text", nopaldb::index::IndexType::FullText).await.expect("fulltext");
    graph.get_or_build_embedding_index(MODEL).await.expect("hnsw");
    eprintln!(
        "built in {:.0} s: {} chunks, {} entities, {} triples ({} also outside the sample)",
        t.elapsed().as_secs_f64(),
        chunks.len(),
        entity.len(),
        triples.len(),
        shared_triples
    );
    let mut all_types: Vec<String> = all_types.into_iter().collect();
    all_types.push("ABOUT".into());
    let mut shared_types: Vec<String> = shared_types.into_iter().collect();
    shared_types.push("ABOUT".into());
    Corpus { graph, _dir: tmp, chunk_of_title, title_of_chunk, shared_types, all_types, eval_triples, shared_triples }
}

#[derive(Default, Clone)]
struct Score {
    r5: f64,
    r10: f64,
    mrr: f64,
    n: usize,
    by_type: HashMap<String, (f64, usize)>,
}

impl Score {
    fn add(&mut self, ranked: &[NodeId], gold: &HashSet<NodeId>, kind: &str) {
        let hits = |k: usize| ranked.iter().take(k).filter(|id| gold.contains(id)).count() as f64 / gold.len() as f64;
        let r10 = hits(10);
        self.r5 += hits(5);
        self.r10 += r10;
        self.mrr += ranked.iter().take(K).position(|id| gold.contains(id)).map_or(0.0, |p| 1.0 / (p + 1) as f64);
        self.n += 1;
        let t = self.by_type.entry(kind.to_string()).or_default();
        t.0 += r10;
        t.1 += 1;
    }

    fn json(&self, name: &str, ef: usize, kg: &str) -> serde_json::Value {
        let n = self.n as f64;
        let round = |x: f64| (x * 1000.0).round() / 1000.0;
        let by_type: serde_json::Map<String, serde_json::Value> = TYPES
            .iter()
            .filter_map(|t| self.by_type.get(*t).map(|(s, c)| (t.to_string(), round(s / *c as f64).into())))
            .collect();
        serde_json::json!({
            "strategy": name,
            "ef": ef,
            "kg": kg,
            "recall@5": round(self.r5 / n),
            "recall@10": round(self.r10 / n),
            "mrr": round(self.mrr / n),
            "recall@10 by type": by_type,
        })
    }

    fn row(&self, name: &str, ef: usize, kg: &str) -> String {
        let n = self.n as f64;
        let types: Vec<String> = TYPES
            .iter()
            .map(|t| self.by_type.get(*t).map_or("—".into(), |(s, c)| format!("{:.3}", s / *c as f64)))
            .collect();
        format!(
            "| {name} | {ef} | {kg} | {:.3} | {:.3} | {:.3} | {} |",
            self.r5 / n,
            self.r10 / n,
            self.mrr / n,
            types.join(" | ")
        )
    }
}

/// `first` in order, then `then` and finally the rest of `fill`, each
/// paragraph once: every strategy gets a full top 10 even when the
/// expansion finds little.
fn ranked_list(first: &[NodeId], then: impl Iterator<Item = NodeId>, fill: &[NodeId]) -> Vec<NodeId> {
    let mut seen = HashSet::new();
    first
        .iter()
        .copied()
        .chain(then)
        .chain(fill.iter().copied())
        .filter(|id| seen.insert(*id))
        .collect()
}

fn row_key(row: &serde_json::Value) -> String {
    format!("{} | ef {} | KG {}", row["strategy"].as_str().unwrap_or("?"), row["ef"], row["kg"].as_str().unwrap_or("?"))
}

/// Compares the recall@10 of every row against `baseline`; returns the
/// failures. The setup (model, questions, seeds) must match: numbers from a
/// different run say nothing about a regression.
fn check_baseline(current: &serde_json::Value, baseline: &serde_json::Value, tolerance: f64) -> Vec<String> {
    let mut failures = Vec::new();
    for field in ["model", "questions", "seeds"] {
        if current[field] != baseline[field] {
            failures.push(format!("{field}: this run {} , baseline {}", current[field], baseline[field]));
        }
    }
    if !failures.is_empty() {
        return failures;
    }
    let rows = |v: &serde_json::Value| -> HashMap<String, f64> {
        v["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| (row_key(r), r["recall@10"].as_f64().expect("recall@10")))
            .collect()
    };
    let (now, base) = (rows(current), rows(baseline));
    let mut keys: Vec<&String> = now.keys().chain(base.keys()).collect::<HashSet<_>>().into_iter().collect();
    keys.sort();
    for key in keys {
        match (now.get(key), base.get(key)) {
            (Some(n), Some(b)) if n + tolerance < *b => {
                failures.push(format!("{key}: recall@10 {n:.3}, baseline {b:.3} (tolerance {tolerance})"))
            }
            (Some(n), Some(b)) if *n > b + tolerance => {
                eprintln!("notice: {key}: recall@10 {n:.3} is above the baseline {b:.3}: raise the baseline")
            }
            (Some(_), Some(_)) => {}
            (Some(_), None) => failures.push(format!("{key}: not in the baseline")),
            (None, _) => failures.push(format!("{key}: in the baseline but not in this run")),
        }
    }
    failures
}

fn main() {
    let dir = PathBuf::from(std::env::var("NOPALDB_BENCH_DATA").expect(
        "NOPALDB_BENCH_DATA=<dir> written by nopaldb/python/scripts/prepare_2wiki.py",
    ));
    let model = env_or("NOPALDB_BENCH_MODEL", "minilm");
    let n_questions = env_usize("NOPALDB_BENCH_QUESTIONS", 1000);
    let seeds = env_usize("NOPALDB_BENCH_SEEDS", 5);
    let efs: Vec<usize> = env_or("NOPALDB_BENCH_EF", "30,100").split(',').map(|x| x.trim().parse().expect("ef")).collect();

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("manifest.json")).expect("manifest");
    let dim = manifest["models"][&model]["dim"].as_u64().expect("model dim") as usize;
    let questions: Vec<Question> = read_jsonl(&dir.join("questions.jsonl"))
        .into_iter()
        .map(|q| Question {
            text: q["question"].as_str().unwrap().to_string(),
            kind: q["type"].as_str().unwrap().to_string(),
            gold: q["gold"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect(),
            evidences: q["evidences"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| (e[0].as_str().unwrap().into(), e[1].as_str().unwrap().into(), e[2].as_str().unwrap().into()))
                .collect(),
        })
        .collect();
    let qvectors = read_f32(&dir.join(format!("{model}.questions.f32")), dim);
    assert_eq!(questions.len(), qvectors.len());
    let picked = sample(questions.len(), n_questions, 212);
    let evaluated: HashSet<usize> = picked.iter().copied().collect();

    let mut rows = Vec::new();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let c = build(&dir, &model, &questions, &evaluated).await;
        eprintln!("triples only in the sample: {}, also outside: {}", c.eval_triples, c.shared_triples);
        println!(
            "| estrategia | ef | KG | recall@5 | recall@10 | MRR | {} |",
            TYPES.iter().map(|t| format!("r@10 {t}")).collect::<Vec<_>>().join(" | ")
        );
        println!("|---|---|---|---|---|---|---|---|---|---|");
        for &ef in &efs {
            let mut vector = Score::default();
            let mut hybrid = Score::default();
            let kgs = [("completo", c.all_types.clone()), ("sin la muestra", c.shared_types.clone())];
            let mut bfs = vec![Score::default(); 2];
            let mut ppr = vec![Score::default(); 2];
            let mut ppr_all = vec![Score::default(); 2];
            let t = Instant::now();
            for &i in &picked {
                let q = &questions[i];
                let gold: HashSet<NodeId> = q.gold.iter().map(|t| c.chunk_of_title[t]).collect();

                let mut vq = HybridQuery::new();
                vq.vector = Some((qvectors[i].clone(), MODEL.into()));
                vq.k = K;
                vq.ef_search = Some(ef);
                let v: Vec<NodeId> = c.graph.search_hybrid(vq).await.expect("vector").into_iter().map(|h| h.node_id).collect();
                vector.add(&v, &gold, &q.kind);

                let mut hq = HybridQuery::new();
                hq.text = Some(q.text.clone());
                hq.vector = Some((qvectors[i].clone(), MODEL.into()));
                hq.k = K;
                hq.ef_search = Some(ef);
                hq.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![] });
                let hits = c.graph.search_hybrid(hq).await.unwrap_or_else(|e| panic!("{:?}: {e}", q.text));
                let h: Vec<NodeId> = hits.iter().map(|h| h.node_id).collect();
                hybrid.add(&h, &gold, &q.kind);

                let seed_ids: Vec<NodeId> = h.iter().take(seeds).copied().collect();
                let weights: HashMap<NodeId, f64> = hits.iter().take(seeds).map(|h| (h.node_id, h.score as f64)).collect();
                for (j, (_, types)) in kgs.iter().enumerate() {
                    let mut opts = ExpandOptions {
                        direction: Direction::Both,
                        edge_types: Some(types.clone()),
                        max_nodes: MAX_NODES,
                        max_edges_per_node: Some(MAX_EDGES_PER_NODE),
                        ..ExpandOptions::default()
                    };
                    let nb = c.graph.neighborhood(&seed_ids, DEPTH, &opts).await.expect("bfs");
                    let expanded = nb.nodes.iter().map(|n| n.id).filter(|id| c.title_of_chunk.contains_key(id));
                    bfs[j].add(&ranked_list(&seed_ids, expanded, &h), &gold, &q.kind);

                    opts.rank = Rank::Ppr { alpha: 0.15, iterations: 20, candidate_factor: 5, seed_weights: Some(weights.clone()) };
                    let nb = c.graph.neighborhood(&seed_ids, DEPTH, &opts).await.expect("ppr");
                    let mut by_score: Vec<(NodeId, f64)> = nb
                        .nodes
                        .iter()
                        .filter(|n| c.title_of_chunk.contains_key(&n.id))
                        .map(|n| (n.id, nb.score_of.get(&n.id).copied().unwrap_or(0.0)))
                        .collect();
                    by_score.sort_by(|a, b| b.1.total_cmp(&a.1));
                    let by_score: Vec<NodeId> = by_score.into_iter().map(|(id, _)| id).collect();
                    ppr[j].add(&ranked_list(&seed_ids, by_score.iter().copied(), &h), &gold, &q.kind);
                    ppr_all[j].add(&ranked_list(&[], by_score.iter().copied(), &h), &gold, &q.kind);
                }
            }
            eprintln!("ef {ef}: {} questions in {:.0} s", picked.len(), t.elapsed().as_secs_f64());
            let mut scored = vec![(vector, "vector".to_string(), "—"), (hybrid, "híbrida".to_string(), "—")];
            for (j, (kg, _)) in kgs.iter().enumerate() {
                scored.push((bfs[j].clone(), format!("híbrida+BFS (s={seeds})"), *kg));
                scored.push((ppr[j].clone(), format!("híbrida+PPR (s={seeds})"), *kg));
                scored.push((ppr_all[j].clone(), format!("PPR reordena semillas (s={seeds})"), *kg));
            }
            for (score, name, kg) in &scored {
                println!("{}", score.row(name, ef, kg));
                rows.push(score.json(name, ef, kg));
            }
        }
        c.graph.close().await.expect("close");
    });

    let report = serde_json::json!({
        "model": model,
        "questions": picked.len(),
        "seeds": seeds,
        "rows": rows,
    });
    if let Ok(path) = std::env::var("NOPALDB_BENCH_JSON") {
        let text = serde_json::to_string_pretty(&report).expect("json") + "\n";
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    if let Ok(path) = std::env::var("NOPALDB_BENCH_BASELINE") {
        let tolerance: f64 =
            env_or("NOPALDB_BENCH_TOLERANCE", "0.02").parse().expect("NOPALDB_BENCH_TOLERANCE no es un número");
        let baseline: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("baseline");
        let failures = check_baseline(&report, &baseline, tolerance);
        if failures.is_empty() {
            eprintln!("recall@10 within {tolerance} of the baseline {path} in every row");
        } else {
            for f in &failures {
                eprintln!("REGRESSION: {f}");
            }
            std::process::exit(1);
        }
    }
}
