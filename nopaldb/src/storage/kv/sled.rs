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

/// Marca con la que sled anuncia que el lock del directorio está tomado.
///
/// Se compara contra el TEXTO porque sled descarta el tipo: envuelve el
/// `WouldBlock` original en un `io::Error` nuevo de kind `Other` y mete el
/// error real dentro del mensaje formateado (`config.rs`, `try_lock`). Mirar
/// `io.kind()` devuelve `Other` y no distingue nada.
///
/// Es frágil —si sled cambia la frase, el reintento deja de activarse en
/// silencio— y por eso existe `retry_engages_on_a_locked_directory`: ese test
/// falla en cuanto la detección deja de funcionar, que es exactamente el modo
/// en que este código puede morir sin avisar. Ya pasó una vez: la primera
/// versión comparaba `io.kind() == WouldBlock`, nunca coincidía, y el
/// reintento estuvo muerto sin que ningún test lo notara.
const MARCA_LOCK_SLED: &str = "could not acquire lock";

/// `true` si el error es "el directorio ya está lockeado", que es lo único
/// que tiene sentido reintentar: cualquier otro fallo de apertura no mejora
/// esperando.
fn es_lock_ocupado(e: &::sled::Error) -> bool {
    matches!(e, ::sled::Error::Io(io) if io.to_string().contains(MARCA_LOCK_SLED))
}

/// Descarta los restos de una creación que nunca terminó.
///
/// Si el proceso muere mientras sled crea la base —el primer arranque de un
/// despliegue nuevo: OOM, kill del contenedor, corte— el directorio queda con
/// un `conf` a medio escribir y sin `db`. A partir de ahí TODA apertura falla
/// con `Read corrupted data`, para siempre: la aplicación no vuelve a
/// arrancar y el mensaje sugiere corrupción de datos cuando en realidad no
/// llegó a existir ninguno.
///
/// La firma es inequívoca porque `db` **es** el archivo de datos de sled: un
/// directorio con `conf` y sin `db` no puede contener nada que perder. Aun
/// así se exige que no haya ningún otro rastro de uso (blobs con contenido,
/// snapshots, el WAL del grafo — que se crea DESPUÉS de abrir el storage, así
/// que su ausencia confirma que nunca se pasó de aquí). Ante cualquier duda no
/// se toca nada y se deja hablar a sled: preferimos un error críptico a borrar
/// datos ajenos.
///
/// La alternativa —crear en un temporal y publicar con un rename, como hace
/// el backend redb— no sirve aquí: la base de sled son varias entradas
/// sueltas dentro del directorio del grafo (`conf`, `db`, `blobs/`), no un
/// archivo único, y mover varias entradas no es atómico.
fn discard_incomplete_creation(path: &std::path::Path) -> Result<()> {
    let conf = path.join("conf");
    let db = path.join("db");

    // O no hay nada que limpiar, o la base existe de verdad.
    if !conf.exists() || db.exists() {
        return Ok(());
    }

    let blobs = path.join("blobs");
    let blobs_vacio = match std::fs::read_dir(&blobs) {
        Ok(mut entradas) => entradas.next().is_none(),
        Err(_) => true, // no existe
    };
    if !blobs_vacio || path.join("nopal.wal").exists() {
        return Ok(());
    }
    // Snapshots de sled: si hay alguno, hubo una base viva en algún momento.
    if let Ok(entradas) = std::fs::read_dir(path) {
        for entrada in entradas.flatten() {
            if entrada.file_name().to_string_lossy().starts_with("snap.") {
                return Ok(());
            }
        }
    }

    log::warn!(
        "descartando una creación de base incompleta en {} (hay `conf` pero no `db`): \
         un arranque anterior murió antes de terminar de crearla y no llegó a \
         guardarse ningún dato",
        path.display()
    );
    std::fs::remove_file(&conf)?;
    if blobs.exists() {
        std::fs::remove_dir_all(&blobs)?;
    }
    Ok(())
}

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
        discard_incomplete_creation(path)?;

        // El lock del directorio se reintenta durante un tiempo acotado.
        //
        // sled lo pide sin bloquear: si el proceso anterior acaba de morir
        // —o su handle acaba de dropearse— el kernel todavía no liberó el
        // flock y la apertura falla con WouldBlock aunque nadie lo tenga ya.
        // Le pasa a cualquiera que cierre y reabra al vuelo (supervisores que
        // reinician, tests de recuperación) y produce fallos que parecen
        // aleatorios. Si de verdad hay otro proceso, el error llega igual,
        // un instante después.
        const ESPERA_LOCK: std::time::Duration = std::time::Duration::from_millis(1500);
        let inicio = std::time::Instant::now();
        loop {
            match Self::config_for(profile).path(path).open() {
                Ok(db) => return Ok(Self { db }),
                Err(e) if es_lock_ocupado(&e) && inicio.elapsed() < ESPERA_LOCK => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) if es_lock_ocupado(&e) => {
                    return Err(StorageError::new(
                        StorageErrorKind::Unsupported,
                        format!(
                            "la base sled en {} ya está abierta por otro proceso \
                             (NopalDB admite un solo escritor por directorio). \
                             Cerrar el otro proceso, o abrir una copia.",
                            path.display()
                        ),
                    )
                    .into());
                }
                Err(e) => return Err(NopalError::from(e)),
            }
        }
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

    fn apply_multi(&self, batches: Vec<(String, WriteBatch)>) -> Result<()> {
        use ::sled::transaction::TransactionError;
        use ::sled::Transactional;

        // Guard OBLIGATORIO, no cortesía: TransactionalTrees::commit indexa
        // inner[0] — un slice de trees vacío haría panic DENTRO de sled.
        if batches.is_empty() {
            return Ok(());
        }

        // Misma regla de resolución que keyspace(): "default" ES el tree
        // default de sled (Db: Deref<Tree>), nunca open_tree("default").
        let trees = batches
            .iter()
            .map(|(name, _)| {
                if name == DEFAULT_KEYSPACE {
                    Ok((*self.db).clone())
                } else {
                    self.db.open_tree(name).map_err(NopalError::from)
                }
            })
            .collect::<Result<Vec<::sled::Tree>>>()?;

        // Transacción multi-tree de sled: un tree repetido en el slice es
        // válido (el lock de stage() es global, no por tree) y commit aplica
        // los overlays en el orden del slice — última escritura gana.
        let result: ::sled::transaction::TransactionResult<(), ()> =
            trees.as_slice().transaction(|txn_trees| {
                for ((_, batch), tree) in batches.iter().zip(txn_trees) {
                    for (key, value) in batch.ops() {
                        match value {
                            Some(v) => {
                                tree.insert(key.as_slice(), v.as_slice())?;
                            }
                            None => {
                                tree.remove(key.as_slice())?;
                            }
                        }
                    }
                }
                Ok(())
            });
        result.map_err(|e| match e {
            TransactionError::Storage(err) => NopalError::from(err),
            // El closure nunca aborta; se mapea por exhaustividad, no panic.
            TransactionError::Abort(()) => StorageError::new(
                StorageErrorKind::Internal,
                "transacción multi-tree de sled abortada sin causa",
            )
            .into(),
        })?;
        Ok(())
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

    /// El reintento del lock SE ACTIVA de verdad.
    ///
    /// Este test existe porque la primera versión de la detección no
    /// funcionaba: comparaba `io.kind() == WouldBlock`, pero sled envuelve
    /// ese error en uno de kind `Other` y deja el original solo en el texto.
    /// La guarda nunca coincidía, el reintento estaba muerto, y como el flake
    /// que debía cubrir es probabilístico, la suite pasó igual y el fallo se
    /// vio hasta CI — con el error CRUDO de sled, que fue la pista.
    ///
    /// Se afirma sobre el error TRADUCIDO y sobre el tiempo: si la detección
    /// deja de funcionar (p. ej. sled cambia la frase), el error vuelve a ser
    /// el crudo y la espera desaparece. Cualquiera de las dos cosas rompe
    /// este test.
    #[test]
    fn retry_engages_on_a_locked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let Ok(_ocupante) = SledEngine::open(dir.path(), StorageProfile::Default) else {
            panic!("la primera apertura debe tomar el lock");
        };

        let inicio = std::time::Instant::now();
        let Err(err) = SledEngine::open(dir.path(), StorageProfile::Default) else {
            panic!("la segunda apertura no puede tomar el lock");
        };
        let esperado = inicio.elapsed();

        let msg = format!("{err}");
        assert!(
            msg.contains("ya está abierta por otro proceso"),
            "el error debe estar traducido, no ser el crudo de sled: {msg}"
        );
        assert!(
            esperado >= std::time::Duration::from_millis(500),
            "debe haber reintentado antes de rendirse; se rindió en {esperado:?}"
        );
    }
}
