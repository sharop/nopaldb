//! La rama de texto bajo un filtro selectivo (#213): cuánto cuesta cada
//! camino del índice full-text con 100k documentos que contienen el término.
//!
//! - `global_escalation`: lo que hacía la rama de texto — pedir el top-N
//!   global (32 → 128 → 512 → 1000) y filtrar después. Con el conjunto
//!   permitido al final del ranking BM25 agota el tope y devuelve 0; con el
//!   conjunto al principio llena en la primera petición.
//! - `term_set`: `query_scored_in`, tantivy puntúa solo los permitidos.
//!
//! Lo que decide `TEXT_EXACT_THRESHOLD`: el tamaño de conjunto hasta el que el
//! term set cuesta lo mismo o menos que el escalado global en su mejor caso.
//! `NOPALDB_BENCH_SCALE` fija los documentos (default 100_000).

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nopaldb::index::{FullTextIndex, Index, IndexQuery};
use nopaldb::types::{NodeId, PropertyValue};

const K_CANDIDATES: usize = 32;
const CAP: usize = 1000;

fn scale() -> usize {
    std::env::var("NOPALDB_BENCH_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000)
}

struct Fixture {
    index: FullTextIndex,
    /// Ids de los documentos con el peor BM25 del término (los más largos).
    tail: Vec<NodeId>,
    /// Ids de los documentos con el mejor BM25 (los más cortos).
    head: Vec<NodeId>,
}

fn build(n: usize) -> Fixture {
    let mut index = FullTextIndex::new(None).expect("index");
    let mut by_len: Vec<(usize, NodeId)> = Vec::with_capacity(n);
    for i in 0..n {
        let id = uuid::Uuid::new_v4();
        // Longitudes 1..=64 palabras de relleno: BM25 decrece con la longitud.
        let len = i % 64 + 1;
        // "nopal" en todos; "raro" en el 1 %: el coste del top-N global
        // depende de cuántos documentos contienen el término, el del term
        // set no (lo señaló una revisión externa de #213).
        let rare = if i % 100 == 0 { " raro" } else { "" };
        let text = format!("nopal{rare} {}", "relleno ".repeat(len));
        index.insert(PropertyValue::String(text), id).expect("insert");
        by_len.push((len, id));
    }
    index.flush().expect("flush");
    by_len.sort_by_key(|(len, _)| *len);
    let head = by_len.iter().take(8000).map(|(_, id)| *id).collect();
    let tail = by_len.iter().rev().take(8000).map(|(_, id)| *id).collect();
    Fixture { index, tail, head }
}

/// La rama de texto de 0.6.12: top-N global escalado ×4 hasta el tope, filtro después.
fn global_escalation(index: &FullTextIndex, q: &IndexQuery, allowed: &HashSet<NodeId>) -> Vec<(NodeId, Option<f32>)> {
    let mut limit = K_CANDIDATES;
    loop {
        let scored = index.query_scored_top(q, limit).expect("top");
        let exhausted = scored.len() < limit || limit >= CAP;
        let kept: Vec<_> = scored.into_iter().filter(|(id, _)| allowed.contains(id)).take(K_CANDIDATES).collect();
        if kept.len() >= K_CANDIDATES || exhausted {
            return kept;
        }
        limit = limit.saturating_mul(4).min(CAP);
    }
}

fn bench_text_filter(c: &mut Criterion) {
    let n = scale();
    let fx = build(n);
    let q = IndexQuery::FullText("nopal".to_string());

    let mut g = c.benchmark_group("text_filter");
    g.sample_size(30).measurement_time(Duration::from_secs(5));
    // Término en el 1 % de los documentos: lo que cuesta cada camino cuando
    // el texto ya es selectivo por sí mismo.
    let rare = IndexQuery::FullText("raro".to_string());
    let tail_512: HashSet<NodeId> = fx.tail.iter().take(512).copied().collect();
    g.bench_with_input(BenchmarkId::new("global_first_request_rare_term", 512), &512usize, |b, _| {
        b.iter(|| black_box(fx.index.query_scored_top(&rare, K_CANDIDATES).expect("top")))
    });
    g.bench_with_input(BenchmarkId::new("global_escalation_tail_rare_term", 512), &512usize, |b, _| {
        b.iter(|| black_box(global_escalation(&fx.index, &rare, &tail_512)))
    });
    g.bench_with_input(BenchmarkId::new("term_set_tail_rare_term", 512), &512usize, |b, _| {
        b.iter(|| black_box(fx.index.query_scored_in(&rare, &tail_512, K_CANDIDATES).expect("in")))
    });
    g.bench_with_input(BenchmarkId::new("global_first_request", 512), &512usize, |b, _| {
        b.iter(|| black_box(fx.index.query_scored_top(&q, K_CANDIDATES).expect("top")))
    });
    for size in [8usize, 32, 128, 256, 512, 1000, 4000] {
        let tail: HashSet<NodeId> = fx.tail.iter().take(size).copied().collect();
        let head: HashSet<NodeId> = fx.head.iter().take(size).copied().collect();
        // Permitidos al final del ranking: el camino global agota el tope (y hoy pierde el resultado).
        g.bench_with_input(BenchmarkId::new("global_escalation_tail", size), &size, |b, _| {
            b.iter(|| black_box(global_escalation(&fx.index, &q, &tail)))
        });
        // Permitidos al principio: el camino global llena con la primera petición (su mejor caso).
        g.bench_with_input(BenchmarkId::new("global_escalation_head", size), &size, |b, _| {
            b.iter(|| black_box(global_escalation(&fx.index, &q, &head)))
        });
        g.bench_with_input(BenchmarkId::new("term_set_tail", size), &size, |b, _| {
            b.iter(|| black_box(fx.index.query_scored_in(&q, &tail, K_CANDIDATES).expect("in")))
        });
        g.bench_with_input(BenchmarkId::new("term_set_head", size), &size, |b, _| {
            b.iter(|| black_box(fx.index.query_scored_in(&q, &head, K_CANDIDATES).expect("in")))
        });
    }
    g.finish();
}

criterion_group!(benches, bench_text_filter);
criterion_main!(benches);
