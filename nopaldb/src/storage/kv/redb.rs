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

use std::collections::{HashMap, VecDeque};
use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

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

// ─── Ventana de commit ──────────────────────────────────────────────────────

/// Edad máxima de una ventana de escritura antes de commitearla.
pub(crate) const WINDOW_MAX_AGE: Duration = Duration::from_millis(2);
/// Operaciones máximas por ventana antes de commitearla.
pub(crate) const WINDOW_MAX_OPS: usize = 256;

/// Overlay de la ventana: keyspace → clave → valor pendiente (`None` =
/// borrado pendiente). Anidado para que la lectura no asigne la clave.
type Overlay = HashMap<String, HashMap<Vec<u8>, Option<Vec<u8>>>>;

/// Una write-transaction de redb abierta que acumula las escrituras de
/// varias operaciones antes de UN commit.
///
/// # Por qué existe
///
/// redb es un B+tree copy-on-write con caché write-through: **cada commit
/// escribe al archivo (`pwrite`) toda página que tocó**, aunque sea
/// `Durability::None` (su propio código lo anota como TODO en
/// `cached_file.rs`). Un `add_edge` directo, que ya es un solo commit desde
/// 0.5.21, seguía costando ~50 µs de `pwrite`s por ~10 µs de trabajo real:
/// 0.22× sled en escritura directa. sled no paga eso porque solo toca su
/// pagecache y difiere el disco a su log.
///
/// Con la ventana, una operación inserta en la transacción abierta y
/// devuelve; el commit ocurre cuando la ventana cumple [`WINDOW_MAX_AGE`],
/// acumula [`WINDOW_MAX_OPS`], alguien necesita un scan, o llega un
/// `flush`/checkpoint. Un escritor secuencial mete cientos de operaciones
/// por commit: el coste de commit por operación cae de ~50 µs a menos de
/// 1 µs.
///
/// # Qué garantiza y qué no (durabilidad)
///
/// Exactamente lo mismo que antes. Los commits `None` **no eran durables**:
/// ante un crash redb vuelve al último checkpoint durable (el del flusher,
/// cada `flush_every_ms`, o el del cierre). Con la ventana la pérdida ante
/// crash sigue acotada por ese mismo checkpoint; solo cambia cuándo se
/// escriben las páginas. La recuperación de las escrituras directas la da
/// el WAL de NopalDB (desde 0.5.23 también las registra), no el motor.
///
/// # Visibilidad
///
/// El `overlay` guarda TODO lo escrito en la ventana y aún no commiteado
/// (`Some(v)` = valor pendiente, `None` = borrado pendiente). `get` lo
/// consulta antes que el árbol commiteado, así que una lectura nunca ve un
/// estado anterior a la última escritura de este proceso. Los scans no
/// fusionan overlay y árbol: **commitean la ventana primero** y luego
/// escanean el árbol. Fusionar rangos sería correcto pero es la complejidad
/// de un LSM para un caso sin beneficio: un scan solo paga ese commit si
/// llega dentro de los 2 ms siguientes a una escritura, y ese commit se iba
/// a hacer igual.
///
/// # Descartado
///
/// - Agrupar en el applier (drenar N ops y aplicar su write-set en un
///   `apply_multi`): no baja la latencia de un escritor secuencial, que es
///   una op por drenaje, y obliga a reescribir los cuerpos `apply_*`.
/// - Mantener abiertos los `Table` de la transacción entre operaciones: el
///   `Table<'txn>` toma prestada la transacción y el struct sería
///   autorreferencial; `open_table` sobre la write-txn cuesta ~0.3 µs, así
///   que se abre por operación.
/// - Ventana de segundos (memtable como la de sled): exige scans de fusión y
///   memoria gestionada por tamaño; no mejora la latencia por operación y en
///   NopalDB casi toda consulta escanea, así que se cerraría sola.
struct WriteWindow {
    txn: ::redb::WriteTransaction,
    overlay: Overlay,
    opened: Instant,
    ops: usize,
}

impl WriteWindow {
    fn is_stale(&self) -> bool {
        self.ops >= WINDOW_MAX_OPS || self.opened.elapsed() >= WINDOW_MAX_AGE
    }
}

/// Estado compartido entre el engine y sus keyspaces: los slots de lectura
/// de todos los keyspaces vivos (`Weak`: un keyspace que ya nadie usa no
/// retiene su snapshot), si hubo commits de datos desde el último
/// checkpoint durable, y la ventana de commit ([`WriteWindow`]).
struct Shared {
    /// La ventana abierta, si la hay. Declarada ANTES de `db` a propósito:
    /// al soltar el último `Shared` la transacción se commitea (ver `Drop`)
    /// antes de que caiga el último `Arc<Database>`, porque `Database::drop`
    /// abre una write-txn propia y se quedaría esperando para siempre a una
    /// ventana viva (medido: el proceso se colgaba al terminar un test cuyo
    /// keyspace sobrevivía a su engine).
    window: Mutex<Option<WriteWindow>>,
    /// Mantiene viva la base hasta que la ventana se haya cerrado (solo
    /// se sostiene, nunca se lee desde aquí).
    _db: Arc<::redb::Database>,
    slots: Mutex<Vec<Weak<ReadSlot>>>,
    /// `true` desde el primer commit de datos tras un checkpoint durable.
    /// Un checkpoint sobre una base sin cambios es un fsync gratis (~7 ms
    /// en un Mac por `F_FULLFSYNC`): el flusher periódico y el cierre lo
    /// omiten cuando no hay nada que persistir.
    dirty: AtomicBool,
    /// TODA write-transaction del motor pasa por el mutex de `window`
    /// (también las de crear tablas, `clear` y checkpoint): redb es
    /// single-writer y abrir una segunda con la ventana viva bloquearía
    /// hasta su commit.
    ///
    /// Espejo de `window.is_some()` para que las lecturas sin ventana no
    /// toquen el mutex.
    window_open: AtomicBool,
    /// `true` si el commit de una ventana falló: sus operaciones ya habían
    /// devuelto `Ok`, así que a partir de ahí toda escritura devuelve error
    /// en vez de seguir acumulando sobre un estado que no se pudo publicar.
    poisoned: AtomicBool,
}

impl Shared {
    fn new(db: Arc<::redb::Database>) -> Self {
        Self {
            window: Mutex::new(None),
            _db: db,
            slots: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
            window_open: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // Último tenedor de la base: publicar lo pendiente antes de que
        // `db` caiga (ver el doc del campo `window`).
        if let Some(window) = self.window.get_mut().unwrap_or_else(|e| e.into_inner()).take()
            && let Err(e) = window.txn.commit()
        {
            log::error!("redb: falló el commit de la última ventana al soltar la base: {e}");
        }
    }
}

type ReadSlots = Arc<Shared>;

/// Se llama DESPUÉS de cada `commit()` de datos: vacía los slots de lectura
/// (cualquier snapshot anterior es viejo, y soltarlo deja que redb recicle
/// las páginas que el commit liberó) y marca la base como pendiente de
/// checkpoint. Un checkpoint durable (commit vacío) no cambia datos y no
/// pasa por aquí.
fn invalidate_reads(shared: &ReadSlots) {
    shared.dirty.store(true, Ordering::Release);
    drop_read_slots(shared);
}

/// Vacía los slots de lectura sin marcar la base como sucia (crear una
/// tabla vacía no necesita checkpoint: al reabrir se recrea).
fn drop_read_slots(shared: &Shared) {
    let mut slots = shared.slots.lock().unwrap_or_else(|e| e.into_inner());
    slots.retain(|w| match w.upgrade() {
        Some(slot) => {
            *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
            true
        }
        None => false,
    });
}

fn poisoned_error() -> NopalError {
    StorageError::new(
        StorageErrorKind::Unsupported,
        "redb: el commit de una ventana de escritura falló; el motor no acepta más escrituras hasta reabrir la base",
    )
    .into()
}

/// Commitea la ventana si la hay. El llamador sostiene el mutex.
fn commit_window_locked(guard: &mut Option<WriteWindow>, shared: &ReadSlots) -> Result<()> {
    let Some(window) = guard.take() else { return Ok(()) };
    let ops = window.ops;
    let out = match window.txn.commit() {
        Ok(()) => {
            log::trace!("redb: ventana commiteada ({ops} ops)");
            invalidate_reads(shared);
            Ok(())
        }
        Err(e) => {
            shared.poisoned.store(true, Ordering::Release);
            log::error!("redb: falló el commit de una ventana con {ops} operaciones ya confirmadas: {e}");
            Err(internal(e))
        }
    };
    // DESPUÉS de commitear e invalidar las tablas cacheadas: un lector que
    // vea `window_open == false` tiene que encontrar ya el árbol nuevo. Si
    // lo ve `true`, espera en el mutex y al entrar no hay ventana: cae al
    // árbol, que para entonces también es el nuevo.
    shared.window_open.store(false, Ordering::Release);
    out
}

/// Commitea la ventana abierta, si la hay.
fn commit_window(shared: &ReadSlots) -> Result<()> {
    if !shared.window_open.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut guard = shared.window.lock().unwrap_or_else(|e| e.into_inner());
    commit_window_locked(&mut guard, shared)
}

/// Ejecuta `f` sobre la ventana (abriéndola si hace falta) y la commitea
/// después si ya está vencida por edad u operaciones.
fn with_window<R>(
    db: &::redb::Database,
    shared: &ReadSlots,
    f: impl FnOnce(&::redb::WriteTransaction, &mut Overlay) -> Result<R>,
) -> Result<R> {
    if shared.poisoned.load(Ordering::Acquire) {
        return Err(poisoned_error());
    }
    let mut guard = shared.window.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut txn = db.begin_write().map_err(internal)?;
        txn.set_durability(::redb::Durability::None).map_err(internal)?;
        *guard = Some(WriteWindow {
            txn,
            overlay: HashMap::new(),
            opened: Instant::now(),
            ops: 0,
        });
        shared.window_open.store(true, Ordering::Release);
    }
    let window = guard.as_mut().expect("ventana recién abierta");
    let out = f(&window.txn, &mut window.overlay)?;
    window.ops += 1;
    if window.is_stale() {
        commit_window_locked(&mut guard, shared)?;
    }
    Ok(out)
}

/// Valor pendiente en la ventana para `(keyspace, key)`: `Some(Some(v))`
/// escritura pendiente, `Some(None)` borrado pendiente, `None` no está en
/// la ventana (leer el árbol commiteado).
fn overlay_lookup(shared: &Shared, keyspace: &str, key: &[u8]) -> Option<Option<Vec<u8>>> {
    if !shared.window_open.load(Ordering::Acquire) {
        return None;
    }
    let guard = shared.window.lock().unwrap_or_else(|e| e.into_inner());
    let window = guard.as_ref()?;
    window.overlay.get(keyspace)?.get(key).cloned()
}

/// Anota en el overlay una escritura (`Some`) o un borrado (`None`).
fn overlay_put(overlay: &mut Overlay, keyspace: &str, key: &[u8], value: Option<&[u8]>) {
    overlay
        .entry(keyspace.to_string())
        .or_default()
        .insert(key.to_vec(), value.map(<[u8]>::to_vec));
}

/// Aplica un `WriteBatch` a la tabla `name` dentro de la transacción y lo
/// refleja en el overlay.
fn apply_batch_in(
    txn: &::redb::WriteTransaction,
    overlay: &mut Overlay,
    name: &str,
    batch: &WriteBatch,
) -> Result<()> {
    let mut table = txn.open_table(table_def(name)).map_err(internal)?;
    for (key, value) in batch.ops() {
        match value {
            Some(v) => {
                table.insert(key.as_slice(), v.as_slice()).map_err(NopalError::from)?;
            }
            None => {
                table.remove(key.as_slice()).map_err(NopalError::from)?;
            }
        }
        overlay_put(overlay, name, key, value.as_deref());
    }
    Ok(())
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
        // En Unix el handle recién creado se conserva y se renombra el
        // archivo abierto (válido: el descriptor y su lock siguen al inodo);
        // eso ahorra cerrar (fsync de cierre de redb) y volver a abrir la
        // base, ~20 ms de los ~75 que costaba crear una base en un Mac. En
        // Windows renombrar un archivo abierto falla, y esta librería
        // publica wheels para Windows: ahí se cierra antes de renombrar y se
        // reabre, como siempre. El invariante es el mismo en los dos: el
        // rename es el punto de publicación y ocurre con la base completa.
        if !path.exists() {
            let tmp = dir.join(TMP_DB_FILE);
            // Restos de un intento anterior que murió antes del rename.
            if tmp.exists() {
                std::fs::remove_file(&tmp)?;
            }
            let created = Self::builder_for(profile).create(&tmp).map_err(internal)?;
            #[cfg(unix)]
            {
                std::fs::rename(&tmp, &path)?;
                return Ok(Self::with_flusher(created, profile));
            }
            #[cfg(not(unix))]
            {
                drop(created);
                std::fs::rename(&tmp, &path)?;
            }
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
        let shared: ReadSlots = Arc::new(Shared::new(Arc::clone(&db)));
        let flusher = profile.tuning().flush_every_ms.map(|ms| {
            let db = Arc::clone(&db);
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let period = Duration::from_millis(ms);
                let mut last_checkpoint = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    // Con una ventana abierta el hilo despierta cada
                    // WINDOW_MAX_AGE para cerrarla aunque no llegue otra
                    // escritura; si no, solo para el checkpoint periódico.
                    let wait = if shared.window_open.load(Ordering::Acquire) {
                        WINDOW_MAX_AGE
                    } else {
                        period.saturating_sub(last_checkpoint.elapsed())
                    };
                    std::thread::park_timeout(wait);
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if last_checkpoint.elapsed() >= period {
                        // Commit vacío durable: persiste todos los commits None previos.
                        let _ = checkpoint_if_dirty(&db, &shared);
                        last_checkpoint = Instant::now();
                    } else {
                        let _ = commit_window_if_stale(&shared);
                    }
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

/// Commitea la ventana si ya venció (edad u operaciones). Lo usa el tick
/// del flusher para que una escritura suelta no espere a la siguiente.
fn commit_window_if_stale(shared: &ReadSlots) -> Result<()> {
    if !shared.window_open.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut guard = shared.window.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().is_some_and(WriteWindow::is_stale) {
        commit_window_locked(&mut guard, shared)?;
    }
    Ok(())
}

/// Checkpoint durable solo si hubo commits de datos desde el anterior. Si
/// el fsync falla, la base vuelve a quedar marcada para reintentarlo.
///
/// Sostiene el mutex de la ventana durante todo el checkpoint: primero la
/// commitea (si la hay) y luego hace el commit `Immediate`; sin el mutex,
/// otro hilo podría abrir una ventana nueva entre ambos y el `begin_write`
/// del checkpoint se quedaría esperando a una ventana que solo cierra...
/// este mismo hilo.
fn checkpoint_if_dirty(db: &::redb::Database, shared: &ReadSlots) -> Result<()> {
    let mut guard = shared.window.lock().unwrap_or_else(|e| e.into_inner());
    commit_window_locked(&mut guard, shared)?;
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
        Ok(self.keyspaces(&[name])?.remove(0))
    }

    fn keyspaces(&self, names: &[&str]) -> Result<Vec<Arc<dyn KvKeyspace>>> {
        // Crea las tablas que falten en UNA transacción: los reads
        // posteriores no lidian con TableDoesNotExist y los handles quedan
        // usables de inmediato. `Storage` abre diez al arrancar; antes eran
        // diez commits (y diez `pwrite` de raíz) por apertura.
        {
            let mut guard = self.read_slots.window.lock().unwrap_or_else(|e| e.into_inner());
            commit_window_locked(&mut guard, &self.read_slots)?;
            let mut txn = self.db.begin_write().map_err(internal)?;
            txn.set_durability(::redb::Durability::None).map_err(internal)?;
            for name in names {
                txn.open_table(table_def(name)).map_err(internal)?;
            }
            txn.commit().map_err(internal)?;
        }
        // Crear tablas vacías no marca la base como sucia: un checkpoint
        // solo por esto sería un fsync inútil (al reabrir se recrean igual).
        drop_read_slots(&self.read_slots);
        let mut slots = self.read_slots.slots.lock().unwrap_or_else(|e| e.into_inner());
        Ok(names
            .iter()
            .map(|name| {
                let slot: Arc<ReadSlot> = Arc::new(RwLock::new(None));
                slots.push(Arc::downgrade(&slot));
                Arc::new(RedbKeyspace {
                    db: Arc::clone(&self.db),
                    name: name.to_string(),
                    slot,
                    read_slots: Arc::clone(&self.read_slots),
                }) as Arc<dyn KvKeyspace>
            })
            .collect())
    }

    fn apply_multi(&self, batches: Vec<(String, WriteBatch)>) -> Result<()> {
        // UNA write-txn cubre todos los keyspaces: el commit único es la
        // atomicidad cross-keyspace (Durability::None, mismo contrato de
        // visibilidad-sin-fsync que el resto de escrituras). Cada tabla se
        // abre y se cierra por batch — reabrir la misma tabla más adelante
        // en la txn es válido porque el handle anterior ya se soltó, y así
        // los keyspaces repetidos se aplican en orden.
        with_window(&self.db, &self.read_slots, |txn, overlay| {
            for (name, batch) in &batches {
                apply_batch_in(txn, overlay, name, batch)?;
            }
            Ok(())
        })
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
    /// Aplica `batch` a este keyspace dentro de la ventana de commit.
    fn write_batch(&self, batch: &WriteBatch) -> Result<()> {
        with_window(&self.db, &self.read_slots, |txn, overlay| {
            apply_batch_in(txn, overlay, &self.name, batch)
        })
    }

    /// Valor vigente de `key` para este proceso: el pendiente en la ventana
    /// si lo hay, si no el del árbol commiteado.
    fn current_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(pending) = overlay_lookup(&self.read_slots, &self.name, key) {
            return Ok(pending);
        }
        self.with_read_table(|table| {
            Ok(table
                .get(key)
                .map_err(NopalError::from)?
                .map(|v| v.value().to_vec()))
        })
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
        // Un scan no fusiona el overlay con el árbol: publica primero lo
        // pendiente y escanea el estado commiteado (ver `WriteWindow`).
        commit_window(&self.ks.read_slots)?;
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
        self.current_value(key)
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let mut batch = WriteBatch::default();
        batch.insert(key.to_vec(), value.to_vec());
        self.write_batch(&batch)
    }

    fn remove(&self, key: &[u8]) -> Result<()> {
        let mut batch = WriteBatch::default();
        batch.remove(key.to_vec());
        self.write_batch(&batch)
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
        // Todo el batch entra en la misma transacción (la ventana): o se
        // publica entero en el siguiente commit, o nada.
        self.write_batch(&batch)
    }

    fn rmw(&self, key: &[u8], f: &mut RmwFn<'_>) -> Result<()> {
        // Leer-modificar-escribir dentro de la misma transacción abierta:
        // el single-writer de redb (y el mutex de la ventana) lo hacen
        // atómico sin CAS-loop. `Table::get` sobre la write-txn ve lo que
        // esta misma ventana ya escribió.
        with_window(&self.db, &self.read_slots, |txn, overlay| {
            use ::redb::ReadableTable;
            let mut table = txn.open_table(table_def(&self.name)).map_err(internal)?;
            let old = table
                .get(key)
                .map_err(NopalError::from)?
                .map(|v| v.value().to_vec());
            match f(old.as_deref()) {
                Some(new) => {
                    table.insert(key, new.as_slice()).map_err(NopalError::from)?;
                    overlay_put(overlay, &self.name, key, Some(&new));
                }
                None => {
                    table.remove(key).map_err(NopalError::from)?;
                    overlay_put(overlay, &self.name, key, None);
                }
            }
            Ok(())
        })
    }

    fn clear(&self) -> Result<()> {
        // Bajo el mutex de la ventana: se commitea lo pendiente (que puede
        // incluir claves de esta tabla) y se borra/recrea en su propia txn.
        let mut guard = self.read_slots.window.lock().unwrap_or_else(|e| e.into_inner());
        commit_window_locked(&mut guard, &self.read_slots)?;
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

    fn window_open(engine: &RedbEngine) -> bool {
        engine.read_slots.window_open.load(Ordering::Acquire)
    }

    /// El mecanismo de la caché de lectura: un `get` deja la tabla cacheada
    /// y cualquier commit de datos (propio, de otro keyspace vía
    /// `apply_multi`, `clear`, cierre de ventana) la vacía. Con la ventana
    /// de commit, una escritura NO commitea de inmediato: el slot puede
    /// seguir cacheado, pero `get` consulta el overlay antes, así que el
    /// resultado siempre es el vigente.
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

        // Escritura propia: visible al instante (overlay), aún sin commit.
        a.insert(b"k", b"1").unwrap();
        assert!(window_open(&engine), "la escritura queda en la ventana");
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"1"[..]));
        // El commit de la ventana vacía el slot; la lectura siguiente lo rellena.
        engine.flush().unwrap();
        assert!(!window_open(&engine));
        assert!(!slot_is_cached(&a), "un commit vacía el slot");
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"1"[..]));
        assert!(slot_is_cached(&a));

        // Escritura en OTRO keyspace vía apply_multi: visible en `b` al
        // instante, y al commitear invalida también a `a` (el snapshot es
        // de toda la base, no de una tabla).
        let mut batch = WriteBatch::default();
        batch.insert(b"x".to_vec(), b"y".to_vec());
        engine.apply_multi(vec![("b".to_string(), batch)]).unwrap();
        assert_eq!(b.get(b"x").unwrap().as_deref(), Some(&b"y"[..]));
        engine.flush().unwrap();
        assert!(!slot_is_cached(&a));

        // rmw y clear: lo escrito se ve de inmediato.
        a.rmw(b"k", &mut |old| old.map(|v| [v, b"2"].concat())).unwrap();
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"12"[..]));
        a.clear().unwrap();
        assert_eq!(a.get(b"k").unwrap(), None);

        // Un scan por chunks intercalado con una escritura continúa sobre el
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

    /// La ventana de commit: read-your-writes sin commit, borrados y rmw
    /// pendientes, visibilidad cross-keyspace, y los cuatro cierres (scan,
    /// tope de ops, edad vía tick, flush).
    #[test]
    fn write_window_batches_commits_and_never_hides_a_write() {
        let engine = RedbEngine::open_temporary(StorageProfile::Default).unwrap();
        let a = engine.keyspace("a").unwrap();
        let b = engine.keyspace("b").unwrap();

        // 1. read-your-writes con la ventana abierta
        a.insert(b"k", b"v1").unwrap();
        assert!(window_open(&engine));
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"v1"[..]));
        assert!(a.contains_key(b"k").unwrap());

        // 2. borrado pendiente tapa el valor commiteado; rmw ve lo pendiente
        engine.flush().unwrap(); // v1 commiteado
        a.remove(b"k").unwrap();
        assert!(window_open(&engine));
        assert_eq!(a.get(b"k").unwrap(), None, "el borrado pendiente se ve");
        a.insert(b"k", b"v2").unwrap();
        a.rmw(b"k", &mut |old| {
            assert_eq!(old, Some(&b"v2"[..]), "rmw ve el valor pendiente");
            Some(b"v3".to_vec())
        })
        .unwrap();
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"v3"[..]));

        // 3. apply_multi en otro keyspace, visible al instante
        let mut batch = WriteBatch::default();
        batch.insert(b"x".to_vec(), b"y".to_vec());
        engine.apply_multi(vec![("b".to_string(), batch)]).unwrap();
        assert_eq!(b.get(b"x").unwrap().as_deref(), Some(&b"y"[..]));

        // 4. un scan cierra la ventana y devuelve lo pendiente
        assert!(window_open(&engine));
        let keys: Vec<Vec<u8>> = a.iter().map(|r| r.unwrap().0).collect();
        assert_eq!(keys, vec![b"k".to_vec()]);
        assert!(!window_open(&engine), "el scan commiteó la ventana");
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"v3"[..]));

        // 5. tope de operaciones: tras WINDOW_MAX_OPS escrituras la ventana
        //    se cerró sola (y las claves están, ventana nueva o no)
        for i in 0..WINDOW_MAX_OPS {
            a.insert(&(i as u32).to_be_bytes(), b"n").unwrap();
        }
        assert!(!window_open(&engine), "la op número WINDOW_MAX_OPS commitea");
        a.insert(b"one-more", b"n").unwrap();
        assert!(window_open(&engine));
        assert_eq!(a.get(&0u32.to_be_bytes()).unwrap().as_deref(), Some(&b"n"[..]));

        // 6. tope de edad: el tick del flusher cierra una ventana sola
        let deadline = Instant::now() + Duration::from_secs(2);
        while window_open(&engine) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!window_open(&engine), "el flusher debe cerrar la ventana por edad");
        assert_eq!(a.get(b"one-more").unwrap().as_deref(), Some(&b"n"[..]));

        // 7. flush cierra la ventana y deja todo legible tras reabrir la tabla
        a.insert(b"last", b"z").unwrap();
        engine.flush().unwrap();
        assert!(!window_open(&engine));
        assert_eq!(a.get(b"last").unwrap().as_deref(), Some(&b"z"[..]));
    }

    /// Un keyspace que sobrevive a su engine (patrón de varios tests de
    /// `Storage`) puede escribir, leer, escanear y soltarse con la ventana
    /// abierta sin colgar el proceso: `Database::drop` abre una write-txn y
    /// esperaría para siempre a una ventana viva si esta no se cerrara antes.
    #[test]
    fn keyspace_outliving_its_engine_drops_cleanly_with_an_open_window() {
        let ks = {
            let engine = RedbEngine::open_temporary(StorageProfile::Default).unwrap();
            engine.keyspace("catalog").unwrap()
        };
        ks.insert(b"k", b"v").unwrap();
        assert_eq!(ks.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(ks.scan_prefix(b"k").count(), 1);
        ks.insert(b"k2", b"v").unwrap(); // ventana abierta al soltar `ks`
        drop(ks);
    }

    /// `Drop` con la ventana abierta no pierde datos: se commitea y se hace
    /// el checkpoint durable antes de soltar la base.
    #[test]
    fn dropping_the_engine_commits_the_open_window() {
        let dir = tempfile::tempdir().unwrap();
        {
            let engine = RedbEngine::open(dir.path(), StorageProfile::Default).unwrap();
            let a = engine.keyspace("a").unwrap();
            a.insert(b"k", b"kept").unwrap();
            assert!(window_open(&engine));
            // drop sin flush()
        }
        let engine = RedbEngine::open(dir.path(), StorageProfile::Default).unwrap();
        let a = engine.keyspace("a").unwrap();
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(&b"kept"[..]));
    }
}
