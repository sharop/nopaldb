use crate::error::Result;

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
