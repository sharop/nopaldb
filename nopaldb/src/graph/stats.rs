//! Observabilidad operativa (#158): el estado de la base en una sola llamada
//! ([`Graph::stats`](super::Graph::stats)) y un evento de progreso para las
//! operaciones que tardan ([`Graph::set_progress_callback`](super::Graph::set_progress_callback)).
//!
//! Hasta 0.6.4 casi todo esto existía y se tiraba: el replay del WAL contaba
//! sus operaciones y las escribía en un `log::info!`, el checkpoint no dejaba
//! rastro, el GC automático solo se veía en el log, y saber cuánto tardó cada
//! fase de `open` exigía un cronómetro fuera. Quien opera desde Python no
//! podía distinguir "esto va lento" de "esto está parado". Este módulo
//! conserva lo que `open`, `checkpoint` y `gc` ya calculan y lo devuelve
//! estructurado; los `log::info!` se mantienen tal cual.
//!
//! No es telemetría: sin Prometheus, sin histogramas, sin tracing por
//! consulta. Quien quiera métricas las construye a partir del reporte.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use super::{AutoGcConfig, EmbeddingIndexStats};

// ─── Progreso ───────────────────────────────────────────────────────────────

/// Un evento de progreso de una operación larga.
///
/// `phase` es un nombre estable (`"wal_replay"`, `"adjacency_rebuild"`,
/// `"index_load"`, `"index_build"`, `"property_index_rebuild"`,
/// `"bulk_load"`, `"upsert_batch"`); `done` cuenta ítems procesados hasta el
/// momento y `total` es `None` cuando no se conoce por adelantado (una carga
/// masiva no sabe cuántas filas vienen). El último evento de una fase lleva
/// `done == total` cuando `total` es conocido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub phase: &'static str,
    pub done: u64,
    pub total: Option<u64>,
}

/// Callback de progreso. `Arc` y no `Box` porque el `Graph` se clona (el
/// applier, el GC automático) y todos los clones deben ver el mismo.
pub type ProgressCallback = Arc<dyn Fn(Progress) + Send + Sync>;

/// Cada cuántos ítems se emite como mínimo un evento, si el tiempo no lo
/// pidió antes.
const EMIT_EVERY_ITEMS: u64 = 1000;
/// Y cada cuánto tiempo, si los ítems no lo pidieron antes. Con lotes lentos
/// (un replay con fsync por registro) el reloj es el que manda.
const EMIT_EVERY_MS: u128 = 250;

/// Emisor de progreso para UNA fase. Sin callback registrado cuesta una
/// comparación por `tick` (el `Option` está resuelto al construirlo, no se
/// vuelve a consultar el `RwLock` del grafo en el bucle).
pub(crate) struct ProgressReporter {
    cb: Option<ProgressCallback>,
    phase: &'static str,
    total: Option<u64>,
    last_emit: Instant,
    next_at: u64,
}

impl ProgressReporter {
    /// Construye el emisor y, si hay callback, anuncia la fase con `done = 0`
    /// (así un `open` tras un crash largo deja de ser silencio hasta el final).
    pub(crate) fn new(cb: Option<ProgressCallback>, phase: &'static str, total: Option<u64>) -> Self {
        let mut r = Self { cb, phase, total, last_emit: Instant::now(), next_at: 0 };
        r.emit(0);
        r.next_at = EMIT_EVERY_ITEMS;
        r
    }

    /// Reporta `done` ítems procesados; emite si tocan los ítems o el tiempo.
    #[inline]
    pub(crate) fn tick(&mut self, done: u64) {
        if self.cb.is_none() {
            return;
        }
        if done >= self.next_at || self.last_emit.elapsed().as_millis() >= EMIT_EVERY_MS {
            self.emit(done);
            self.next_at = done.saturating_add(EMIT_EVERY_ITEMS);
        }
    }

    /// Cierra la fase: emite siempre (si hay callback) con `total` conocido.
    pub(crate) fn finish(mut self, done: u64) {
        if self.cb.is_none() {
            return;
        }
        if self.total.is_none() {
            self.total = Some(done);
        }
        self.emit(done);
    }

    fn emit(&mut self, done: u64) {
        if let Some(cb) = &self.cb {
            cb(Progress { phase: self.phase, done, total: self.total });
            self.last_emit = Instant::now();
        }
    }
}

// ─── Estado operativo que el grafo conserva ─────────────────────────────────

/// Milisegundos del reloj de pared desde la época Unix.
pub(crate) fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Lo que `open`, `checkpoint` y `gc` dejan anotado para que `stats()` lo
/// devuelva después. Compartido por todos los clones del `Graph`.
#[derive(Default)]
pub(crate) struct OpsState {
    /// Lo que pasó en el `open` que creó este grafo. Se escribe una vez.
    pub(crate) recovery: OnceLock<RecoverySection>,
    /// Directorio de la base (`None` en memoria).
    pub(crate) data_dir: Option<PathBuf>,
    /// Checkpoints completados por este handle (manuales, automáticos y el
    /// de `close`).
    pub(crate) checkpoints: AtomicU64,
    /// Reloj de pared (ms Unix) del último checkpoint; `0` = ninguno aún.
    pub(crate) last_checkpoint_unix_ms: AtomicU64,
    /// Resumen del último ciclo de GC (manual o automático).
    pub(crate) last_gc: Mutex<Option<GcRun>>,
    /// Callback de progreso registrado, si alguno.
    pub(crate) progress: RwLock<Option<ProgressCallback>>,
}

impl OpsState {
    pub(crate) fn progress_callback(&self) -> Option<ProgressCallback> {
        self.progress.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn reporter(&self, phase: &'static str, total: Option<u64>) -> ProgressReporter {
        ProgressReporter::new(self.progress_callback(), phase, total)
    }

    pub(crate) fn note_checkpoint(&self) {
        self.checkpoints.fetch_add(1, Ordering::Relaxed);
        self.last_checkpoint_unix_ms.store(unix_ms_now(), Ordering::Relaxed);
    }

    pub(crate) fn note_gc(&self, run: GcRun) {
        *self.last_gc.lock().unwrap_or_else(|e| e.into_inner()) = Some(run);
    }
}

// ─── El reporte ─────────────────────────────────────────────────────────────

/// Estado operativo completo de una base abierta. Ver [`Graph::stats`](super::Graph::stats).
#[derive(Debug, Clone, PartialEq)]
pub struct StatsReport {
    pub graph: GraphSection,
    pub storage: StorageSection,
    pub wal: WalSection,
    pub recovery: RecoverySection,
    pub indexes: Vec<IndexSection>,
    /// Un elemento por modelo cuyo índice HNSW está en caché (se construye o
    /// se carga en la primera búsqueda; hasta entonces no aparece). Vacío en
    /// builds sin `embeddings-index`.
    pub hnsw: Vec<EmbeddingIndexStats>,
    pub gc: GcSection,
}

/// Tamaño y forma del grafo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphSection {
    pub total_nodes: usize,
    pub total_edges: usize,
    /// `total_edges / total_nodes`; `0.0` sin nodos.
    pub avg_degree: f64,
    pub nodes_per_label: BTreeMap<String, usize>,
    pub edges_per_type: BTreeMap<String, usize>,
}

/// Con qué se abrió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageSection {
    /// `"redb"` o `"sled"`.
    pub engine: &'static str,
    /// `"default"`, `"mobile"` o `"server"`.
    pub profile: &'static str,
    /// `None` en memoria.
    pub data_dir: Option<PathBuf>,
    pub read_only: bool,
}

/// El WAL y su checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalSection {
    /// Lo que el próximo `open` reproduciría si el proceso muriera ahora.
    pub bytes: u64,
    /// Umbral del checkpoint automático (`0` = solo manual y en `close`).
    pub checkpoint_threshold_bytes: u64,
    /// Checkpoints hechos por este handle desde que se abrió.
    pub checkpoints_this_session: u64,
    /// Reloj de pared (ms Unix) del último; `None` si aún no hubo.
    pub last_checkpoint_unix_ms: Option<u64>,
    /// `"process_crash"` o `"immediate"` (ver `DirectWriteDurability`).
    pub direct_write_durability: &'static str,
}

/// Lo que pasó en el `open` que creó este handle. Se calcula una vez y se
/// conserva; hasta 0.6.4 se tiraba tras escribirlo en el log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoverySection {
    /// Registros válidos leídos del WAL (una base cerrada limpiamente tiene
    /// 1: el `Checkpoint` que `close` deja).
    pub wal_records_read: usize,
    /// Operaciones commiteadas que hubo que volver a aplicar.
    pub operations_replayed: usize,
    /// Transacciones sin `Commit` en el WAL, descartadas.
    pub uncommitted_txs_discarded: usize,
    /// `true` si el WAL traía transacciones commiteadas sin aplicar: el
    /// proceso anterior no cerró limpio.
    pub crash_recovery: bool,
    /// `true` si la adyacencia se reconstruyó desde las aristas (por crash
    /// recovery o porque el keyspace de adyacencia estaba vacío).
    pub adjacency_rebuilt: bool,
    pub open_ms: OpenPhasesMs,
}

/// Duración de cada fase del `open`, en milisegundos. Las fases no son
/// contiguas (`total` incluye lo que queda entre ellas: relojes, migración
/// del índice de propiedades, taxonomía).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenPhasesMs {
    /// Abrir el motor KV, lock del directorio y migración de layout.
    pub storage: u64,
    /// Análisis del WAL más el replay.
    pub wal_replay: u64,
    /// Carga de la adyacencia y, si tocó, su reconstrucción desde aristas.
    pub adjacency: u64,
    /// Carga de los índices de usuario desde `indexes/metadata.bin`.
    pub indexes: u64,
    pub total: u64,
}

/// Un índice de usuario, con lo que `list_indexes` no daba: tamaño y
/// analizador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSection {
    pub name: String,
    pub label: String,
    pub property: String,
    /// `"Hash"`, `"BTree"`, `"FullText"` o `"Taxonomy"`.
    pub kind: String,
    /// Entradas indexadas.
    pub size: usize,
    /// Solo full-text: `"default"` o lo activo unido con `+`, el mismo
    /// formato que el informe de migración (`"spanish+stemming+stopwords"`).
    pub analyzer: Option<String>,
}

/// El garbage collector MVCC.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcSection {
    /// `true` mientras el scheduler de `start_auto_gc` está vivo.
    pub auto_running: bool,
    /// Configuración del scheduler, si se arrancó.
    pub auto: Option<GcAutoSummary>,
    /// Último ciclo (manual o automático); `None` si no hubo ninguno.
    pub last_run: Option<GcRun>,
}

/// `AutoGcConfig` aplanada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcAutoSummary {
    pub interval_secs: u64,
    pub cutoff_timestamp: u64,
    pub min_versions_to_keep: usize,
    pub max_nodes_per_cycle: usize,
    pub dry_run: bool,
    pub use_active_horizon: bool,
}

impl From<&AutoGcConfig> for GcAutoSummary {
    fn from(c: &AutoGcConfig) -> Self {
        Self {
            interval_secs: c.interval_secs,
            cutoff_timestamp: c.gc_config.cutoff_timestamp,
            min_versions_to_keep: c.gc_config.min_versions_to_keep,
            max_nodes_per_cycle: c.gc_config.max_nodes_per_cycle,
            dry_run: c.gc_config.dry_run,
            use_active_horizon: c.gc_config.use_active_horizon,
        }
    }
}

/// Un ciclo de GC terminado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcRun {
    /// Reloj de pared (ms Unix) al terminar.
    pub unix_ms: u64,
    pub nodes_scanned: usize,
    pub versions_removed: usize,
    pub bytes_freed: usize,
    pub duration_ms: u64,
    /// `true` si solo contó (no borró nada).
    pub dry_run: bool,
}

// ─── Texto para el CLI ──────────────────────────────────────────────────────

/// Renderiza el reporte como texto de terminal (`nopaldb stats <dir>`).
pub fn render(r: &StatsReport) -> String {
    let mut out = String::new();
    let dir = r.storage.data_dir.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "(memoria)".into());
    out.push_str(&format!(
        "Base: {dir}\n  motor: {}  perfil: {}  solo lectura: {}\n",
        r.storage.engine,
        r.storage.profile,
        if r.storage.read_only { "sí" } else { "no" }
    ));
    out.push_str(&format!(
        "\nGrafo: {} nodos, {} aristas, grado medio {:.2}\n",
        r.graph.total_nodes, r.graph.total_edges, r.graph.avg_degree
    ));
    for (label, n) in &r.graph.nodes_per_label {
        out.push_str(&format!("  {label:<28} {n} nodos\n"));
    }
    for (t, n) in &r.graph.edges_per_type {
        out.push_str(&format!("  {t:<28} {n} aristas\n"));
    }

    out.push_str(&format!(
        "\nWAL: {} bytes (umbral de checkpoint automático: {})\n",
        r.wal.bytes,
        if r.wal.checkpoint_threshold_bytes == 0 { "desactivado".to_string() } else { format!("{} bytes", r.wal.checkpoint_threshold_bytes) }
    ));
    out.push_str(&format!(
        "  checkpoints en esta sesión: {}  último: {}  durabilidad de escrituras directas: {}\n",
        r.wal.checkpoints_this_session,
        r.wal.last_checkpoint_unix_ms.map(|ms| format!("{ms} ms Unix")).unwrap_or_else(|| "ninguno".into()),
        r.wal.direct_write_durability
    ));

    let rec = &r.recovery;
    out.push_str(&format!(
        "\nÚltimo open: {} ms en total (storage {} · replay WAL {} · adyacencia {} · índices {})\n",
        rec.open_ms.total, rec.open_ms.storage, rec.open_ms.wal_replay, rec.open_ms.adjacency, rec.open_ms.indexes
    ));
    out.push_str(&format!(
        "  registros WAL leídos: {}  operaciones reproducidas: {}  txs sin commit descartadas: {}\n  crash recovery: {}  adyacencia reconstruida: {}\n",
        rec.wal_records_read,
        rec.operations_replayed,
        rec.uncommitted_txs_discarded,
        if rec.crash_recovery { "sí" } else { "no" },
        if rec.adjacency_rebuilt { "sí" } else { "no" }
    ));

    out.push_str("\nÍndices de usuario:");
    if r.indexes.is_empty() {
        out.push_str(" ninguno\n");
    } else {
        out.push('\n');
        for ix in &r.indexes {
            out.push_str(&format!("  {:<28} {:<9} {}.{}  {} entradas", ix.name, ix.kind, ix.label, ix.property, ix.size));
            if let Some(a) = &ix.analyzer {
                out.push_str(&format!("  analizador: {a}"));
            }
            out.push('\n');
        }
    }

    out.push_str("\nÍndices HNSW en caché:");
    if r.hnsw.is_empty() {
        out.push_str(" ninguno (se construyen o cargan en la primera búsqueda)\n");
    } else {
        out.push('\n');
        for h in &r.hnsw {
            out.push_str(&format!(
                "  {:<28} {} puntos, {} tombstones, dim {}, rebuild pendiente: {}, en disco: {}{}\n",
                h.model,
                h.size,
                h.tombstones,
                h.dimension,
                if h.needs_rebuild { "sí" } else { "no" },
                if h.persisted { "sí" } else { "no" },
                h.loaded_from_disk_ms.map(|ms| format!(", cargado en {ms} ms")).unwrap_or_default()
            ));
        }
    }

    out.push_str(&format!("\nGC: automático {}", if r.gc.auto_running { "corriendo" } else { "parado" }));
    if let Some(a) = &r.gc.auto {
        out.push_str(&format!(
            " (cada {} s, cutoff {}, conserva {} versiones, máx {} nodos/ciclo{}{})",
            a.interval_secs,
            a.cutoff_timestamp,
            a.min_versions_to_keep,
            a.max_nodes_per_cycle,
            if a.dry_run { ", dry run" } else { "" },
            if a.use_active_horizon { ", horizonte activo" } else { "" }
        ));
    }
    out.push('\n');
    match &r.gc.last_run {
        Some(run) => out.push_str(&format!(
            "  último ciclo: {} ms Unix, {} nodos escaneados, {} versiones retiradas{}, {} bytes, {} ms\n",
            run.unix_ms,
            run.nodes_scanned,
            run.versions_removed,
            if run.dry_run { " (dry run)" } else { "" },
            run.bytes_freed,
            run.duration_ms
        )),
        None => out.push_str("  ningún ciclo todavía\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reporter_without_callback_is_silent_and_cheap() {
        let mut r = ProgressReporter::new(None, "x", Some(10));
        for i in 0..10 {
            r.tick(i);
        }
        r.finish(10);
    }

    #[test]
    fn reporter_emits_start_every_n_and_finish() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let cb: ProgressCallback = Arc::new(move |p| sink.lock().unwrap().push(p));
        let mut r = ProgressReporter::new(Some(cb), "wal_replay", None);
        for i in 1..=2500 {
            r.tick(i);
        }
        r.finish(2500);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first().unwrap().done, 0, "announces the phase");
        assert!(seen.iter().any(|p| p.done == 1000));
        assert!(seen.iter().any(|p| p.done == 2000));
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.total), (2500, Some(2500)), "finish fills in the total");
        assert!(seen.iter().all(|p| p.phase == "wal_replay"));
    }
}
