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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageEngine {
    Sled,
    /// Experimental (0.5.x): requiere la feature `storage-redb`.
    Redb,
}

/// Storage creation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageOptions {
    pub engine: StorageEngine,
    pub profile: StorageProfile,
}

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            // Precedencia sled: nada cambia para nadie mientras la feature
            // default esté activa. Con SOLO storage-redb compilado, el
            // default cae a Redb — así la suite completa corre contra redb
            // sin tocar un solo test (`--no-default-features --features
            // storage-redb`).
            #[cfg(feature = "storage-sled")]
            engine: StorageEngine::Sled,
            #[cfg(all(not(feature = "storage-sled"), feature = "storage-redb"))]
            engine: StorageEngine::Redb,
            profile: StorageProfile::Default,
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
