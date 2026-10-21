// Capa KV: el contrato que desacopla `Storage` (la capa de dominio: MVCC,
// adyacencia, índices) de cualquier motor KV concreto, más las
// implementaciones por motor.
//
// El contrato se diseñó mapeando 1:1 la superficie de sled realmente usada
// por `storage/mod.rs` — nada de superficie sin consumidor (snapshots,
// merge operators, CAS) entra hasta que exista quien la use. Los nombres de
// método siguen la convención de sled (`insert`/`remove`/`contains_key`)
// a propósito: el rewire de ~100 call sites fue un cambio de receptor, no
// de vocabulario, y eso mantiene el diff auditable. La semántica exigible
// es la del rustdoc de cada método, no la del motor que dio el nombre.
//
// El módulo vive como `kv` (no `sled`/`redb`) para no sombrear los crates
// de los motores: `mod sled` dentro de `storage` ocultaría al crate `sled`
// para todas las rutas del módulo padre.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::storage::backend::{StorageOptions, StorageProfile};

pub(crate) mod redb;

pub(crate) mod migrate;

#[cfg(test)]
mod conformance;

/// Nombre del keyspace por defecto (el "tree default" de sled; una tabla
/// homónima en motores con tablas). Contiene los namespaces compuestos
/// `node:` / `idx:out:` / `idx:in:` / `ts:` / `meta:`.
pub(crate) const DEFAULT_KEYSPACE: &str = "default";

/// Par clave/valor con bytes OWNED. Los guards/slices con lifetime del motor
/// (IVec de sled, AccessGuard de redb) no cruzan esta frontera: se copian.
/// El costo se midió al hacer el rewire; si algún día domina un perfil, la
/// contingencia diseñada es un `OwnedBytes` con `Deref` (no cambiar el trait).
pub(crate) type KvPair = (Vec<u8>, Vec<u8>);

/// Iterador de pares en orden LEXICOGRÁFICO ASCENDENTE de clave, boxed para
/// object-safety. Cada item puede fallar (I/O del motor).
pub(crate) type KvIter<'a> = Box<dyn Iterator<Item = Result<KvPair>> + Send + 'a>;

/// Closure de `KvKeyspace::rmw`: valor actual → valor nuevo (`None` = borrar).
/// Alias exigido por clippy (type_complexity); el contrato vive en el rustdoc
/// de `rmw`.
pub(crate) type RmwFn<'a> = dyn FnMut(Option<&[u8]>) -> Option<Vec<u8>> + 'a;

/// Write-set que se aplica atómicamente sobre UN keyspace.
///
/// Es el contrato de `sled::Batch` + `apply_batch`: atómico y crash-safe
/// dentro del keyspace. Motores con transacciones globales (redb, fjall) lo
/// satisfacen trivialmente. `apply_batch` sigue siendo la vía
/// single-keyspace; la atomicidad ENTRE keyspaces distintos la promete
/// `KvEngine::apply_multi`, que recibe varios `WriteBatch` (uno por
/// keyspace) y los aplica como una sola transacción.
#[derive(Default)]
pub(crate) struct WriteBatch {
    ops: Vec<(Vec<u8>, Option<Vec<u8>>)>, // Some = insert, None = remove
}

impl WriteBatch {
    pub fn insert(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.ops.push((key.into(), Some(value.into())));
    }

    pub fn remove(&mut self, key: impl Into<Vec<u8>>) {
        self.ops.push((key.into(), None));
    }

    pub fn ops(&self) -> &[(Vec<u8>, Option<Vec<u8>>)] {
        &self.ops
    }
}

/// Un keyspace nombrado de un motor KV (sled::Tree, tabla de redb, keyspace
/// de fjall). Object-safe y sync: los motores embebidos son síncronos y la
/// capa async vive arriba, en `Storage`.
///
/// # Invariantes exigibles a toda implementación
///
/// (pinneadas en `conformance.rs` de este módulo — ningún motor entra al
/// dispatch de `open_engine` sin pasar la suite completa)
///
/// - **Orden**: `iter`/`scan_prefix`/`range_from` recorren en orden
///   lexicográfico ascendente de bytes de clave. Enteros big-endian ordenan
///   numéricamente.
/// - **Prefijos**: `scan_prefix(p)` produce exactamente las claves con
///   prefijo `p` — nunca la clave inmediatamente posterior al rango.
/// - **`range_from(start)`**: produce claves `>= start` (inclusivo); el
///   caller que pagina con cursor filtra el propio cursor si no lo quiere
///   (así lo hace `scan_nodes_batch`).
/// - **Atomicidad**: `apply_batch` aplica todo o nada, incluso ante crash
///   (la garantía de recuperación es la del motor; con durabilidad diferida
///   el batch puede perderse COMPLETO, jamás verse a medias).
/// - **Última escritura gana** dentro de un batch para la misma clave.
/// - **Visibilidad**: toda escritura confirmada es visible para cualquier
///   lectura posterior del proceso (read-your-writes local).
/// - **Durabilidad**: las escrituras individuales son duraderas según la
///   política del engine (flush periódico); `KvEngine::flush` fuerza fsync.
/// - **`rmw` es atómico** respecto a otros `rmw`/escrituras de la misma
///   clave (CAS-loop o lock interno del motor, según implementación).
pub(crate) trait KvKeyspace: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    /// Varios `get` de una vez, en el orden de `keys` (`None` donde la
    /// clave no existe; una clave repetida se devuelve repetida). Igual que
    /// llamar a `get` por clave; un motor puede resolverlas todas bajo una
    /// sola tabla/transacción de lectura (#224). Hoy lo usa la carga de
    /// embeddings por conjunto; sin esa feature queda sin llamador.
    #[cfg_attr(not(feature = "embeddings"), allow(dead_code))]
    fn get_many(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
        keys.iter().map(|k| self.get(k)).collect()
    }
    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()>;
    fn remove(&self, key: &[u8]) -> Result<()>;
    fn contains_key(&self, key: &[u8]) -> Result<bool>;
    fn iter(&self) -> KvIter<'_>;
    fn scan_prefix(&self, prefix: &[u8]) -> KvIter<'_>;
    fn range_from(&self, start: &[u8]) -> KvIter<'_>;
    fn apply_batch(&self, batch: WriteBatch) -> Result<()>;
    /// Read-modify-write atómico de UNA clave. `f` recibe el valor actual y
    /// devuelve el nuevo (`None` = borrar). Puede ejecutarse más de una vez
    /// (CAS-loop) — debe ser pura. Único uso hoy: los relojes lógicos
    /// (`put_meta_u64_max`), que escriben el máximo monótono.
    fn rmw(&self, key: &[u8], f: &mut RmwFn<'_>) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// Un motor KV abierto. Entrega handles de keyspace cacheables (baratos de
/// clonar; `Storage` los cachea en su constructor).
pub(crate) trait KvEngine: Send + Sync {
    fn engine_name(&self) -> &'static str;
    fn keyspace(&self, name: &str) -> Result<Arc<dyn KvKeyspace>>;
    /// Varios keyspaces de una vez, en el mismo orden que `names`. Igual
    /// que llamar a `keyspace` por cada nombre, salvo que un motor puede
    /// crear todas las tablas en una sola transacción (`Storage` abre diez
    /// al arrancar; en redb cada creación era un commit).
    fn keyspaces(&self, names: &[&str]) -> Result<Vec<Arc<dyn KvKeyspace>>> {
        names.iter().map(|n| self.keyspace(n)).collect()
    }
    /// Aplica varios `WriteBatch` — cada uno sobre el keyspace nombrado —
    /// como UNA transacción TODO-o-nada CRUZANDO keyspaces, incluso ante
    /// crash: o se ven todos los cambios de todos los batches, o ninguno
    /// (con durabilidad diferida el conjunto puede perderse COMPLETO, jamás
    /// verse a medias ni parcialmente por keyspace). Keyspaces repetidos en
    /// el `Vec` están permitidos: sus batches se aplican en orden, así que
    /// la última escritura de una clave gana. `Vec` vacío es no-op.
    ///
    /// Es la promesa cross-keyspace que el doc de `WriteBatch` dejó
    /// diferida hasta tener caller: el layout v2 (F5.3) lo es — una arista y
    /// sus entradas de adyacencia viven en keyspaces distintos y jamás
    /// deben verse a medias (`Storage::insert_edge_with_adjacency` y
    /// compañía). Para escribir en UN solo keyspace,
    /// `KvKeyspace::apply_batch` sigue siendo la vía.
    fn apply_multi(&self, batches: Vec<(String, WriteBatch)>) -> Result<()>;
    /// Fuerza durabilidad de todo lo escrito (fsync). Síncrono y bloqueante.
    fn flush(&self) -> Result<()>;
}

/// Rutas de las bases abiertas en ESTE proceso (canonicalizadas).
///
/// Ningún motor puede decir si el lock del directorio lo tiene otro proceso
/// o este mismo: el flock no lleva PID, y en redb `DatabaseAlreadyOpen` es
/// justamente la doble apertura en el mismo proceso. Sin este registro,
/// reabrir una ruta que este proceso todavía tiene abierta —el caso típico:
/// `graph.close()` sin soltar el valor— esperaba el reintento completo del
/// lock y fallaba culpando a "otro proceso". Un `Vec` porque `Vec::new` es
/// `const` y aquí caben pocas rutas.
static OPEN_PATHS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Reserva de una ruta en [`OPEN_PATHS`] que se libera al soltarla. Vive
/// dentro del engine, así que se libera exactamente cuando el engine muere,
/// que es cuando el motor suelta el flock.
pub(crate) struct PathLease(PathBuf);

impl PathLease {
    /// Registra `path`; `Err` inmediato (sin reintento) si este proceso ya
    /// tiene esa ruta abierta.
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        // Canonicalizar exige que exista; los motores crean el directorio
        // igual, así que hacerlo aquí no cambia nada observable.
        std::fs::create_dir_all(path)?;
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut open = OPEN_PATHS.lock().unwrap_or_else(|e| e.into_inner());
        if open.contains(&canonical) {
            return Err(crate::error::StorageError::new(
                crate::error::StorageErrorKind::Unsupported,
                format!(
                    "la base en {} ya está abierta en este proceso: el lock del directorio se \
                     libera al soltar el `Graph` (drop), no en `close()`. Descarta el valor \
                     anterior antes de reabrir, o abre una copia.",
                    path.display()
                ),
            )
            .into());
        }
        open.push(canonical.clone());
        Ok(Self(canonical))
    }
}

impl Drop for PathLease {
    fn drop(&mut self) {
        let mut open = OPEN_PATHS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = open.iter().position(|p| *p == self.0) {
            open.swap_remove(i);
        }
    }
}

/// Abre un motor persistente en `path` según las opciones. Único punto del
/// crate con dispatch por engine — un backend nuevo agrega su brazo aquí.
///
/// Antes de tocar el motor se reserva la ruta en [`OPEN_PATHS`]: si este
/// proceso ya la tiene abierta, el error es inmediato y dice cómo salir
/// (soltar el `Graph`), en vez de esperar el reintento del lock y culpar a
/// otro proceso.
pub(crate) fn open_engine(
    path: &Path,
    profile: StorageProfile,
    options: &StorageOptions,
) -> Result<Arc<dyn KvEngine>> {
    resolve_engine(path, options.engine)?;
    let lease = PathLease::acquire(path)?;
    Ok(Arc::new(redb::RedbEngine::open(path, profile)?.with_lease(lease)))
}

/// Abre un motor efímero en memoria (tests, `Graph::in_memory`).
pub(crate) fn open_in_memory(
    profile: StorageProfile,
    options: &StorageOptions,
) -> Result<Arc<dyn KvEngine>> {
    use crate::storage::backend::StorageEngine;
    if options.engine == StorageEngine::Sled {
        return Err(sled_removed(None));
    }
    Ok(Arc::new(redb::RedbEngine::open_temporary(profile)?))
}

/// Qué motor dejó la base que ya hay en `dir`, por sus archivos: redb crea
/// `nopal.redb`; sled crea `conf` y `db`. `None` si no hay base (directorio
/// nuevo o vacío). sled ya no se compila (retirado en 0.8.0, #141): se sigue
/// reconociendo solo para que abrir una base de 0.5.x falle con la receta de
/// migración y no con un error críptico de redb.
pub fn detect_engine(dir: &Path) -> Option<crate::storage::backend::StorageEngine> {
    use crate::storage::backend::StorageEngine;
    if dir.join(REDB_DB_FILE).exists() {
        Some(StorageEngine::Redb)
    } else if dir.join("conf").exists() && dir.join("db").exists() {
        Some(StorageEngine::Sled)
    } else {
        None
    }
}

/// Archivo de la base redb (espejo de `redb::DB_FILE`).
const REDB_DB_FILE: &str = "nopal.redb";

/// El motor con que se abre `dir`: siempre redb. Pedir sled, o `Auto` sobre
/// un directorio con una base sled de 0.5.x, da el error con la receta de
/// migración (antes de crear nada en el directorio).
pub(crate) fn resolve_engine(
    dir: &Path,
    requested: crate::storage::backend::StorageEngine,
) -> Result<crate::storage::backend::StorageEngine> {
    use crate::storage::backend::StorageEngine;
    let sled_dir = detect_engine(dir) == Some(StorageEngine::Sled);
    match requested {
        StorageEngine::Sled => Err(sled_removed(Some(dir))),
        StorageEngine::Auto if sled_dir => Err(sled_removed(Some(dir))),
        _ => Ok(StorageEngine::Redb),
    }
}

/// sled se retiró en 0.8.0 (#141). Pedirlo, o abrir con `Auto` un
/// directorio que contiene una base sled, da este error con la receta: la
/// última versión que lee sled es la 0.7, y su wheel la copia a redb.
pub(crate) const SLED_REMOVED_RECIPE: &str = "sled support was removed in NopalDB 0.8.0. Migrate the \
     database to redb with NopalDB 0.7, the last version that reads sled: \
     `pip install \"nopaldb==0.7.*\"` and `nopaldb.Graph.migrate(src, dst)`, or \
     `cargo install nopaldb --version \"0.7.*\" --features cli` and \
     `nopaldb migrate <src> <dst>`; then open the copy with this version. See \
     docs/MIGRATION_0.6.md.";

fn sled_removed(dir: Option<&Path>) -> crate::error::NopalError {
    let what = match dir {
        Some(d) if detect_engine(d) == Some(crate::storage::backend::StorageEngine::Sled) => {
            format!("{} contains a sled database. ", d.display())
        }
        _ => String::new(),
    };
    crate::error::StorageError::new(
        crate::error::StorageErrorKind::Unsupported,
        format!("{what}{SLED_REMOVED_RECIPE}"),
    )
    .into()
}

#[cfg(test)]
mod engine_availability_tests {
    use super::*;
    use crate::storage::backend::StorageEngine;

    /// sled (retirado en 0.8.0) no tiene feature que nombrar: la receta dice
    /// con qué versión migrar.
    #[test]
    fn sled_error_gives_the_migration_recipe() {
        let msg = format!("{}", sled_removed(None));
        assert!(msg.contains("0.7") && msg.contains("Graph.migrate"), "receta de migración: {msg}");
    }

    /// Una base sled (sus archivos) abierta con `Auto` o pidiendo sled: el
    /// error dice que es sled y cómo migrarla, en vez de un fallo de redb.
    #[test]
    fn a_sled_directory_fails_with_the_migration_recipe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conf"), b"").unwrap();
        std::fs::write(dir.path().join("db"), b"").unwrap();
        for requested in [StorageEngine::Auto, StorageEngine::Sled] {
            let err = resolve_engine(dir.path(), requested).expect_err("sled ya no abre");
            let msg = format!("{err}");
            assert!(msg.contains("contains a sled database"), "{msg}");
            assert!(msg.contains("nopaldb==0.7"), "{msg}");
        }
    }

    /// El engine que SÍ está compilado abre; el dispatch no cae al error.
    #[test]
    fn compiled_engine_opens_temporary() {
        let opts = crate::storage::backend::StorageOptions::default();
        assert!(
            open_in_memory(crate::storage::backend::StorageProfile::Default, &opts).is_ok(),
            "el engine por defecto de este build debe abrir"
        );
    }

    /// Pedir sled (retirado) falla con el error accionable en vez de con un
    /// panic.
    #[test]
    fn missing_backend_is_an_error_not_a_panic() {
        let (missing, feature) = (StorageEngine::Sled, "nopaldb==0.7");
        let opts = crate::storage::backend::StorageOptions {
            engine: missing,
            profile: crate::storage::backend::StorageProfile::Default,
            ..Default::default()
        };
        let Err(err) = open_in_memory(crate::storage::backend::StorageProfile::Default, &opts)
        else {
            panic!("{missing:?} no está compilado en este build: abrir debe fallar");
        };
        let msg = format!("{err}");
        assert!(msg.contains(feature), "mensaje accionable: {msg}");
    }

    /// `detect_engine` reconoce los archivos de cada motor y nada más.
    #[test]
    fn detect_engine_by_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_engine(dir.path()), None);
        std::fs::write(dir.path().join("conf"), b"").unwrap();
        assert_eq!(detect_engine(dir.path()), None, "solo `conf` no es una base sled");
        std::fs::write(dir.path().join("db"), b"").unwrap();
        assert_eq!(detect_engine(dir.path()), Some(StorageEngine::Sled));
        std::fs::write(dir.path().join("nopal.redb"), b"").unwrap();
        assert_eq!(detect_engine(dir.path()), Some(StorageEngine::Redb), "nopal.redb manda");
    }
}

// ─── Sello de solo-lectura ──────────────────────────────────────────────────

/// Interruptor compartido que convierte un engine en solo-lectura.
///
/// Empieza ABIERTO porque abrir la base escribe: se crean tablas y keyspaces,
/// se aplican migraciones de layout y se reproduce el WAL. Un engine que
/// rechazara escrituras desde el principio no daría una apertura de solo
/// lectura — haría fallar el `open`. El sello se cierra cuando la
/// inicialización terminó y la base ya está consistente.
#[derive(Clone, Default)]
pub(crate) struct WriteSeal(Arc<std::sync::atomic::AtomicBool>);

impl WriteSeal {
    pub(crate) fn open() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    /// Cierra el sello: a partir de aquí toda escritura falla.
    pub(crate) fn close(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn is_sealed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn deny(&self, que: &str) -> crate::error::NopalError {
        crate::error::StorageError::new(
            crate::error::StorageErrorKind::Unsupported,
            format!(
                "esta base se abrió en modo solo-lectura; `{que}` no está permitido. \
                 Abrirla con `Graph::open` para escribir."
            ),
        )
        .into()
    }
}

/// Engine que deja de aceptar escrituras cuando su sello se cierra.
///
/// Envuelve al ENGINE y no a los keyspaces ya abiertos a propósito: así
/// cubre tanto los que `Storage` cachea al abrir (todos los del layout
/// desde #224, embeddings incluidos) como cualquiera que se pida bajo
/// demanda después. Todo keyspace nace de `keyspace()`, así que
/// interceptarlo ahí los cubre a todos.
pub(crate) struct SealedEngine {
    inner: Arc<dyn KvEngine>,
    seal: WriteSeal,
}

impl SealedEngine {
    pub(crate) fn new(inner: Arc<dyn KvEngine>, seal: WriteSeal) -> Self {
        Self { inner, seal }
    }
}

impl KvEngine for SealedEngine {
    fn engine_name(&self) -> &'static str {
        self.inner.engine_name()
    }

    fn keyspace(&self, name: &str) -> Result<Arc<dyn KvKeyspace>> {
        // Abrir un keyspace CREA la tabla si no existe, así que también es
        // una escritura una vez sellado.
        if self.seal.is_sealed() {
            // Ya sellado: el keyspace debe existir; se envuelve igual para
            // que sus escrituras se rechacen.
            let inner = self.inner.keyspace(name)?;
            return Ok(Arc::new(SealedKeyspace { inner, seal: self.seal.clone() }));
        }
        let inner = self.inner.keyspace(name)?;
        Ok(Arc::new(SealedKeyspace { inner, seal: self.seal.clone() }))
    }

    fn keyspaces(&self, names: &[&str]) -> Result<Vec<Arc<dyn KvKeyspace>>> {
        // Misma regla que `keyspace`: sellado o no, el handle se envuelve.
        Ok(self
            .inner
            .keyspaces(names)?
            .into_iter()
            .map(|inner| Arc::new(SealedKeyspace { inner, seal: self.seal.clone() }) as Arc<dyn KvKeyspace>)
            .collect())
    }

    fn apply_multi(&self, batches: Vec<(String, WriteBatch)>) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("apply_multi"));
        }
        self.inner.apply_multi(batches)
    }

    fn flush(&self) -> Result<()> {
        // El flush no muta datos: fuerza a disco lo ya escrito. Se permite
        // siempre para que cerrar limpio siga funcionando.
        self.inner.flush()
    }
}

struct SealedKeyspace {
    inner: Arc<dyn KvKeyspace>,
    seal: WriteSeal,
}

impl KvKeyspace for SealedKeyspace {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn get_many(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
        // Reenviado a propósito: con el default del trait el lote se
        // desharía en `get`s sueltos al pasar por el sello.
        self.inner.get_many(keys)
    }
    fn contains_key(&self, key: &[u8]) -> Result<bool> {
        self.inner.contains_key(key)
    }
    fn iter(&self) -> KvIter<'_> {
        self.inner.iter()
    }
    fn scan_prefix(&self, prefix: &[u8]) -> KvIter<'_> {
        self.inner.scan_prefix(prefix)
    }
    fn range_from(&self, start: &[u8]) -> KvIter<'_> {
        self.inner.range_from(start)
    }

    fn insert(&self, key: &[u8], value: &[u8]) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("insert"));
        }
        self.inner.insert(key, value)
    }
    fn remove(&self, key: &[u8]) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("remove"));
        }
        self.inner.remove(key)
    }
    fn apply_batch(&self, batch: WriteBatch) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("apply_batch"));
        }
        self.inner.apply_batch(batch)
    }
    fn rmw(&self, key: &[u8], f: &mut RmwFn<'_>) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("rmw"));
        }
        self.inner.rmw(key, f)
    }
    fn clear(&self) -> Result<()> {
        if self.seal.is_sealed() {
            return Err(self.seal.deny("clear"));
        }
        self.inner.clear()
    }
}
