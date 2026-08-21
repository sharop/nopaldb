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

use std::path::Path;
use std::sync::Arc;

use crate::error::Result;
use crate::storage::backend::{StorageOptions, StorageProfile};

#[cfg(feature = "storage-sled")]
pub(crate) mod sled;

#[cfg(feature = "storage-redb")]
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

/// Abre un motor persistente en `path` según las opciones. Único punto del
/// crate con dispatch por engine — un backend nuevo agrega su brazo aquí.
pub(crate) fn open_engine(
    path: &Path,
    profile: StorageProfile,
    options: &StorageOptions,
) -> Result<Arc<dyn KvEngine>> {
    use crate::storage::backend::StorageEngine;
    match options.engine {
        #[cfg(feature = "storage-sled")]
        StorageEngine::Sled => Ok(Arc::new(sled::SledEngine::open(path, profile)?)),
        #[cfg(feature = "storage-redb")]
        StorageEngine::Redb => Ok(Arc::new(redb::RedbEngine::open(path, profile)?)),
        #[allow(unreachable_patterns)]
        other => Err(engine_not_compiled(other)),
    }
}

/// Abre un motor efímero en memoria (tests, `Graph::in_memory`).
pub(crate) fn open_in_memory(
    profile: StorageProfile,
    options: &StorageOptions,
) -> Result<Arc<dyn KvEngine>> {
    use crate::storage::backend::StorageEngine;
    match options.engine {
        #[cfg(feature = "storage-sled")]
        StorageEngine::Sled => Ok(Arc::new(sled::SledEngine::open_temporary(profile)?)),
        #[cfg(feature = "storage-redb")]
        StorageEngine::Redb => Ok(Arc::new(redb::RedbEngine::open_temporary(profile)?)),
        #[allow(unreachable_patterns)]
        other => Err(engine_not_compiled(other)),
    }
}

/// Error claro (no panic) cuando piden un engine cuyo backend no se compiló:
/// el enum es público y non_exhaustive — el valor puede llegar por API.
fn engine_not_compiled(engine: crate::storage::backend::StorageEngine) -> crate::error::NopalError {
    use crate::storage::backend::StorageEngine;
    // Nombrar la feature EXACTA, no el comodín `storage-*`: quien recibe este
    // error está a una línea de Cargo.toml de arreglarlo, y adivinar el
    // nombre es trabajo innecesario.
    // `StorageEngine` es `non_exhaustive` de puertas afuera, pero aquí dentro
    // el match es exhaustivo: una variante nueva rompe la compilación y obliga
    // a nombrar su feature, que es justo lo que queremos.
    let feature = match engine {
        StorageEngine::Sled => "storage-sled",
        StorageEngine::Redb => "storage-redb",
    };
    crate::error::StorageError::new(
        crate::error::StorageErrorKind::Unsupported,
        format!(
            "storage engine {engine:?} was not compiled into this build; \
             rebuild with the `{feature}` Cargo feature enabled"
        ),
    )
    .into()
}

#[cfg(test)]
mod engine_availability_tests {
    use super::*;
    use crate::storage::backend::StorageEngine;

    /// El error nombra la feature EXACTA que falta, no un comodín: quien lo
    /// recibe está a una línea de Cargo.toml de arreglarlo.
    #[test]
    fn not_compiled_error_names_the_exact_feature() {
        let msg = format!("{}", engine_not_compiled(StorageEngine::Redb));
        assert!(msg.contains("storage-redb"), "debe nombrar la feature: {msg}");
        assert!(!msg.contains("storage-*"), "sin comodines: {msg}");

        let msg = format!("{}", engine_not_compiled(StorageEngine::Sled));
        assert!(msg.contains("storage-sled"), "debe nombrar la feature: {msg}");
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

    /// Un engine cuyo backend NO se compiló falla con el error accionable en
    /// vez de con un panic. Solo se puede afirmar cuando falta exactamente
    /// uno de los dos.
    #[cfg(all(feature = "storage-sled", not(feature = "storage-redb")))]
    #[test]
    fn missing_backend_is_an_error_not_a_panic() {
        let opts = crate::storage::backend::StorageOptions {
            engine: StorageEngine::Redb,
            profile: crate::storage::backend::StorageProfile::Default,
        };
        let Err(err) = open_in_memory(crate::storage::backend::StorageProfile::Default, &opts)
        else {
            panic!("redb no está compilado en este build: abrir debe fallar");
        };
        let msg = format!("{err}");
        assert!(msg.contains("storage-redb"), "mensaje accionable: {msg}");
    }
}
