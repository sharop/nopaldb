// Copia verificada de una base entre motores KV (sled ↔ redb).
//
// Los índices, versiones MVCC y metadata son BYTES para esta capa: la copia
// es keyspace por keyspace, par por par, sin interpretar nada — por eso el
// resultado es byte-idéntico y el time-travel sobrevive intacto. La
// verificación es doble: conteo de pares y checksum FNV-1a 64 del stream
// `len(k)‖k‖len(v)‖v` en orden de iteración, recalculado con un re-scan del
// destino. NO se usa el export/import de alto nivel (Parquet es lossy) ni
// el export nativo de ningún motor (no portable).
//
// Precondición: la base origen debe estar CERRADA y con el WAL aplicado —
// abre y cierra el `Graph` normalmente antes de migrar (el replay corre en
// el open). Los locks de cada motor impiden copiar una base abierta por
// otro proceso; el sentinel de engine impide equivocarse de motor.
//
// No todo lo que una base necesita vive en el KV (#152). Los índices de
// usuario (hash/btree/full-text/taxonomy) guardan su catálogo en
// `<dir>/indexes/metadata.bin` y, los full-text, sus segmentos tantivy y el
// `analyzer.json` en `<dir>/indexes/fulltext_<nombre>/`; el índice HNSW
// persistido vive en `<dir>/hnsw/`. Hasta 0.6.2 la copia los ignoraba y el
// destino abría sin índices, en silencio, con `verified=true`. Ahora se
// copian archivo por archivo (verificados por tamaño y checksum) y el
// informe dice qué índices viajaron y si el HNSW viajó o se reconstruirá.

use std::path::{Path, PathBuf};

use crate::error::{Result, StorageError, StorageErrorKind};
use crate::storage::backend::StorageOptions;

use super::{KvEngine, WriteBatch};

/// Los keyspaces que componen una base. Fuente de verdad ÚNICA para la
/// migración; si Storage abre un keyspace nuevo, se agrega aquí o la copia
/// quedará incompleta (el test de round-trip lo detectaría).
///
/// `default` SE QUEDA aunque el layout v2 (F5) lo vacíe: copiar una base
/// legacy (pre-migración de layout) debe seguir funcionando — la matriz
/// layout×backend está desacoplada a propósito (copiar una base v1 y abrirla
/// después la migra de layout in-place). En una base ya migrada los
/// keyspaces vacíos son no-ops de la copia.
pub(crate) const ALL_KEYSPACES: &[&str] = &[
    super::DEFAULT_KEYSPACE,
    "edges",
    "versioned_edges",
    "versioned_edges_current",
    "prop_idx_v2",
    "embeddings",
    "path_ref_embeddings",
    // Layout v2 (F5): catálogo/entidades/historia/adyacencia/índices.
    "catalog",
    "entities",
    "history",
    "adjacency",
    "indexes",
];

const BATCH_PAIRS: usize = 10_000;

/// Subdirectorio de los índices de usuario (el que abre `IndexManager`).
pub(crate) const INDEXES_SUBDIR: &str = "indexes";
/// Subdirectorio del dump HNSW (`embeddings::persistence::DUMP_SUBDIR`; el
/// test de abajo mantiene los dos iguales sin acoplar este módulo a la
/// feature `embeddings-index`).
pub(crate) const HNSW_SUBDIR: &str = "hnsw";

/// Un directorio auxiliar copiado junto al KV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarReport {
    /// `indexes` o `hnsw`.
    pub dir: String,
    pub files: u64,
    pub bytes: u64,
}

/// Un índice de usuario tal como lo declara `indexes/metadata.bin` del
/// origen: es lo que el destino tendrá al abrir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSummary {
    pub name: String,
    pub label: String,
    pub property: String,
    /// `Hash`, `BTree`, `FullText` o `Taxonomy`.
    pub kind: String,
    /// Solo full-text: el analizador leído de su `analyzer.json`
    /// (`"default"` si el índice es anterior a 0.5.13 y no tiene sidecar).
    pub analyzer: Option<String>,
}

/// Resultado de una copia entre motores, por keyspace y con verificación.
///
/// Tres secciones, porque tres cosas distintas pueden faltar: los datos
/// (`keyspaces`, `verified`), los índices de usuario (`indexes`, copiados
/// dentro del sidecar `indexes`) y el índice vectorial (`hnsw_copied`; si
/// es `false` no había dump en el origen y el destino lo reconstruye desde
/// el keyspace `embeddings` en la primera búsqueda, que es lo que también
/// haría el origen).
#[derive(Debug, Clone)]
pub struct MigrationReport {
    /// (keyspace, pares copiados, bytes de payload copiados)
    pub keyspaces: Vec<(String, u64, u64)>,
    /// El re-scan del destino reprodujo conteos y checksums del origen, y
    /// cada archivo de los sidecars copiados coincide en tamaño y checksum.
    pub verified: bool,
    /// Directorios auxiliares copiados (`indexes`, `hnsw`), solo los que
    /// existían en el origen.
    pub sidecars: Vec<SidecarReport>,
    /// Índices de usuario que viajaron con el sidecar `indexes`.
    pub indexes: Vec<IndexSummary>,
    /// El dump HNSW existía en el origen y se copió.
    pub hnsw_copied: bool,
}

impl MigrationReport {
    pub fn total_pairs(&self) -> u64 {
        self.keyspaces.iter().map(|(_, n, _)| n).sum()
    }
    pub fn total_bytes(&self) -> u64 {
        self.keyspaces.iter().map(|(_, _, b)| b).sum()
    }
}

/// Checksum de un archivo con el mismo FNV-1a del KV.
fn file_digest(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut hash = Fnv1a::new();
    let mut len = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        len += n as u64;
    }
    Ok((len, hash.0))
}

fn io_err(ctx: &str, path: &Path, e: std::io::Error) -> crate::error::NopalError {
    StorageError::new(
        StorageErrorKind::Io,
        format!("migración: {ctx} {}: {e}", path.display()),
    )
    .into()
}

/// Archivos de `root`, recursivo, relativos a `root`, en orden estable. Los
/// `.lock` de tantivy se saltan: son artefactos del proceso que tuvo el
/// índice abierto, no parte del índice.
fn walk_files(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let dir = root.join(rel);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| io_err("no se pudo leer", &dir, e))?
        .collect::<std::io::Result<_>>()
        .map_err(|e| io_err("no se pudo leer", &dir, e))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let rel_path = rel.join(&name);
        let ty = entry.file_type().map_err(|e| io_err("no se pudo inspeccionar", &root.join(&rel_path), e))?;
        if ty.is_dir() {
            walk_files(root, &rel_path, out)?;
        } else if ty.is_file() && !name.to_string_lossy().ends_with(".lock") {
            out.push(rel_path);
        }
    }
    Ok(())
}

/// Copia `src_dir/<sub>` a `dst_dir/<sub>` archivo por archivo y verifica
/// cada copia (tamaño y checksum releídos del destino). `None` si el origen
/// no tiene ese subdirectorio. Error si el destino ya lo tiene con
/// contenido: la migración no mezcla.
fn copy_sidecar(src_dir: &Path, dst_dir: &Path, sub: &str) -> Result<Option<SidecarReport>> {
    let src = src_dir.join(sub);
    if !src.is_dir() {
        return Ok(None);
    }
    let dst = dst_dir.join(sub);
    if dst.is_dir() && std::fs::read_dir(&dst).map_err(|e| io_err("no se pudo leer", &dst, e))?.next().is_some() {
        return Err(StorageError::new(
            StorageErrorKind::InvalidData,
            format!(
                "el destino no está vacío (`{sub}/` tiene archivos); la migración no mezcla bases"
            ),
        )
        .into());
    }

    let mut files = Vec::new();
    walk_files(&src, Path::new(""), &mut files)?;
    let mut bytes = 0u64;
    for rel in &files {
        let from = src.join(rel);
        let to = dst.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("no se pudo crear", parent, e))?;
        }
        std::fs::copy(&from, &to).map_err(|e| io_err("no se pudo copiar", &from, e))?;
        let (src_len, src_hash) = file_digest(&from).map_err(|e| io_err("no se pudo leer", &from, e))?;
        let (dst_len, dst_hash) = file_digest(&to).map_err(|e| io_err("no se pudo releer", &to, e))?;
        if src_len != dst_len || src_hash != dst_hash {
            return Err(StorageError::new(
                StorageErrorKind::Corruption,
                format!(
                    "migración: `{sub}/{}` no verifica tras copiar (bytes {src_len}→{dst_len}); el destino NO debe usarse",
                    rel.display()
                ),
            )
            .into());
        }
        bytes += src_len;
    }
    // Un directorio vacío también viaja (p. ej. `indexes/` sin índices).
    std::fs::create_dir_all(&dst).map_err(|e| io_err("no se pudo crear", &dst, e))?;
    Ok(Some(SidecarReport { dir: sub.to_string(), files: files.len() as u64, bytes }))
}

/// Los índices de usuario que declara `indexes/metadata.bin` del origen, con
/// el analizador de cada full-text leído de su `analyzer.json`.
fn summarize_indexes(src_dir: &Path) -> Result<Vec<IndexSummary>> {
    use crate::index::{FullTextAnalyzer, IndexType};
    let indexes_dir = src_dir.join(INDEXES_SUBDIR);
    let metadata = indexes_dir.join("metadata.bin");
    if !metadata.is_file() {
        return Ok(Vec::new());
    }
    let mut metas = crate::index::storage::load_metadata(&metadata)?;
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = Vec::with_capacity(metas.len());
    for meta in metas {
        let kind = match meta.index_type {
            IndexType::Hash => "Hash",
            IndexType::BTree => "BTree",
            IndexType::FullText => "FullText",
            IndexType::Taxonomy => "Taxonomy",
        };
        let analyzer = if meta.index_type == IndexType::FullText {
            let file = indexes_dir.join(format!("fulltext_{}", meta.name)).join("analyzer.json");
            let analyzer: FullTextAnalyzer = if file.is_file() {
                let bytes = std::fs::read(&file).map_err(|e| io_err("no se pudo leer", &file, e))?;
                serde_json::from_slice(&bytes).map_err(|e| {
                    StorageError::new(
                        StorageErrorKind::InvalidData,
                        format!("migración: {} no es un analizador válido: {e}", file.display()),
                    )
                })?
            } else {
                FullTextAnalyzer::default()
            };
            Some(describe_analyzer(&analyzer))
        } else {
            None
        };
        out.push(IndexSummary {
            name: meta.name,
            label: meta.label,
            property: meta.property,
            kind: kind.to_string(),
            analyzer,
        });
    }
    Ok(out)
}

/// `"default"` o `"spanish+stemming+stopwords+ascii_folding"` (lo activo).
/// El mismo texto que la sección `indexes` de `Graph::stats` (#158).
fn describe_analyzer(a: &crate::index::FullTextAnalyzer) -> String {
    a.describe()
}

/// FNV-1a 64 streaming — determinista, sin dependencia nueva. No es
/// criptográfico: detecta corrupción/omisión, no adversarios. Compartido con
/// la verificación de identidad de la migración de layout
/// (`storage::layout_migrate`).
pub(crate) struct Fnv1a(pub(crate) u64);

impl Fnv1a {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn pair(&mut self, k: &[u8], v: &[u8]) {
        self.update(&(k.len() as u64).to_be_bytes());
        self.update(k);
        self.update(&(v.len() as u64).to_be_bytes());
        self.update(v);
    }
}

/// Escanea un keyspace completo: (pares, bytes, checksum).
fn scan_digest(engine: &dyn KvEngine, name: &str) -> Result<(u64, u64, u64)> {
    let ks = engine.keyspace(name)?;
    let mut pairs = 0u64;
    let mut bytes = 0u64;
    let mut hash = Fnv1a::new();
    for item in ks.iter() {
        let (k, v) = item?;
        hash.pair(&k, &v);
        pairs += 1;
        bytes += (k.len() + v.len()) as u64;
    }
    Ok((pairs, bytes, hash.0))
}

pub(crate) fn copy_between_engines(
    src: &dyn KvEngine,
    dst: &dyn KvEngine,
) -> Result<MigrationReport> {
    // Destino vacío o nada: una migración jamás mezcla datos en silencio.
    for name in ALL_KEYSPACES {
        let ks = dst.keyspace(name)?;
        if ks.iter().next().is_some() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidData,
                format!(
                    "el destino no está vacío (keyspace `{name}` tiene datos); la migración no mezcla bases"
                ),
            )
            .into());
        }
    }

    let mut report = Vec::with_capacity(ALL_KEYSPACES.len());
    let mut src_digests = Vec::with_capacity(ALL_KEYSPACES.len());

    for name in ALL_KEYSPACES {
        let src_ks = src.keyspace(name)?;
        let dst_ks = dst.keyspace(name)?;

        let mut pairs = 0u64;
        let mut bytes = 0u64;
        let mut hash = Fnv1a::new();
        let mut batch = WriteBatch::default();
        let mut in_batch = 0usize;

        for item in src_ks.iter() {
            let (k, v) = item?;
            hash.pair(&k, &v);
            pairs += 1;
            bytes += (k.len() + v.len()) as u64;
            batch.insert(k, v);
            in_batch += 1;
            if in_batch >= BATCH_PAIRS {
                dst_ks.apply_batch(std::mem::take(&mut batch))?;
                in_batch = 0;
            }
        }
        if in_batch > 0 {
            dst_ks.apply_batch(batch)?;
        }

        src_digests.push((pairs, hash.0));
        report.push(((*name).to_string(), pairs, bytes));
    }

    dst.flush()?;

    // Verificación: re-scan del DESTINO contra los digests del origen.
    let mut verified = true;
    for (name, (src_pairs, src_hash)) in ALL_KEYSPACES.iter().zip(&src_digests) {
        let (dst_pairs, _, dst_hash) = scan_digest(dst, name)?;
        if dst_pairs != *src_pairs || dst_hash != *src_hash {
            verified = false;
            log::error!(
                "migración: keyspace `{name}` no verifica (pares {src_pairs}→{dst_pairs}, checksum {})",
                if dst_hash == *src_hash { "ok" } else { "DIFIERE" }
            );
        }
    }

    Ok(MigrationReport {
        keyspaces: report,
        verified,
        sidecars: Vec::new(),
        indexes: Vec::new(),
        hnsw_copied: false,
    })
}

/// Abre origen y destino según sus opciones y copia todo, verificado.
pub(crate) fn copy_database_dirs(
    src_dir: &Path,
    src_opts: StorageOptions,
    dst_dir: &Path,
    dst_opts: StorageOptions,
) -> Result<MigrationReport> {
    // `Auto` en el origen exige que haya una base que detectar: migrar
    // "nada" a un destino nuevo sería crear dos bases vacías sin avisar.
    if src_opts.engine == crate::storage::backend::StorageEngine::Auto
        && super::detect_engine(src_dir).is_none()
    {
        return Err(StorageError::new(
            StorageErrorKind::InvalidData,
            format!("no hay una base NopalDB en {} (ni sled ni redb)", src_dir.display()),
        )
        .into());
    }
    // Los sidecars se comprueban ANTES de tocar el KV: si el destino ya
    // tiene `indexes/` o `hnsw/`, mejor fallar sin haber escrito nada.
    for sub in [INDEXES_SUBDIR, HNSW_SUBDIR] {
        let dst = dst_dir.join(sub);
        if dst.is_dir() && std::fs::read_dir(&dst).map_err(|e| io_err("no se pudo leer", &dst, e))?.next().is_some() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidData,
                format!("el destino no está vacío (`{sub}/` tiene archivos); la migración no mezcla bases"),
            )
            .into());
        }
    }
    let src = super::open_engine(src_dir, src_opts.profile, &src_opts)?;
    let dst = super::open_engine(dst_dir, dst_opts.profile, &dst_opts)?;
    let mut report = copy_between_engines(src.as_ref(), dst.as_ref())?;
    if !report.verified {
        return Err(StorageError::new(
            StorageErrorKind::Corruption,
            "la verificación post-copia falló (ver logs); el destino NO debe usarse",
        )
        .into());
    }
    // Soltar los motores antes de copiar sidecars: el destino se abre
    // después con `Graph::open`, que es quien carga índices y HNSW.
    drop(src);
    drop(dst);

    report.indexes = summarize_indexes(src_dir)?;
    if let Some(sc) = copy_sidecar(src_dir, dst_dir, INDEXES_SUBDIR)? {
        report.sidecars.push(sc);
    }
    if let Some(sc) = copy_sidecar(src_dir, dst_dir, HNSW_SUBDIR)? {
        report.hnsw_copied = sc.files > 0;
        report.sidecars.push(sc);
    }
    Ok(report)
}

#[cfg(test)]
mod sidecar_tests {
    use super::*;

    #[cfg(feature = "embeddings-index")]
    #[test]
    fn hnsw_subdir_matches_the_persistence_module() {
        assert_eq!(HNSW_SUBDIR, crate::embeddings::persistence::DUMP_SUBDIR);
    }

    #[test]
    fn indexes_subdir_matches_the_index_manager_path() {
        // `Graph::open` hace `path.join("indexes")`; si cambia, la copia
        // dejaría de encontrar el catálogo.
        assert_eq!(INDEXES_SUBDIR, "indexes");
    }

    #[test]
    fn copy_sidecar_copies_nested_files_skips_locks_and_verifies() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let idx = src.path().join("indexes");
        std::fs::create_dir_all(idx.join("fulltext_a")).unwrap();
        std::fs::write(idx.join("metadata.bin"), b"meta").unwrap();
        std::fs::write(idx.join("fulltext_a/analyzer.json"), b"{}").unwrap();
        std::fs::write(idx.join("fulltext_a/.tantivy-writer.lock"), b"").unwrap();

        let sc = copy_sidecar(src.path(), dst.path(), "indexes").unwrap().unwrap();
        assert_eq!(sc, SidecarReport { dir: "indexes".into(), files: 2, bytes: 6 });
        assert!(dst.path().join("indexes/fulltext_a/analyzer.json").is_file());
        assert!(!dst.path().join("indexes/fulltext_a/.tantivy-writer.lock").exists());

        assert!(copy_sidecar(src.path(), dst.path(), "hnsw").unwrap().is_none());
        // Destino ya poblado: se rechaza.
        assert!(copy_sidecar(src.path(), dst.path(), "indexes").is_err());
    }

    #[test]
    fn describe_analyzer_lists_what_is_active() {
        use crate::index::FullTextAnalyzer;
        assert_eq!(describe_analyzer(&FullTextAnalyzer::default()), "default");
        let a = FullTextAnalyzer { language: Some("spanish".into()), stemming: true, stopwords: false, ascii_folding: true };
        assert_eq!(describe_analyzer(&a), "spanish+stemming+ascii_folding");
    }
}
