// Backend redb del contrato KV.
//
// Única frontera del crate con el crate `redb`. Decisiones clave:
//
// - **Layout**: la base vive como UN archivo `nopal.redb` dentro del
//   directorio de la base (convive con nopal.wal, indexes/ y hnsw_*.meta;
//   los nombres de sled — conf, db, blobs/ — no colisionan).
// - **Sentinel estructural de engine**: abrir un directorio que contiene
//   una base sled falla con error explícito ANTES de crear un nopal.redb
//   vacío al lado (la doble apertura cruzada no la protege ningún lock del
//   OS: cada motor lockea archivos distintos).
// - **Durabilidad diferida, réplica del contrato sled**: cada escritura es
//   una write-txn con `Durability::None` (visible, sin fsync); un flusher
//   propio hace un commit vacío `Immediate` cada `flush_every_ms`, que
//   persiste todo lo anterior. El WAL propio de NopalDB sigue siendo la
//   garantía por commit; tras SIGKILL, redb retrocede al último commit
//   durable y el replay del WAL reconstruye — mismo modelo que sled.
// - **Iteradores por chunks**: `ReadOnlyTable` es owned pero sus ranges lo
//   borrowean (auto-referencia). En vez de pelear lifetimes, cada chunk
//   abre una read-txn corta y retoma desde la última clave — el mismo
//   patrón de paginación que `scan_nodes_batch` usa arriba, y ninguna
//   iteración nuestra exige snapshot puntual (sled tampoco lo daba).

use std::collections::VecDeque;
use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::{NopalError, Result, StorageError, StorageErrorKind};
use crate::storage::backend::StorageProfile;

use super::{KvEngine, KvIter, KvKeyspace, RmwFn, WriteBatch};

const DB_FILE: &str = "nopal.redb";
const CHUNK: usize = 512;

fn table_def(name: &str) -> ::redb::TableDefinition<'_, &'static [u8], &'static [u8]> {
    ::redb::TableDefinition::new(name)
}

fn internal(e: impl std::fmt::Display) -> NopalError {
    StorageError::new(StorageErrorKind::Internal, e.to_string()).into()
}

impl From<::redb::StorageError> for NopalError {
    fn from(e: ::redb::StorageError) -> Self {
        let kind = match &e {
            ::redb::StorageError::Io(_) => StorageErrorKind::Io,
            ::redb::StorageError::Corrupted(_) => StorageErrorKind::Corruption,
            _ => StorageErrorKind::Internal,
        };
        StorageError::new(kind, e.to_string()).into()
    }
}

// ─── Engine ─────────────────────────────────────────────────────────────────

pub(crate) struct RedbEngine {
    db: Arc<::redb::Database>,
    stop: Arc<AtomicBool>,
    flusher: Option<std::thread::JoinHandle<()>>,
}

impl RedbEngine {
    pub(crate) fn open(dir: &Path, profile: StorageProfile) -> Result<Self> {
        // Sentinel estructural: una base sled se identifica por sus archivos.
        if dir.join("conf").exists() && dir.join("db").exists() && !dir.join(DB_FILE).exists() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidData,
                format!(
                    "el directorio {} contiene una base sled; ábrela con engine=sled o migra los datos",
                    dir.display()
                ),
            )
            .into());
        }
        std::fs::create_dir_all(dir)?;
        let db = Self::builder_for(profile)
            .create(dir.join(DB_FILE))
            .map_err(internal)?;
        Ok(Self::with_flusher(db, profile))
    }

    pub(crate) fn open_temporary(profile: StorageProfile) -> Result<Self> {
        let db = Self::builder_for(profile)
            .create_with_backend(::redb::backends::InMemoryBackend::new())
            .map_err(internal)?;
        Ok(Self::with_flusher(db, profile))
    }

    fn builder_for(profile: StorageProfile) -> ::redb::Builder {
        let tuning = profile.tuning();
        let mut builder = ::redb::Builder::new();
        if let Some(bytes) = tuning.cache_capacity_bytes {
            builder.set_cache_size(bytes as usize);
        }
        // use_compression: redb no comprime; el knob se ignora sin warning
        // (a diferencia de sled, aquí nunca fue una promesa del perfil).
        builder
    }

    fn with_flusher(db: ::redb::Database, profile: StorageProfile) -> Self {
        let db = Arc::new(db);
        let stop = Arc::new(AtomicBool::new(false));
        let flusher = profile.tuning().flush_every_ms.map(|ms| {
            let db = Arc::clone(&db);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(std::time::Duration::from_millis(ms));
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // Commit vacío durable: persiste todos los commits None previos.
                    let _ = durable_checkpoint(&db);
                }
            })
        });
        Self { db, stop, flusher }
    }
}

fn durable_checkpoint(db: &::redb::Database) -> Result<()> {
    let mut txn = db.begin_write().map_err(internal)?;
    txn.set_durability(::redb::Durability::Immediate)
        .map_err(internal)?;
    txn.commit().map_err(internal)?;
    Ok(())
}

impl Drop for RedbEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.flusher.take() {
            h.thread().unpark();
            let _ = h.join();
        }
        // Cierre limpio: lo escrito queda durable aunque el timer no alcanzara.
        let _ = durable_checkpoint(&self.db);
    }
}

impl KvEngine for RedbEngine {
    fn engine_name(&self) -> &'static str {
        "redb"
    }

    fn keyspace(&self, name: &str) -> Result<Arc<dyn KvKeyspace>> {
        // Crea la tabla si no existe: los reads posteriores no lidian con
        // TableDoesNotExist y el handle queda usable de inmediato.
        let mut txn = self.db.begin_write().map_err(internal)?;
        txn.set_durability(::redb::Durability::None).map_err(internal)?;
        txn.open_table(table_def(name)).map_err(internal)?;
        txn.commit().map_err(internal)?;
        Ok(Arc::new(RedbKeyspace {
            db: Arc::clone(&self.db),
            name: name.to_string(),
        }))
    }

    fn flush(&self) -> Result<()> {
        durable_checkpoint(&self.db)
    }
}

// ─── Keyspace ───────────────────────────────────────────────────────────────

pub(crate) struct RedbKeyspace {
    db: Arc<::redb::Database>,
    name: String,
}

impl RedbKeyspace {
    fn write<T>(
        &self,
        f: impl FnOnce(&mut ::redb::Table<'_, &'static [u8], &'static [u8]>) -> Result<T>,
    ) -> Result<T> {
        let mut txn = self.db.begin_write().map_err(internal)?;
        txn.set_durability(::redb::Durability::None).map_err(internal)?;
        let out = {
            let mut table = txn.open_table(table_def(&self.name)).map_err(internal)?;
            f(&mut table)?
        };
        txn.commit().map_err(internal)?;
        Ok(out)
    }

    fn read_table(&self) -> Result<::redb::ReadOnlyTable<&'static [u8], &'static [u8]>> {
        use ::redb::ReadableDatabase;
        let txn = self.db.begin_read().map_err(internal)?;
        txn.open_table(table_def(&self.name)).map_err(internal)
    }
}

/// Iterador por chunks: cada relleno abre una read-txn corta y retoma
/// ESTRICTAMENTE después de la última clave entregada.
struct ChunkedIter {
    ks: RedbKeyspace,
    prefix: Option<Vec<u8>>,
    next_from: Bound<Vec<u8>>,
    buf: VecDeque<super::KvPair>,
    done: bool,
    error: Option<NopalError>,
}

impl Iterator for ChunkedIter {
    type Item = Result<super::KvPair>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(e) = self.error.take() {
            self.done = true;
            return Some(Err(e));
        }
        if self.buf.is_empty()
            && !self.done
            && let Err(e) = self.fill()
        {
            self.done = true;
            return Some(Err(e));
        }
        self.buf.pop_front().map(Ok)
    }
}

impl ChunkedIter {
    fn new(ks: RedbKeyspace, start: Vec<u8>, prefix: Option<Vec<u8>>) -> Self {
        Self {
            ks,
            prefix,
            next_from: Bound::Included(start),
            buf: VecDeque::new(),
            done: false,
            error: None,
        }
    }

    fn fill(&mut self) -> Result<()> {
        let table = self.ks.read_table()?;
        let from = match &self.next_from {
            Bound::Included(k) => Bound::Included(k.as_slice()),
            Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let range = table
            .range::<&[u8]>((from, Bound::Unbounded))
            .map_err(internal)?;

        let mut last: Option<Vec<u8>> = None;
        for item in range.take(CHUNK) {
            let (k, v) = item.map_err(NopalError::from)?;
            let key = k.value().to_vec();
            if let Some(p) = &self.prefix
                && !key.starts_with(p)
            {
                self.done = true;
                return Ok(());
            }
            last = Some(key.clone());
            self.buf.push_back((key, v.value().to_vec()));
        }
        match last {
            Some(k) => self.next_from = Bound::Excluded(k),
            None => self.done = true,
        }
        Ok(())
    }
}

impl KvKeyspace for RedbKeyspace {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let table = self.read_table()?;
        Ok(table
            .get(key)
            .map_err(NopalError::from)?
            .map(|v| v.value().to_vec()))
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.write(|t| {
            t.insert(key, value).map_err(NopalError::from)?;
            Ok(())
        })
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        self.write(|t| {
            t.remove(key).map_err(NopalError::from)?;
            Ok(())
        })
    }

    fn contains_key(&self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }

    fn iter(&self) -> KvIter<'_> {
        Box::new(ChunkedIter::new(self.clone_handle(), Vec::new(), None))
    }

    fn scan_prefix(&self, prefix: &[u8]) -> KvIter<'_> {
        Box::new(ChunkedIter::new(
            self.clone_handle(),
            prefix.to_vec(),
            Some(prefix.to_vec()),
        ))
    }

    fn range_from(&self, start: &[u8]) -> KvIter<'_> {
        Box::new(ChunkedIter::new(self.clone_handle(), start.to_vec(), None))
    }

    fn apply_batch(&self, batch: WriteBatch) -> Result<()> {
        // Una write-txn = atomicidad todo-o-nada del batch.
        self.write(|t| {
            for (key, value) in batch.ops() {
                match value {
                    Some(v) => {
                        t.insert(key.as_slice(), v.as_slice())
                            .map_err(NopalError::from)?;
                    }
                    None => {
                        t.remove(key.as_slice()).map_err(NopalError::from)?;
                    }
                }
            }
            Ok(())
        })
    }

    fn rmw(&self, key: &[u8], f: &mut RmwFn<'_>) -> Result<()> {
        // El single-writer de redb serializa las write-txn: leer-modificar-
        // escribir dentro de una sola txn es atómico sin CAS-loop.
        self.write(|t| {
            use ::redb::ReadableTable;
            let old = t
                .get(key)
                .map_err(NopalError::from)?
                .map(|v| v.value().to_vec());
            match f(old.as_deref()) {
                Some(new) => {
                    t.insert(key, new.as_slice()).map_err(NopalError::from)?;
                }
                None => {
                    t.remove(key).map_err(NopalError::from)?;
                }
            }
            Ok(())
        })
    }

    fn clear(&self) -> Result<()> {
        let mut txn = self.db.begin_write().map_err(internal)?;
        txn.set_durability(::redb::Durability::None).map_err(internal)?;
        txn.delete_table(table_def(&self.name)).map_err(internal)?;
        // Recrear vacía: el handle sigue siendo usable tras clear().
        txn.open_table(table_def(&self.name)).map_err(internal)?;
        txn.commit().map_err(internal)?;
        Ok(())
    }
}

impl RedbKeyspace {
    fn clone_handle(&self) -> RedbKeyspace {
        RedbKeyspace {
            db: Arc::clone(&self.db),
            name: self.name.clone(),
        }
    }
}

/// ¿El directorio contiene una base redb? (para el sentinel inverso de sled)
pub(crate) fn sled_dir_has_redb(dir: &Path) -> bool {
    dir.join(DB_FILE).exists()
}
