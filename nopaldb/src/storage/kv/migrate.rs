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

use std::path::Path;

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

/// Resultado de una copia entre motores, por keyspace y con verificación.
#[derive(Debug, Clone)]
pub struct MigrationReport {
    /// (keyspace, pares copiados, bytes de payload copiados)
    pub keyspaces: Vec<(String, u64, u64)>,
    /// El re-scan del destino reprodujo conteos y checksums del origen.
    pub verified: bool,
}

impl MigrationReport {
    pub fn total_pairs(&self) -> u64 {
        self.keyspaces.iter().map(|(_, n, _)| n).sum()
    }
    pub fn total_bytes(&self) -> u64 {
        self.keyspaces.iter().map(|(_, _, b)| b).sum()
    }
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
    })
}

/// Abre origen y destino según sus opciones y copia todo, verificado.
pub(crate) fn copy_database_dirs(
    src_dir: &Path,
    src_opts: StorageOptions,
    dst_dir: &Path,
    dst_opts: StorageOptions,
) -> Result<MigrationReport> {
    let src = super::open_engine(src_dir, src_opts.profile, &src_opts)?;
    let dst = super::open_engine(dst_dir, dst_opts.profile, &dst_opts)?;
    let report = copy_between_engines(src.as_ref(), dst.as_ref())?;
    if !report.verified {
        return Err(StorageError::new(
            StorageErrorKind::Corruption,
            "la verificación post-copia falló (ver logs); el destino NO debe usarse",
        )
        .into());
    }
    Ok(report)
}
