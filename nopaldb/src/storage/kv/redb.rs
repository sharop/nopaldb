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
use std::sync::{Arc, Mutex, RwLock, Weak};

use crate::error::{NopalError, Result, StorageError, StorageErrorKind};
use crate::storage::backend::StorageProfile;

use super::{KvEngine, KvIter, KvKeyspace, RmwFn, WriteBatch};

const DB_FILE: &str = "nopal.redb";
/// Ruta de staging de la primera creación: `nopal.redb` solo aparece cuando
/// la base ya es válida (ver `open`). Un temporal huérfano indica un arranque
/// en frío interrumpido y se descarta.
const TMP_DB_FILE: &str = "nopal.redb.creating";
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

// ─── Caché de lectura ───────────────────────────────────────────────────────

type ReadOnlyTable = ::redb::ReadOnlyTable<&'static [u8], &'static [u8]>;

/// La tabla de solo-lectura que un keyspace reutiliza entre `get`s hasta el
/// siguiente commit del proceso. `None` = hay que abrir una nueva.
type ReadSlot = RwLock<Option<ReadOnlyTable>>;

/// Estado compartido entre el engine y sus keyspaces: los slots de lectura
/// de todos los keyspaces vivos (`Weak`: un keyspace que ya nadie usa no
/// retiene su snapshot) y si hubo commits de datos desde el último
/// checkpoint durable.
#[derive(Default)]
struct Shared {
    slots: Mutex<Vec<Weak<ReadSlot>>>,
    /// `true` desde el primer commit de datos tras un checkpoint durable.
    /// Un checkpoint sobre una base sin cambios es un fsync gratis (~7 ms
    /// en un Mac por `F_FULLFSYNC`): el flusher periódico y el cierre lo
    /// omiten cuando no hay nada que persistir.
    dirty: AtomicBool,
}

type ReadSlots = Arc<Shared>;

/// Se llama DESPUÉS de cada `commit()` de datos: vacía los slots de lectura
/// (cualquier snapshot anterior es viejo, y soltarlo deja que redb recicle
/// las páginas que el commit liberó) y marca la base como pendiente de
/// checkpoint. Un checkpoint durable (commit vacío) no cambia datos y no
/// pasa por aquí.
fn invalidate_reads(shared: &ReadSlots) {
    shared.dirty.store(true, Ordering::Release);
    let mut slots = shared.slots.lock().unwrap_or_else(|e| e.into_inner());
    slots.retain(|w| match w.upgrade() {
        Some(slot) => {
            *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
            true
        }
        None => false,
    });
}

// ─── Engine ─────────────────────────────────────────────────────────────────

pub(crate) struct RedbEngine {
    db: Arc<::redb::Database>,
    stop: Arc<AtomicBool>,
    flusher: Option<std::thread::JoinHandle<()>>,
    /// Reserva de la ruta en el registro del proceso; se suelta con el engine.
    _lease: Option<super::PathLease>,
    /// Ver [`ReadSlots`].
    read_slots: ReadSlots,
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
        let path = dir.join(DB_FILE);

        // La PRIMERA creación se hace aparte y se publica con un rename.
        //
        // Crear directamente sobre la ruta final abre una ventana en la que
        // el archivo ya existe pero todavía no es una base redb válida: si el
        // proceso muere ahí (OOM, kill del contenedor, corte), el directorio
        // queda envenenado para siempre — toda apertura posterior falla con
        // "invalid data" y la app no vuelve a arrancar. Medido: matando el
        // proceso dentro de esa ventana, 14 de 64 intentos dejaban la base
        // inabrible; sled, 0 de 64. Una base YA establecida no corre ese
        // riesgo (0 de 44), así que el arreglo solo necesita cubrir el
        // arranque en frío.
        //
        // El rename es atómico, así que el invariante pasa a ser: si
        // `nopal.redb` existe, es una base completa y válida. Morir antes del
        // rename solo deja el temporal, que la siguiente apertura descarta.
        //
        // Se cierra el handle ANTES de renombrar a propósito: renombrar un
        // archivo abierto funciona en Unix pero falla en Windows, y esta
        // librería publica wheels para Windows. El costo es una apertura
        // extra, y solo la primera vez.
        if !path.exists() {
            let tmp = dir.join(TMP_DB_FILE);
            // Restos de un intento anterior que murió antes del rename.
            if tmp.exists() {
                std::fs::remove_file(&tmp)?;
            }
            {
                let db = Self::builder_for(profile).create(&tmp).map_err(internal)?;
                drop(db);
            }
            std::fs::rename(&tmp, &path)?;
        }

        let db = Self::open_existing(&path, profile)?;
        Ok(Self::with_flusher(db, profile))
    }

    /// Abre una base que ya existe, distinguiendo las dos formas de fallar.
    ///
    /// Importa mantenerlas separadas: el remedio de una es borrar el
    /// directorio y el de la otra es cerrar el otro proceso. Confundirlas
    /// manda a alguien a destruir una base sana.
    fn open_existing(path: &Path, profile: StorageProfile) -> Result<::redb::Database> {
        // El lock de redb es `try_lock` — no bloqueante y sin reintento. Tras
        // un SIGKILL el kernel libera el flock del proceso muerto, pero no
        // necesariamente antes de que el siguiente intento lo pida: quien
        // reabre inmediatamente después de matar (harnesses de crash,
        // supervisores que reinician al vuelo) se topa con un "already open"
        // que se resuelve solo en milisegundos. Se reintenta con una espera
        // acotada; si de verdad hay otro proceso, el error llega igual, solo
        // que un instante después y diciendo lo que pasa.
        const ESPERA_LOCK: std::time::Duration = std::time::Duration::from_millis(1500);
        let inicio = std::time::Instant::now();
        loop {
            match Self::builder_for(profile).create(path) {
                Ok(db) => return Ok(db),
                Err(::redb::DatabaseError::DatabaseAlreadyOpen) => {
                    if inicio.elapsed() >= ESPERA_LOCK {
                        return Err(StorageError::new(
                            StorageErrorKind::Unsupported,
                            // `DatabaseAlreadyOpen` es el registro interno de redb:
                            // la tiene abierta ESTE proceso (otro handle vivo, o un
                            // `Graph` cerrado pero no soltado). Un lock de otro
                            // proceso llega como error de I/O, no como este.
                            format!(
                                "la base redb en {} ya está abierta en este proceso: el lock se \
                                 libera al soltar el `Graph` (drop), no en `close()`. Descarta el \
                                 valor anterior antes de reabrir, o abre una copia.",
                                path.display()
                            ),
                        )
                        .into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => {
                    // El archivo existe pero no se puede leer. El caso que el
                    // motor sí puede nombrar es una base creada a medias por
                    // una versión anterior al rename atómico.
                    return Err(StorageError::new(
                        StorageErrorKind::InvalidData,
                        format!(
                            "no se pudo abrir la base redb en {}: {e}. Si el proceso \
                             murió durante el PRIMER arranque de esta base (versiones \
                             ≤0.5.6), el archivo quedó a medio crear y no contiene \
                             datos: borrar el directorio y reabrir lo resuelve. \
                             Verificar antes que no haya datos que preservar.",
                            path.display()
                        ),
                    )
                    .into());
                }
            }
        }
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
        let shared: ReadSlots = Arc::new(Shared::default());
        let flusher = profile.tuning().flush_every_ms.map(|ms| {
            let db = Arc::clone(&db);
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(std::time::Duration::from_millis(ms));
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // Commit vacío durable: persiste todos los commits None previos.
                    let _ = checkpoint_if_dirty(&db, &shared);
                }
            })
        });
        Self { db, stop, flusher, _lease: None, read_slots: shared }
    }

    /// Adjunta la reserva de ruta del proceso (ver `kv::open_engine`).
    pub(crate) fn with_lease(mut self, lease: super::PathLease) -> Self {
        self._lease = Some(lease);
        self
    }
}

fn durable_checkpoint(db: &::redb::Database) -> Result<()> {
    let mut txn = db.begin_write().map_err(internal)?;
    txn.set_durability(::redb::Durability::Immediate)
        .map_err(internal)?;
    txn.commit().map_err(internal)?;
    Ok(())
}

/// Checkpoint durable solo si hubo commits de datos desde el anterior. Si
/// el fsync falla, la base vuelve a quedar marcada para reintentarlo.
fn checkpoint_if_dirty(db: &::redb::Database, shared: &Shared) -> Result<()> {
    if !shared.dirty.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    durable_checkpoint(db).inspect_err(|_| shared.dirty.store(true, Ordering::Release))
}

impl Drop for RedbEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.flusher.take() {
            h.thread().unpark();
            let _ = h.join();
        }
        // Cierre limpio: lo escrito queda durable aunque el timer no alcanzara.
        let _ = checkpoint_if_dirty(&self.db, &self.read_slots);
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
        invalidate_reads(&self.read_slots);
        let slot: Arc<ReadSlot> = Arc::new(RwLock::new(None));
        self.read_slots
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::downgrade(&slot));
        Ok(Arc::new(RedbKeyspace {
            db: Arc::clone(&self.db),
            name: name.to_string(),
            slot,
            read_slots: Arc::clone(&self.read_slots),
        }))
    }

    fn apply_multi(&self, batches: Vec<(String, WriteBatch)>) -> Result<()> {
        // UNA write-txn cubre todos los keyspaces: el commit único es la
        // atomicidad cross-keyspace (Durability::None, mismo contrato de
        // visibilidad-sin-fsync que el resto de escrituras). Cada tabla se
        // abre y se cierra por batch — reabrir la misma tabla más adelante
        // en la txn es válido porque el handle anterior ya se soltó, y así
        // los keyspaces repetidos se aplican en orden.
        let mut txn = self.db.begin_write().map_err(internal)?;
        txn.set_durability(::redb::Durability::None).map_err(internal)?;
        for (name, batch) in &batches {
            let mut table = txn.open_table(table_def(name)).map_err(internal)?;
            for (key, value) in batch.ops() {
                match value {
                    Some(v) => {
                        table
                            .insert(key.as_slice(), v.as_slice())
                            .map_err(NopalError::from)?;
                    }
                    None => {
                        table.remove(key.as_slice()).map_err(NopalError::from)?;
                    }
                }
            }
        }
        txn.commit().map_err(internal)?;
        invalidate_reads(&self.read_slots);
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        checkpoint_if_dirty(&self.db, &self.read_slots)
    }
}

// ─── Keyspace ───────────────────────────────────────────────────────────────

/// Un keyspace = una tabla de redb.
///
/// # Lecturas: una tabla de solo-lectura reutilizada hasta el próximo commit
///
/// `begin_read` + `open_table` por cada `get` costaba 0.30 µs/get sin
/// escritor y 0.70 µs/get con uno (medido aparte sobre redb 4.1: 0.42 vs
/// 0.12 µs/get, y 0.93 vs 0.22), y era todo el exceso de redb frente a sled
/// en lecturas puntuales. Aquí la tabla abierta se guarda en `slot` y se
/// reutiliza; **todo** commit de datos del proceso (`write`, `apply_multi`,
/// `clear`, la creación de tablas) vacía los slots de todos los keyspaces
/// ([`invalidate_reads`]), así que:
///
/// - una lectura nunca ve un snapshot anterior al último commit (redb es
///   single-writer y ese escritor somos nosotros: no hay commits ajenos);
/// - el snapshot solo vive durante fases sin escritura, donde no retiene
///   nada que redb quisiera liberar; en cuanto hay un commit se suelta.
///
/// Descartado: un snapshot por operación de grafo (obliga a cambiar el
/// contrato `KvKeyspace`/`Storage` y no ayuda a `get_node`, que es un solo
/// `get`); un contador de generación sin vaciado por el escritor (retendría
/// el snapshot viejo durante una fase de solo escritura, p. ej. un bulk, y
/// el archivo crecería mientras tanto).
pub(crate) struct RedbKeyspace {
    db: Arc<::redb::Database>,
    name: String,
    slot: Arc<ReadSlot>,
    read_slots: ReadSlots,
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
        invalidate_reads(&self.read_slots);
        Ok(out)
    }

    fn open_read_table(&self) -> Result<ReadOnlyTable> {
        use ::redb::ReadableDatabase;
        let txn = self.db.begin_read().map_err(internal)?;
        txn.open_table(table_def(&self.name)).map_err(internal)
    }

    /// Ejecuta `f` sobre la tabla de lectura vigente: la cacheada si ningún
    /// commit la invalidó, o una recién abierta que queda cacheada. El
    /// read-lock del slot se sostiene durante `f`, así que un commit
    /// concurrente vacía el slot justo después, nunca a medias.
    fn with_read_table<R>(&self, f: impl FnOnce(&ReadOnlyTable) -> Result<R>) -> Result<R> {
        {
            let guard = self.slot.read().unwrap_or_else(|e| e.into_inner());
            if let Some(table) = guard.as_ref() {
                return f(table);
            }
        }
        let table = self.open_read_table()?;
        let mut guard = self.slot.write().unwrap_or_else(|e| e.into_inner());
        let out = f(&table);
        // Solo se cachea si nadie la invalidó mientras se abría: si un commit
        // entró en medio, esta tabla ya es vieja y el slot se queda vacío.
        if guard.is_none() {
            *guard = Some(table);
        }
        out
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
        let from = match &self.next_from {
            Bound::Included(k) => Bound::Included(k.as_slice()),
            Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let prefix = self.prefix.as_deref();
        // (pares del chunk, se cortó por prefijo)
        let (chunk, stopped_by_prefix) = self.ks.with_read_table(|table| {
            let range = table
                .range::<&[u8]>((from, Bound::Unbounded))
                .map_err(internal)?;
            let mut chunk: Vec<super::KvPair> = Vec::with_capacity(CHUNK);
            for item in range.take(CHUNK) {
                let (k, v) = item.map_err(NopalError::from)?;
                let key = k.value().to_vec();
                if let Some(p) = prefix
                    && !key.starts_with(p)
                {
                    return Ok((chunk, true));
                }
                chunk.push((key, v.value().to_vec()));
            }
            Ok((chunk, false))
        })?;

        let last = chunk.last().map(|(k, _)| k.clone());
        self.buf.extend(chunk);
        if stopped_by_prefix {
            self.done = true;
            return Ok(());
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
        self.with_read_table(|table| {
            Ok(table
                .get(key)
                .map_err(NopalError::from)?
                .map(|v| v.value().to_vec()))
        })
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
        invalidate_reads(&self.read_slots);
        Ok(())
    }
}

impl RedbKeyspace {
    fn clone_handle(&self) -> RedbKeyspace {
        RedbKeyspace {
            db: Arc::clone(&self.db),
            name: self.name.clone(),
            slot: Arc::clone(&self.slot),
            read_slots: Arc::clone(&self.read_slots),
        }
    }
}

/// ¿El directorio contiene una base redb? (para el sentinel inverso de sled;
/// cfg: su único caller vive en kv/sled.rs — sin sled compilado es dead code
/// y el CI compila con -D warnings).
#[cfg(feature = "storage-sled")]
pub(crate) fn sled_dir_has_redb(dir: &Path) -> bool {
    dir.join(DB_FILE).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El error de doble apertura está TRADUCIDO, no es el crudo de redb.
    ///
    /// El de sled ya tenía test desde que su detección resultó estar rota;
    /// este faltaba. Aquí la detección es por TIPO
    /// (`DatabaseError::DatabaseAlreadyOpen`), no por texto, así que no
    /// puede morir por un cambio de wording — pero sí por un cambio de
    /// variante, y el retry podría dejar de engancharse igual.
    #[test]
    fn second_open_reports_an_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let Ok(_ocupante) = RedbEngine::open(dir.path(), StorageProfile::Default) else {
            panic!("la primera apertura debe tomar el lock");
        };

        let inicio = std::time::Instant::now();
        let Err(err) = RedbEngine::open(dir.path(), StorageProfile::Default) else {
            panic!("la segunda apertura no puede tomar el lock");
        };
        let esperado = inicio.elapsed();

        let msg = format!("{err}");
        assert!(
            msg.contains("ya está abierta en este proceso"),
            "el error debe estar traducido, no ser el crudo de redb: {msg}"
        );
        assert!(
            msg.contains(&dir.path().display().to_string()),
            "debe nombrar el directorio: {msg}"
        );
        assert!(
            esperado >= std::time::Duration::from_millis(500),
            "debe haber reintentado antes de rendirse; se rindió en {esperado:?}"
        );
    }

    fn slot_is_cached(ks: &Arc<dyn KvKeyspace>) -> bool {
        // Solo el test conoce el tipo concreto; el contrato no expone el slot.
        let raw = Arc::as_ptr(ks) as *const RedbKeyspace;
        // SAFETY: todos los keyspaces de RedbEngine son RedbKeyspace; el
        // puntero viene de un Arc vivo y solo se lee.
        let ks = unsafe { &*raw };
        ks.slot.read().unwrap().is_some()
    }

    /// El mecanismo de la caché de lectura: un `get` deja la tabla cacheada,
    /// y CUALQUIER commit de datos (propio, de otro keyspace vía
    /// `apply_multi`, `clear`) la vacía, de modo que la siguiente lectura ve
    /// lo escrito. Fija el comportamiento por el que redb dejó de pagar
    /// `begin_read` + `open_table` en cada lectura.
    #[test]
    fn read_table_is_reused_until_the_next_commit_and_never_stale() {
        let engine = RedbEngine::open_temporary(StorageProfile::Default).unwrap();
        let a = engine.keyspace("a").unwrap();
        let b = engine.keyspace("b").unwrap();

        assert!(!slot_is_cached(&a), "nada cacheado antes de leer");
        assert_eq!(a.get(b"k").unwrap(), None);
        assert!(slot_is_cached(&a), "la primera lectura deja la tabla cacheada");
        assert_eq!(a.get(b"k").unwrap(), None);
        assert!(slot_is_cached(&a));

        // Escritura propia: invalida y la lectura siguiente ve el valor.
        a.insert(b"k", b"1").unwrap();
        assert!(!slot_is_cached(&a), "un commit vacía el slot");
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"1"[..]));
        assert!(slot_is_cached(&a));

        // Escritura en OTRO keyspace vía apply_multi: también invalida a `a`
        // (el snapshot es de toda la base, no de una tabla).
        let mut batch = WriteBatch::default();
        batch.insert(b"x".to_vec(), b"y".to_vec());
        engine.apply_multi(vec![("b".to_string(), batch)]).unwrap();
        assert!(!slot_is_cached(&a));
        assert_eq!(b.get(b"x").unwrap().as_deref(), Some(&b"y"[..]));

        // rmw y clear pasan por commits: lo escrito se ve de inmediato.
        a.rmw(b"k", &mut |old| old.map(|v| [v, b"2"].concat())).unwrap();
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"12"[..]));
        a.clear().unwrap();
        assert_eq!(a.get(b"k").unwrap(), None);

        // Un scan por chunks intercalado con un commit continúa sobre el
        // estado nuevo (la clave insertada por delante del cursor aparece).
        for i in 0..5u8 {
            a.insert(&[i], b"v").unwrap();
        }
        let mut it = a.iter();
        assert_eq!(it.next().unwrap().unwrap().0, vec![0]);
        a.insert(&[9], b"late").unwrap();
        let rest: Vec<Vec<u8>> = it.map(|r| r.unwrap().0).collect();
        assert_eq!(rest, vec![vec![1], vec![2], vec![3], vec![4], vec![9]]);
    }
}
