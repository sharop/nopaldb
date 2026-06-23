use crate::error::Result;

/// Runtime profile for storage tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageProfile {
    Default,
    Mobile,
    Server,
}

/// Logical storage engine selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEngine {
    Sled,
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
            engine: StorageEngine::Sled,
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
                cache_capacity_bytes: None,
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
                use_compression: true,
            },
        }
    }
}

/// Minimal contract to decouple query/graph layers from a concrete KV engine.
///
/// NOTE: We start with a narrow trait and grow it with stable semantics.
pub trait StorageBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn profile(&self) -> StorageProfile;
    fn tuning(&self) -> StorageTuning {
        self.profile().tuning()
    }

    /// Health check hook for backend-specific validation.
    fn verify_health(&self) -> Result<()> {
        Ok(())
    }

    /// Basic scan capability contract used by streaming executor.
    fn supports_node_batch_scan(&self) -> bool {
        true
    }

    /// Optional hint for backend implementations that can expose metrics.
    fn estimated_cache_capacity_bytes(&self) -> Option<u64> {
        self.tuning().cache_capacity_bytes
    }

    /// Backend-agnostic integrity hook. Concrete backend may no-op.
    fn repair_metadata_if_needed(&self) -> Result<()> {
        Ok(())
    }
}
