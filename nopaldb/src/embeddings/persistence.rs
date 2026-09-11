// src/embeddings/persistence.rs
//! Persistencia del índice HNSW: el grafo de `hnsw_rs` va a disco con su
//! `file_dump` nativo y NopalDB guarda al lado lo que el grafo no sabe: el
//! mapa `DataId → NodeId`, los tombstones y una huella de los embeddings que
//! el índice describe. Al reabrir, si la huella coincide con lo que hay en
//! storage, el índice se carga en vez de reconstruirse (169 ms frente a
//! 14 s con 50k vectores de 384 dimensiones; #114).
//!
//! # Archivos
//!
//! En `<data_dir>/hnsw/`, por modelo, con basename saneado
//! ([`basename_for`]):
//!
//! - `<base>.hnsw.graph` y `<base>.hnsw.data`: el dump de `hnsw_rs`.
//! - `<base>.meta`: `DumpMeta` en bincode. Se escribe **el último** y lleva
//!   longitud y FNV-1a de los otros dos, así que una escritura interrumpida
//!   en cualquier punto deja o el juego viejo completo o un `meta` que no
//!   casa con los archivos, y en ambos casos la carga lo detecta.
//!
//! Todo se escribe a nombres temporales y se renombra; nunca se trunca un
//! archivo que otro `open` pudiera estar leyendo.
//!
//! # Cuándo se escribe
//!
//! Tras construir el índice completo desde storage y en `Graph::close` si el
//! índice cambió desde el último dump (`HnswIndex::is_dirty`). No en cada
//! `insert`: un proceso que muere sin `close` reabre con la huella cambiada
//! y reconstruye, que es lo correcto; un dump por inserción solo añadiría
//! coste sin evitar ese rebuild. Solo se persisten índices con más de
//! [`EXACT_SEARCH_THRESHOLD`] puntos: por debajo la reconstrucción cuesta
//! milisegundos y el camino exacto necesita los vectores en memoria, que el
//! dump no trae de forma directa.
//!
//! # Por qué la carga verifica antes de llamar a `hnsw_rs`
//!
//! `HnswIo::load_hnsw` hace `unwrap`/`assert` sobre el contenido de los
//! archivos: un dump corrupto es un panic, y el perfil release del workspace
//! lleva `panic = "abort"`. Por eso `meta` guarda longitud y hash de los dos
//! archivos y [`load`] los comprueba primero; si no cuadran, reconstruye.
//!
//! # Por qué `Box::leak`
//!
//! `load_hnsw<'b, 'a>(&'a mut HnswIo) -> Hnsw<'b>` con `'a: 'b`: el grafo
//! cargado queda atado al `HnswIo` por si los datos vienen de un mmap. Aquí
//! el mmap está desactivado (`ReloadOptions::default`), así que el `Hnsw` no
//! toma prestado nada de él, pero el tipo sigue exigiendo que el `HnswIo`
//! viva tanto como el grafo. Se filtra con `Box::leak`: unos cientos de bytes
//! (`PathBuf`, `String`, opciones, un `Arc`) por carga, que en la práctica es
//! una vez por modelo y por `open`. Se descartaron: un crate de structs
//! autorreferenciales (dependencia nueva para un solo sitio), guardar el
//! `HnswIo` junto al `Hnsw` (mismo problema de autorreferencia) y transmutar
//! la lifetime (correcto solo mientras nadie active el mmap).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw, HnswIo};
use serde::{Deserialize, Serialize};

use super::index::{HnswIndex, EXACT_SEARCH_THRESHOLD};
use crate::error::NopalError;
use crate::types::NodeId;

/// Versión del formato de `<base>.meta`. Se incrementa si cambia
/// [`DumpMeta`]; un `meta` de otra versión se trata como ausente.
pub const DUMP_FORMAT_VERSION: u32 = 1;

/// Subdirectorio de `data_dir` donde viven los dumps.
pub const DUMP_SUBDIR: &str = "hnsw";

/// FNV-1a de 64 bits: rápido, sin dependencias, y solo tiene que detectar
/// que algo cambió (no resistir a un adversario).
pub struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub fn new() -> Self {
        Self(Self::OFFSET)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

/// Huella de los embeddings de un modelo en storage: cuántos y un hash de
/// claves y valores. La calcula `Storage::node_embeddings_digest_for_model_sync`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingsDigest {
    pub count: usize,
    pub hash: u64,
}

/// Longitud y hash de uno de los archivos del dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FileDigest {
    len: u64,
    hash: u64,
}

/// Lo que el grafo de `hnsw_rs` no guarda y NopalDB necesita para rearmar
/// el [`HnswIndex`].
#[derive(Serialize, Deserialize)]
struct DumpMeta {
    format_version: u32,
    model: String,
    dimension: usize,
    id_map: HashMap<usize, NodeId>,
    next_data_id: usize,
    tombstones: usize,
    /// Los embeddings que este dump describe.
    embeddings: EmbeddingsDigest,
    graph_file: FileDigest,
    data_file: FileDigest,
}

/// Resultado de [`dump`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpInfo {
    /// Directorio donde quedaron los archivos.
    pub dir: PathBuf,
    /// Bytes de los dos archivos de `hnsw_rs` (el `meta` aparte).
    pub bytes: u64,
}

/// Qué pasó al intentar cargar un dump.
pub enum LoadOutcome {
    /// El dump describe exactamente los embeddings actuales. En `Box` por el
    /// tamaño del índice frente a las otras variantes (clippy `large_enum_variant`).
    Loaded(Box<HnswIndex>),
    /// No hay dump para este modelo.
    Missing,
    /// Hay dump pero los embeddings cambiaron desde que se escribió.
    Stale(String),
    /// Los archivos no pasan la verificación o `hnsw_rs` no pudo leerlos.
    Corrupt(String),
}

/// `<data_dir>/hnsw`.
pub fn dump_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DUMP_SUBDIR)
}

/// Nombre de archivo estable y seguro para un modelo: los caracteres fuera
/// de `[A-Za-z0-9._-]` se sustituyen por `_` y se añade un hash corto del
/// nombre completo, así `openai/x` y `openai_x` no colisionan.
pub fn basename_for(model: &str) -> String {
    let safe: String = model
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let mut h = Fnv1a::new();
    h.write(model.as_bytes());
    format!("{}-{:08x}", safe, (h.finish() & 0xffff_ffff) as u32)
}

/// `true` si un índice de `len` puntos vivos se persiste. Ver el doc del
/// módulo.
pub fn is_persistable(len: usize) -> bool {
    len > EXACT_SEARCH_THRESHOLD
}

fn meta_path(dir: &Path, base: &str) -> PathBuf {
    dir.join(format!("{base}.meta"))
}

fn graph_path(dir: &Path, base: &str) -> PathBuf {
    dir.join(format!("{base}.hnsw.graph"))
}

fn data_path(dir: &Path, base: &str) -> PathBuf {
    dir.join(format!("{base}.hnsw.data"))
}

fn io_err(ctx: &str, e: std::io::Error) -> NopalError {
    NopalError::custom(format!("HnswPersistence::{ctx}: {e}"))
}

fn file_digest(path: &Path) -> std::io::Result<FileDigest> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Fnv1a::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        len += n as u64;
    }
    Ok(FileDigest { len, hash: h.finish() })
}

/// Escribe el dump de `index` en `<data_dir>/hnsw` y lo marca limpio.
///
/// `embeddings` es la huella de storage que el índice describe; el llamador
/// la calcula y comprueba que `embeddings.count == index.len()` antes de
/// llamar (aquí se vuelve a comprobar y es error si no cuadra: un dump que
/// certifique un estado que no tiene sería peor que ninguno).
pub fn dump(
    index: &mut HnswIndex,
    data_dir: &Path,
    embeddings: EmbeddingsDigest,
) -> Result<DumpInfo, NopalError> {
    if embeddings.count != index.len() {
        return Err(NopalError::custom(format!(
            "HnswPersistence::dump({}): el índice tiene {} puntos vivos y storage {} embeddings; no se escribe un dump que no los describa",
            index.model(),
            index.len(),
            embeddings.count
        )));
    }
    let dir = dump_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create_dir", e))?;
    let base = basename_for(index.model());
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp_base = format!(
        ".tmp-{}-{}-{}",
        base,
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );

    // `file_dump` devuelve el basename que usó de verdad: cuando el índice
    // se recargó de disco, `hnsw_rs` se niega a sobrescribir y elige un
    // sufijo aleatorio si el nombre ya existe. Con un temporal único no
    // ocurre, pero se respeta lo devuelto igualmente.
    let written = index
        .inner()
        .file_dump(&dir, &tmp_base)
        .map_err(|e| NopalError::custom(format!("HnswPersistence::dump: hnsw_rs file_dump: {e}")))?;
    let tmp_graph = graph_path(&dir, &written);
    let tmp_data = data_path(&dir, &written);

    let finish = (|| -> Result<DumpInfo, NopalError> {
        let graph_file = file_digest(&tmp_graph).map_err(|e| io_err("digest graph", e))?;
        let data_file = file_digest(&tmp_data).map_err(|e| io_err("digest data", e))?;
        std::fs::rename(&tmp_graph, graph_path(&dir, &base)).map_err(|e| io_err("rename graph", e))?;
        std::fs::rename(&tmp_data, data_path(&dir, &base)).map_err(|e| io_err("rename data", e))?;

        let meta = DumpMeta {
            format_version: DUMP_FORMAT_VERSION,
            model: index.model().to_string(),
            dimension: index.dimension(),
            id_map: index.id_map().clone(),
            next_data_id: index.next_data_id(),
            tombstones: index.tombstones(),
            embeddings,
            graph_file,
            data_file,
        };
        let bytes = bincode::serialize(&meta)
            .map_err(|e| NopalError::custom(format!("HnswPersistence::dump: meta: {e}")))?;
        let tmp_meta = dir.join(format!("{tmp_base}.meta"));
        std::fs::write(&tmp_meta, bytes).map_err(|e| io_err("write meta", e))?;
        std::fs::rename(&tmp_meta, meta_path(&dir, &base)).map_err(|e| io_err("rename meta", e))?;
        Ok(DumpInfo { dir: dir.clone(), bytes: graph_file.len + data_file.len })
    })();

    match finish {
        Ok(info) => {
            index.mark_clean();
            Ok(info)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_graph);
            let _ = std::fs::remove_file(&tmp_data);
            Err(e)
        }
    }
}

/// Carga el dump de `model` si existe y describe exactamente `expected`.
///
/// Nunca llama a `hnsw_rs` sobre archivos que no pasen longitud y hash
/// (ver el doc del módulo). Cualquier cosa distinta de `Loaded` deja al
/// llamador reconstruir desde storage.
pub fn load(model: &str, data_dir: &Path, expected: &EmbeddingsDigest) -> LoadOutcome {
    let dir = dump_dir(data_dir);
    let base = basename_for(model);
    let meta_path = meta_path(&dir, &base);
    let bytes = match std::fs::read(&meta_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(e) => return LoadOutcome::Corrupt(format!("leer {}: {e}", meta_path.display())),
    };
    let meta: DumpMeta = match bincode::deserialize(&bytes) {
        Ok(m) => m,
        Err(e) => return LoadOutcome::Corrupt(format!("meta ilegible: {e}")),
    };
    if meta.format_version != DUMP_FORMAT_VERSION {
        return LoadOutcome::Stale(format!(
            "formato {} (este binario escribe {})",
            meta.format_version, DUMP_FORMAT_VERSION
        ));
    }
    if meta.model != model {
        return LoadOutcome::Corrupt(format!("meta de otro modelo: {}", meta.model));
    }
    if meta.embeddings != *expected {
        return LoadOutcome::Stale(format!(
            "storage tiene {} embeddings (hash {:016x}); el dump describía {} ({:016x})",
            expected.count, expected.hash, meta.embeddings.count, meta.embeddings.hash
        ));
    }
    if meta.id_map.len() != meta.embeddings.count {
        return LoadOutcome::Corrupt(format!(
            "meta incoherente: {} ids para {} embeddings",
            meta.id_map.len(),
            meta.embeddings.count
        ));
    }
    for (label, path, want) in [
        ("graph", graph_path(&dir, &base), meta.graph_file),
        ("data", data_path(&dir, &base), meta.data_file),
    ] {
        match file_digest(&path) {
            Ok(got) if got == want => {}
            Ok(got) => {
                return LoadOutcome::Corrupt(format!(
                    "{label}: {} bytes/hash {:016x}, el meta esperaba {} bytes/hash {:016x}",
                    got.len, got.hash, want.len, want.hash
                ))
            }
            Err(e) => return LoadOutcome::Corrupt(format!("{label}: {e}")),
        }
    }

    let start = Instant::now();
    // Ver "Por qué `Box::leak`" en el doc del módulo.
    let io: &'static mut HnswIo = Box::leak(Box::new(HnswIo::new(&dir, &base)));
    let inner: Hnsw<'static, f32, DistCosine> = match io.load_hnsw() {
        Ok(h) => h,
        Err(e) => return LoadOutcome::Corrupt(format!("hnsw_rs load_hnsw: {e}")),
    };
    let expected_points = meta.id_map.len() + meta.tombstones;
    if inner.get_nb_point() != expected_points {
        return LoadOutcome::Corrupt(format!(
            "el grafo trae {} puntos y el meta {} ({} vivos + {} tombstones)",
            inner.get_nb_point(),
            expected_points,
            meta.id_map.len(),
            meta.tombstones
        ));
    }
    LoadOutcome::Loaded(Box::new(HnswIndex::from_parts(
        inner,
        meta.model,
        meta.dimension,
        meta.id_map,
        meta.next_data_id,
        meta.tombstones,
        start.elapsed(),
    )))
}

/// Borra el dump de `model`, si existe. Se usa cuando el índice deja de ser
/// persistible (bajó del umbral) para no dejar archivos que ya no describen
/// nada.
pub fn remove(model: &str, data_dir: &Path) -> Result<bool, NopalError> {
    let dir = dump_dir(data_dir);
    let base = basename_for(model);
    let mut removed = false;
    for p in [meta_path(&dir, &base), graph_path(&dir, &base), data_path(&dir, &base)] {
        match std::fs::remove_file(&p) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("remove", e)),
        }
    }
    Ok(removed)
}

/// `true` si hay un `meta` para `model` (sin verificarlo).
pub fn exists(model: &str, data_dir: &Path) -> bool {
    meta_path(&dump_dir(data_dir), &basename_for(model)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors(n: usize, dim: usize) -> Vec<(NodeId, Vec<f32>)> {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..n)
            .map(|i| {
                let v = (0..dim)
                    .map(|_| {
                        s ^= s >> 12;
                        s ^= s << 25;
                        s ^= s >> 27;
                        ((s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
                    })
                    .collect();
                (NodeId::from_u128(0xA000 + i as u128), v)
            })
            .collect()
    }

    #[test]
    fn basename_is_filesystem_safe_and_collision_free() {
        let a = basename_for("openai/x");
        let b = basename_for("openai_x");
        assert_ne!(a, b);
        assert!(a.starts_with("openai_x-"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')));
    }

    #[test]
    fn dump_then_load_roundtrip_preserves_ids_and_results() {
        let dir = tempfile::tempdir().unwrap();
        let data = vectors(EXACT_SEARCH_THRESHOLD + 200, 16);
        let mut index = HnswIndex::build_batch(data.clone(), "m", 16).unwrap();
        index.remove(data[3].0);
        assert!(index.is_dirty());
        let digest = EmbeddingsDigest { count: index.len(), hash: 42 };
        let info = dump(&mut index, dir.path(), digest).unwrap();
        assert!(!index.is_dirty());
        assert!(info.bytes > 0);
        assert!(exists("m", dir.path()));

        let q = &data[10].1;
        let before = index.search_knn(q, 5).unwrap();
        let LoadOutcome::Loaded(loaded) = load("m", dir.path(), &digest) else {
            panic!("expected Loaded")
        };
        assert_eq!(loaded.len(), index.len());
        assert_eq!(loaded.tombstones(), 1);
        assert!(!loaded.contains(data[3].0));
        assert!(!loaded.is_dirty());
        assert!(loaded.loaded_in().is_some());
        assert_eq!(loaded.search_knn(q, 5).unwrap(), before);
    }

    #[test]
    fn load_reports_missing_stale_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load("m", dir.path(), &EmbeddingsDigest { count: 0, hash: 0 }), LoadOutcome::Missing));

        let data = vectors(EXACT_SEARCH_THRESHOLD + 10, 8);
        let mut index = HnswIndex::build_batch(data, "m", 8).unwrap();
        let digest = EmbeddingsDigest { count: index.len(), hash: 1 };
        dump(&mut index, dir.path(), digest).unwrap();

        let other = EmbeddingsDigest { count: index.len(), hash: 2 };
        assert!(matches!(load("m", dir.path(), &other), LoadOutcome::Stale(_)));

        // Corromper el grafo en medio: el hash lo detecta antes de hnsw_rs.
        let gp = graph_path(&dump_dir(dir.path()), &basename_for("m"));
        let mut bytes = std::fs::read(&gp).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        std::fs::write(&gp, bytes).unwrap();
        assert!(matches!(load("m", dir.path(), &digest), LoadOutcome::Corrupt(_)));

        // Truncar (escritura interrumpida): también Corrupt.
        let f = std::fs::OpenOptions::new().write(true).open(&gp).unwrap();
        f.set_len(100).unwrap();
        assert!(matches!(load("m", dir.path(), &digest), LoadOutcome::Corrupt(_)));

        assert!(remove("m", dir.path()).unwrap());
        assert!(matches!(load("m", dir.path(), &digest), LoadOutcome::Missing));
    }

    #[test]
    fn dump_refuses_a_digest_that_does_not_match_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = HnswIndex::build_batch(vectors(EXACT_SEARCH_THRESHOLD + 1, 4), "m", 4).unwrap();
        let bad = EmbeddingsDigest { count: index.len() + 1, hash: 0 };
        assert!(dump(&mut index, dir.path(), bad).is_err());
        assert!(index.is_dirty());
        assert!(!exists("m", dir.path()));
    }
}
