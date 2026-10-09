// #206: hnsw_rs 0.3.4 reserva de más cuando recibe el número de puntos
// esperado como pista de capacidad (le falta un `exp` a la fracción por
// capa): con M = 24 son ~3.4 KB por punto que nunca se usan, 330 MiB para
// un índice vacío de 100k. `HnswIndex` le pasa 0. Este test mide con un
// allocator contador y falla si la reserva vuelve.
//
// Un solo `#[test]`: el contador es global al binario y dos tests en
// paralelo se sumarían.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nopaldb::embeddings::HnswIndex;
use nopaldb::types::NodeId;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const DIM: usize = 32;
const N: usize = 5_000;

fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 2001) as f32 / 1000.0 - 1.0
        })
        .collect()
}

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

#[test]
fn the_index_does_not_reserve_memory_it_never_uses() {
    // Vacío, con una pista de 100k: con la reserva de hnsw_rs eran 330 MiB.
    let before = live();
    let empty = HnswIndex::new("m", 384, 100_000);
    let used = live().saturating_sub(before);
    drop(empty);
    assert!(used < 1 << 20, "índice vacío: {used} bytes (esperado < 1 MiB)");

    // Construido: por punto, el vector (128 B a 32 dims), la estructura del
    // punto y sus aristas suman ~2 KB; con la reserva eran ~5.5 KB.
    let vectors: Vec<(NodeId, Vec<f32>)> =
        (0..N as u64).map(|i| (NodeId::new_v4(), vector(i))).collect();
    let before = live();
    let index = HnswIndex::build_batch(vectors, "m", DIM).unwrap();
    let per_point = live().saturating_sub(before) / N;
    drop(index);
    eprintln!("bytes por punto: {per_point}");
    assert!(
        per_point < 3_500,
        "índice construido: {per_point} bytes por punto (esperado < 3.5 KB)"
    );
}
