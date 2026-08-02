// Backend sled del contrato KV, más la conversión de errores.
//
// Es la ÚNICA frontera del crate con el crate `sled`: errores, tipos y
// tuning viven aquí. Rutas `::sled` absolutas a propósito.

use std::sync::Arc;

use crate::error::{NopalError, Result, StorageError, StorageErrorKind};
use crate::storage::backend::StorageProfile;

use super::{KvEngine, KvIter, KvKeyspace, RmwFn, WriteBatch, DEFAULT_KEYSPACE};

// ─── Errores ────────────────────────────────────────────────────────────────

impl From<::sled::Error> for StorageError {
    fn from(error: ::sled::Error) -> Self {
        let kind = match &error {
            ::sled::Error::Io(_) => StorageErrorKind::Io,
            ::sled::Error::Corruption { .. } => StorageErrorKind::Corruption,
            ::sled::Error::Unsupported(_) => StorageErrorKind::Unsupported,
            ::sled::Error::ReportableBug(_) => StorageErrorKind::Internal,
            ::sled::Error::CollectionNotFound(_) => StorageErrorKind::InvalidData,
        };

        StorageError::new(kind, error.to_string())
    }
}

impl From<::sled::Error> for NopalError {
    fn from(error: ::sled::Error) -> Self {
        StorageError::from(error).into()
    }
}

// ─── Engine ─────────────────────────────────────────────────────────────────

pub(crate) struct SledEngine {
    db: ::sled::Db,
}

impl SledEngine {
    fn config_for(profile: StorageProfile) -> ::sled::Config {
        let tuning = profile.tuning();
        let mut config = ::sled::Config::new();
        if let Some(cache_capacity_bytes) = tuning.cache_capacity_bytes {
            config = config.cache_capacity(cache_capacity_bytes);
        }
        // La compresión de sled es inactivable en este workspace (su zstd 0.9
        // colisiona por `links` con el de parquet) y con use_compression(true)
        // sled FALLA al abrir. Se ignora el knob con aviso, nunca se brickea
        // el open — el perfil Server estuvo roto exactamente por esto.
        if tuning.use_compression {
            log::warn!(
                "el backend sled no soporta compresión en este build; se abre sin comprimir"
            );
        }
        config.flush_every_ms(tuning.flush_every_ms)
    }

    pub(crate) fn open(path: &std::path::Path, profile: StorageProfile) -> Result<Self> {
        // Sentinel estructural inverso: si el directorio ya contiene una
        // base redb, abrir con sled crearía una base sled vacía al lado y
        // "desaparecerían" los datos. Ningún lock del OS protege el cruce
        // (cada motor lockea archivos distintos).
        #[cfg(feature = "storage-redb")]
        if super::redb::sled_dir_has_redb(path) && !path.join("conf").exists() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidData,
                format!(
                    "el directorio {} contiene una base redb; ábrela con engine=redb o migra los datos",
                    path.display()
                ),
            )
            .into());
        }
        let db = Self::config_for(profile)
            .path(path)
            .open()
            .map_err(NopalError::from)?;
        Ok(Self { db })
    }

    pub(crate) fn open_temporary(profile: StorageProfile) -> Result<Self> {
        let db = Self::config_for(profile)
            .temporary(true)
            .open()
            .map_err(NopalError::from)?;
        Ok(Self { db })
    }
}

impl KvEngine for SledEngine {
    fn engine_name(&self) -> &'static str {
        "sled"
    }

    fn keyspace(&self, name: &str) -> Result<Arc<dyn KvKeyspace>> {
        // El keyspace "default" ES el tree default de sled (Db: Deref<Tree>),
        // donde viven los namespaces node:/idx:/ts:/meta: — mantenerlo así
        // preserva el formato en disco byte a byte.
        let tree: ::sled::Tree = if name == DEFAULT_KEYSPACE {
            (*self.db).clone()
        } else {
            self.db.open_tree(name)?
        };
        Ok(Arc::new(SledKeyspace { tree }))
    }

    fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

// ─── Keyspace ───────────────────────────────────────────────────────────────

pub(crate) struct SledKeyspace {
    tree: ::sled::Tree,
}

fn own_pairs<'a>(
    iter: impl Iterator<Item = std::result::Result<(::sled::IVec, ::sled::IVec), ::sled::Error>>
        + Send
        + 'a,
) -> KvIter<'a> {
    Box::new(iter.map(|item| {
        item.map(|(k, v)| (k.to_vec(), v.to_vec()))
            .map_err(NopalError::from)
    }))
}

impl KvKeyspace for SledKeyspace {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.tree.get(key)?.map(|v| v.to_vec()))
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.tree.insert(key, value)?;
        Ok(())
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        self.tree.remove(key)?;
        Ok(())
    }

    fn contains_key(&self, key: &[u8]) -> Result<bool> {
        Ok(self.tree.contains_key(key)?)
    }

    fn iter(&self) -> KvIter<'_> {
        own_pairs(self.tree.iter())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> KvIter<'_> {
        own_pairs(self.tree.scan_prefix(prefix))
    }

    fn range_from(&self, start: &[u8]) -> KvIter<'_> {
        own_pairs(self.tree.range(start.to_vec()..))
    }

    fn apply_batch(&self, batch: WriteBatch) -> Result<()> {
        let mut b = ::sled::Batch::default();
        for (key, value) in batch.ops() {
            match value {
                Some(v) => b.insert(key.as_slice(), v.as_slice()),
                None => b.remove(key.as_slice()),
            }
        }
        self.tree.apply_batch(b)?;
        Ok(())
    }

    fn rmw(&self, key: &[u8], f: &mut RmwFn<'_>) -> Result<()> {
        self.tree
            .fetch_and_update(key, |old: Option<&[u8]>| f(old))?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        self.tree.clear()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sled_error_is_converted_without_leaking_backend_type() {
        let io_error = std::io::Error::new(std::io::ErrorKind::Other, "test storage failure");

        let sled_error = ::sled::Error::Io(io_error);
        let error: NopalError = sled_error.into();

        match error {
            NopalError::StorageError(storage_error) => {
                assert_eq!(storage_error.kind(), StorageErrorKind::Io);
                assert!(storage_error.message().contains("test storage failure"));
            }
            other => panic!("expected StorageError, got {other:?}"),
        }
    }
}
