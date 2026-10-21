/// Runtime profile for storage tuning.
///
/// `#[non_exhaustive]`: new profiles can be added without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageProfile {
    Default,
    Mobile,
    Server,
}

/// Logical storage engine selector.
///
/// `#[non_exhaustive]`: new engines (behind their own feature flags) can be
/// added without a breaking change.
///
/// redb es el único motor desde 0.8.0 (sled se retiró, #141). `Auto` es lo
/// que usa `StorageOptions::default()` y resuelve a redb; existía para que
/// las bases sled de 0.5.x abrieran sin tocar código, y sigue mirando el
/// directorio para que una base sled falle con la receta de migración
/// (migrar con NopalDB 0.7) en vez de con un error críptico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StorageEngine {
    /// redb, salvo que el directorio contenga una base sled de 0.5.x: entonces
    /// un error con la receta de migración.
    #[default]
    Auto,
    /// Motor de 0.5.x, **retirado en 0.8.0** (#141). La variante queda para
    /// que pedirlo dé un error con la receta (migrar con NopalDB 0.7, la
    /// última versión que lee sled) en vez de no compilar en quien la nombre.
    Sled,
    /// Motor por defecto desde 0.6.0 (feature `storage-redb`).
    Redb,
}

/// Durabilidad de las escrituras **directas** (`add_node`, `add_edge`,
/// borrados, sin transacción). Desde 0.5.23 cada una se registra en el WAL
/// como una transacción automática antes de aplicarse; esta opción decide
/// cuándo ese registro llega al disco.
///
/// Las transacciones (`begin_transaction` + `commit`) siempre hacen fsync
/// por lote del applier; esta opción no las afecta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectWriteDurability {
    /// El registro se escribe al archivo del WAL sin fsync por operación;
    /// el fsync lo hacen un sincronizador periódico (cada `flush_every_ms`
    /// del perfil) y `close()`. Sobrevive a que el proceso muera (el sistema
    /// operativo conserva lo escrito); ante un apagón se pierde como mucho
    /// el último periodo. Coste por escritura: microsegundos.
    #[default]
    ProcessCrash,
    /// fsync por lote del applier, igual que las transacciones. Sobrevive a
    /// un apagón. Coste: el fsync (milisegundos por escritura aislada).
    Immediate,
}

/// Storage creation options.
///
/// Construir con `..Default::default()` para no depender de la lista de
/// campos (crece en versiones menores).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageOptions {
    pub engine: StorageEngine,
    pub profile: StorageProfile,
    /// Ver [`DirectWriteDurability`].
    pub direct_write_durability: DirectWriteDurability,
    /// Tamaño del WAL a partir del cual el applier hace un checkpoint
    /// automático (motor durable + WAL truncado) al terminar el lote en
    /// curso. `0` desactiva el automático: el WAL crece hasta que alguien
    /// llame a `Graph::checkpoint()` o a `close()`. Ver [`DEFAULT_WAL_CHECKPOINT_BYTES`].
    pub wal_checkpoint_bytes: u64,
}

/// 16 MiB: a ~250 B por operación son ~64k operaciones entre checkpoints,
/// un replay de un segundo largo si el proceso muere justo antes del
/// siguiente. El checkpoint cuesta un fsync del motor (milisegundos), así
/// que bajar el umbral es barato; subirlo solo alarga el replay al abrir.
pub const DEFAULT_WAL_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            // `Auto`: redb (`kv::resolve_engine`); una base sled de 0.5.x da
            // la receta de migración.
            engine: StorageEngine::Auto,
            profile: StorageProfile::Default,
            direct_write_durability: DirectWriteDurability::default(),
            wal_checkpoint_bytes: DEFAULT_WAL_CHECKPOINT_BYTES,
        }
    }
}

/// Storage tuning knobs.
#[derive(Debug, Clone, Copy)]
pub struct StorageTuning {
    pub cache_capacity_bytes: Option<u64>,
    pub flush_every_ms: Option<u64>,
    pub use_compression: bool,
}

impl StorageProfile {
    pub fn tuning(self) -> StorageTuning {
        match self {
            StorageProfile::Default => StorageTuning {
                // Explicit on purpose (it was sled 0.34's implicit default,
                // which the profile used to inherit via `None`): an
                // engine-agnostic profile must not depend on whatever a
                // particular engine defaults to.
                cache_capacity_bytes: Some(1024 * 1024 * 1024),
                flush_every_ms: Some(1000),
                use_compression: false,
            },
            StorageProfile::Mobile => StorageTuning {
                // Keep memory footprint conservative on constrained devices.
                cache_capacity_bytes: Some(16 * 1024 * 1024),
                flush_every_ms: Some(3000),
                use_compression: false,
            },
            StorageProfile::Server => StorageTuning {
                cache_capacity_bytes: Some(256 * 1024 * 1024),
                flush_every_ms: Some(500),
                // false desde 0.5.0 (con sled, pedir compresión hacía FALLAR
                // el open). La compresión vuelve como capacidad por-keyspace
                // de un motor que la dé sin conflicto (p. ej. fjall, LZ4).
                use_compression: false,
            },
        }
    }
}

// Aquí vivió `pub trait StorageBackend` (metadata: backend_name/profile/hooks
// de salud). Murió en 0.5.0 sin haber tenido jamás un caller: el contrato
// real de desacople es `storage::kv::KvEngine`/`KvKeyspace` (pub(crate)).
