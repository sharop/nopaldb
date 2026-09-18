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
/// Desde 0.6.0 el motor por defecto es redb y `Auto` es lo que usa
/// `StorageOptions::default()`. `Auto` existe para que una base creada con
/// sled en 0.5.x se abra con el binario nuevo **sin tocar código**: mira el
/// directorio y elige el motor que ya está ahí (redb deja `nopal.redb`; sled
/// deja `conf` y `db`); si el directorio es nuevo, usa el motor por defecto
/// del build. Un motor explícito sigue siendo explícito: pedir `Redb` sobre
/// una base sled es un error que dice cómo migrar, no una sorpresa
/// silenciosa. Se descartó "redb salvo que…" sin variante propia porque
/// entonces un valor explícito y el default serían indistinguibles al abrir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StorageEngine {
    /// El motor del directorio si ya contiene una base; si no, el motor por
    /// defecto del build (redb si está compilado, si no sled).
    #[default]
    Auto,
    /// Motor de 0.5.x. Opcional desde 0.6.0 (feature `storage-sled`), para
    /// leer y migrar bases existentes; disponible al menos hasta 0.7.
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
}

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            // `Auto`: el motor del directorio si ya hay base; si no, el del
            // build (`kv::default_engine`: redb si está compilado, si no
            // sled). Así la suite entera corre contra el único motor
            // compilado sin tocar un test, y una base de 0.5.x abre igual.
            engine: StorageEngine::Auto,
            profile: StorageProfile::Default,
            direct_write_durability: DirectWriteDurability::default(),
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
                // Explicit on purpose: this equals sled 0.34's implicit
                // default, which the profile used to inherit silently via
                // `None`. Right-sizing it is a separate, deliberate decision;
                // an engine-agnostic profile must not depend on whatever a
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
                // false desde 0.5.0: pedirle compresión a sled hacía FALLAR
                // el open del perfil completo ("the 'compression' feature
                // must be enabled") — y esa feature es inactivable aquí: su
                // zstd 0.9 colisiona por `links` con el zstd de parquet.
                // La compresión vuelve como capacidad por-keyspace de los
                // motores que la den sin conflicto (p. ej. fjall, LZ4).
                use_compression: false,
            },
        }
    }
}

// Aquí vivió `pub trait StorageBackend` (metadata: backend_name/profile/hooks
// de salud). Murió en 0.5.0 sin haber tenido jamás un caller: el contrato
// real de desacople es `storage::kv::KvEngine`/`KvKeyspace` (pub(crate)).
