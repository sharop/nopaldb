//! Migración entre motores de almacenamiento, con verificación: la única
//! implementación detrás del binario `nopaldb migrate`, del ejemplo
//! `migrate_engine` y (vía `Storage::copy_database`) de `Graph.migrate` en
//! Python. Ver `docs/MIGRATION_0.6.md`.

use std::path::Path;

use crate::error::{NopalError, Result};
use crate::storage::{MigrationReport, Storage, StorageEngine, StorageOptions, StorageProfile};

/// Parsea `auto | sled | redb` (sin distinguir mayúsculas).
pub fn parse_engine(s: &str) -> Result<StorageEngine> {
    match s.to_ascii_lowercase().as_str() {
        "auto" => Ok(StorageEngine::Auto),
        "sled" => Ok(StorageEngine::Sled),
        "redb" => Ok(StorageEngine::Redb),
        other => Err(NopalError::custom(format!(
            "motor desconocido: {other} (usa auto, sled o redb)"
        ))),
    }
}

/// Parsea `default | mobile | server`.
pub fn parse_profile(s: &str) -> Result<StorageProfile> {
    match s.to_ascii_lowercase().as_str() {
        "default" => Ok(StorageProfile::Default),
        "mobile" => Ok(StorageProfile::Mobile),
        "server" => Ok(StorageProfile::Server),
        other => Err(NopalError::custom(format!(
            "perfil desconocido: {other} (usa default, mobile o server)"
        ))),
    }
}

/// Nombre corto de un motor para imprimir.
pub fn engine_name(engine: StorageEngine) -> &'static str {
    match engine {
        StorageEngine::Auto => "auto",
        StorageEngine::Sled => "sled",
        StorageEngine::Redb => "redb",
    }
}

/// Copia `src` a `dst` (vacío o inexistente) con verificación por keyspace.
/// `Auto` en el origen detecta el motor por los archivos del directorio (y
/// es error si no hay base); en el destino es el motor por defecto del
/// build. Precondiciones y semántica: `Storage::copy_database`.
pub async fn migrate(
    src: impl AsRef<Path>,
    src_engine: StorageEngine,
    dst: impl AsRef<Path>,
    dst_engine: StorageEngine,
    profile: StorageProfile,
) -> Result<MigrationReport> {
    let src_opts = StorageOptions { engine: src_engine, profile, ..Default::default() };
    let dst_opts = StorageOptions { engine: dst_engine, profile, ..Default::default() };
    Storage::copy_database(src, src_opts, dst, dst_opts).await
}

/// Tabla legible del informe: una línea por keyspace (pares, bytes),
/// totales y veredicto de la verificación.
pub fn render_report(report: &MigrationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("{:<28} {:>12} {:>14}\n", "keyspace", "pares", "bytes"));
    for (name, pairs, bytes) in &report.keyspaces {
        out.push_str(&format!("{name:<28} {pairs:>12} {bytes:>14}\n"));
    }
    out.push_str(&format!(
        "{:<28} {:>12} {:>14}\n",
        "total",
        report.total_pairs(),
        report.total_bytes()
    ));
    out.push_str(if report.verified {
        "Verificación: OK (conteos y checksums del destino coinciden con el origen)\n"
    } else {
        "Verificación: FALLÓ — no uses el destino\n"
    });

    out.push_str("\nÍndices de usuario:");
    if report.indexes.is_empty() {
        out.push_str(" ninguno declarado en el origen\n");
    } else {
        out.push('\n');
        for ix in &report.indexes {
            out.push_str(&format!("  {:<28} {:<9} {}.{}", ix.name, ix.kind, ix.label, ix.property));
            if let Some(a) = &ix.analyzer {
                out.push_str(&format!("  analizador: {a}"));
            }
            out.push('\n');
        }
    }
    for sc in &report.sidecars {
        out.push_str(&format!("  copiado `{}/`: {} archivos, {} bytes, verificados\n", sc.dir, sc.files, sc.bytes));
    }
    out.push_str(if report.hnsw_copied {
        "Índice HNSW: copiado (el destino lo carga de disco en la primera búsqueda)\n"
    } else {
        "Índice HNSW: sin dump en el origen; el destino lo reconstruye desde `embeddings` en la primera búsqueda\n"
    });
    out
}
