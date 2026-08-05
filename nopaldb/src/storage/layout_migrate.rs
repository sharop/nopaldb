// src/storage/layout_migrate.rs
//
// F5.5 — Migración automática del layout v1→v2. El paso delicado del ciclo
// F5: MUEVE la fuente de verdad (nodos + historia MVCC + metas) de la sopa
// del tree `default` a los keyspaces v2 (`entities`/`history`/`catalog`) y
// reconstruye los derivados (`adjacency` desde `edges`, `indexes` desde
// `history`).
//
// Orden de fases (invariante 3 del diseño F5):
//
//   A copy      — legacy INTACTO; copia por lotes a los keyspaces v2.
//   B verify    — IDENTIDAD, no solo conteos: digest FNV por namespace del
//                 contenido normalizado (independiente de la representación
//                 de la clave), `c|` apunta a versión existente, relojes no
//                 retroceden. Falla ⇒ error fuerte: la base NO se usa en v2
//                 y el legacy queda intacto (recuperable con un binario v1).
//   C rebuild   — derivados: adyacencia desde `edges` (las claves legacy
//                 `idx:out/in`, huérfanas incluidas, jamás se leen) e índice
//                 ts des-blobeado desde `history`.
//   D activate  — sentinel `layout_format=2` ANTES de limpiar: significa
//                 "layout v2 completo y verificado", no "cero bytes legacy".
//   E cleanup   — borra el legacy del default con predicados EXACTOS
//                 (`keys::is_legacy_layout_key`; jamás el prefijo genérico
//                 `idx:`, que pisaría `idx:prop:` de la sub-migración
//                 prop-idx) y deja la marca de diagnóstico
//                 `meta:layout_migrated_to=2` para binarios viejos.
//
// Máquina de estados en catalog (`layout_migration_state`): copying →
// rebuilding → verified → complete. Reanudable en cualquier punto: cada
// fase es idempotente (copy sobrescribe con los mismos bytes, rebuild
// deriva de la fuente de verdad, cleanup re-escanea) y la transición de
// estado se hace durable (flush) antes de avanzar.

use crate::error::{NopalError, Result, StorageError, StorageErrorKind};
use crate::mvcc::VersionedNode;

use super::keys::{self, v2, LegacyNodeKey};
use super::kv;
use super::kv::migrate::Fnv1a;
use super::{
    deserialize, malformed_key, Storage, EMPTY_VALUE, ENTITIES_TREE, HISTORY_TREE,
    META_NEXT_TIMESTAMP, META_NEXT_TX_ID,
};

/// Versión actual del layout en disco. Ausente = v1 (sopa del tree default);
/// `2` = keyspaces separados con claves binarias (`keys::v2`).
pub(crate) const LAYOUT_FORMAT_CURRENT: u64 = 2;

/// Cota de operaciones por lote en copy/rebuild/cleanup (memoria acotada;
/// las fases son idempotentes, no necesitan ser una sola transacción).
const MIGRATION_BATCH_OPS: usize = 10_000;

/// Fases de la máquina de estados (persistidas como bytes ASCII en
/// `catalog: m|layout_migration_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationState {
    Copying,
    Rebuilding,
    Verified,
    Complete,
}

impl MigrationState {
    fn as_bytes(self) -> &'static [u8] {
        match self {
            MigrationState::Copying => b"copying",
            MigrationState::Rebuilding => b"rebuilding",
            MigrationState::Verified => b"verified",
            MigrationState::Complete => b"complete",
        }
    }

    fn parse(raw: &[u8]) -> Result<Self> {
        match raw {
            b"copying" => Ok(MigrationState::Copying),
            b"rebuilding" => Ok(MigrationState::Rebuilding),
            b"verified" => Ok(MigrationState::Verified),
            b"complete" => Ok(MigrationState::Complete),
            other => Err(layout_error(format!(
                "estado de migración de layout desconocido: {:?}",
                String::from_utf8_lossy(other)
            ))),
        }
    }
}

fn layout_error(msg: String) -> NopalError {
    StorageError::new(StorageErrorKind::Corruption, format!("migración de layout v1→v2: {msg}"))
        .into()
}

/// Digest de identidad por namespace: conteo + suma wrapping de FNV-1a 64
/// por entrada NORMALIZADA (length-prefixed). La suma es conmutativa a
/// propósito — un multiconjunto —: el orden de scan del legacy (string:
/// `v10` < `v2`) y el del v2 (u64 BE: numérico) difieren, así que un digest
/// dependiente del orden compararía peras con manzanas.
#[derive(Default, PartialEq, Eq, Debug, Clone, Copy)]
struct NamespaceDigest {
    count: u64,
    sum: u64,
}

impl NamespaceDigest {
    fn add(&mut self, parts: &[&[u8]]) {
        let mut h = Fnv1a::new();
        for p in parts {
            h.update(&(p.len() as u64).to_be_bytes());
            h.update(p);
        }
        self.count += 1;
        self.sum = self.sum.wrapping_add(h.0);
    }
}

impl Storage {
    /// Migración automática del layout v1→v2 (F5.5). La llama
    /// `Graph::open_with_options` ANTES de cualquier otro lector del open
    /// (ver el comentario del call site para el porqué del orden). No-op
    /// para bases ya migradas; para bases nuevas (default sin claves v1)
    /// solo escribe los sentinels; para bases v1 corre la máquina completa.
    ///
    /// Las bases in-memory y los `Storage` abiertos sin `Graph` no pasan
    /// por aquí — mismo régimen que la sub-migración del índice de
    /// propiedades.
    pub(crate) async fn migrate_layout_if_needed(&self) -> Result<()> {
        // Sentinel presente = layout v2 completo y verificado. Lo único que
        // puede quedar pendiente es la limpieza idempotente del legacy.
        if self
            .get_meta_u64_sync(v2::META_LAYOUT_FORMAT)?
            .unwrap_or(1)
            >= LAYOUT_FORMAT_CURRENT
        {
            if self.get_meta_u64_sync(v2::META_LEGACY_CLEANUP_DONE)?.is_none() {
                log::info!("layout v2 activo con limpieza pendiente — reanudando cleanup del legacy");
                self.layout_cleanup_legacy()?;
            }
            return Ok(());
        }

        let state = self.read_layout_state()?;
        if state.is_none() && !self.default_has_v1_layout_keys()? {
            // Base nueva (o creada 100% v2 por F5.2–F5.4, que aún no escribía
            // el sentinel): activar directo, no hay nada que migrar ni limpiar.
            let mut batch = kv::WriteBatch::default();
            batch.insert(
                v2::catalog_meta_key(v2::META_LAYOUT_FORMAT),
                LAYOUT_FORMAT_CURRENT.to_be_bytes().to_vec(),
            );
            batch.insert(
                v2::catalog_meta_key(v2::META_LEGACY_CLEANUP_DONE),
                1u64.to_be_bytes().to_vec(),
            );
            self.catalog_ks.apply_batch(batch)?;
            return Ok(());
        }

        let state = state.unwrap_or(MigrationState::Copying);
        log::info!("migración de layout v1→v2: iniciando en fase {state:?} (O(datos); no interrumpir a mano — es reanudable, pero el primer open tarda)");

        if state == MigrationState::Copying {
            self.set_layout_state(MigrationState::Copying)?;
            self.layout_copy_legacy()?;
            self.layout_verify_copy()?;
            self.set_layout_state(MigrationState::Rebuilding)?;
        }
        if matches!(state, MigrationState::Copying | MigrationState::Rebuilding) {
            self.layout_rebuild_derived().await?;
            self.set_layout_state(MigrationState::Verified)?;
        }
        // Verified: solo falta activar y limpiar. Complete sin sentinel no
        // debería existir (se escriben en el mismo batch); si aparece, la
        // reactivación es idempotente.
        self.layout_activate().await?;
        self.layout_cleanup_legacy()?;
        log::info!("migración de layout v1→v2 completa");
        Ok(())
    }

    fn read_layout_state(&self) -> Result<Option<MigrationState>> {
        let key = v2::catalog_meta_key(v2::META_LAYOUT_MIGRATION_STATE);
        match self.catalog_ks.get(&key)? {
            Some(raw) => Ok(Some(MigrationState::parse(&raw)?)),
            None => Ok(None),
        }
    }

    /// Persiste la transición de fase y la hace DURABLE: la reanudación
    /// tras crash depende de que el estado nunca vaya por delante de los
    /// datos que describe.
    fn set_layout_state(&self, state: MigrationState) -> Result<()> {
        self.catalog_ks.insert(
            &v2::catalog_meta_key(v2::META_LAYOUT_MIGRATION_STATE),
            state.as_bytes(),
        )?;
        self.engine.flush()
    }

    /// ¿El tree default contiene claves del layout v1? (La marca de
    /// diagnóstico `meta:layout_migrated_to` no cuenta: es post-migración.
    /// `idx:prop:*` tampoco: pertenece a la sub-migración prop-idx.)
    fn default_has_v1_layout_keys(&self) -> Result<bool> {
        for prefix in [
            keys::NODE_PREFIX.as_bytes(),
            keys::TS_PREFIX.as_bytes(),
            keys::ADJ_OUT_PREFIX.as_bytes(),
            keys::ADJ_IN_PREFIX.as_bytes(),
        ] {
            if self.default_ks.scan_prefix(prefix).next().transpose()?.is_some() {
                return Ok(true);
            }
        }
        for item in self.default_ks.scan_prefix(keys::LEGACY_META_PREFIX.as_bytes()) {
            let (key, _) = item?;
            if key != keys::LEGACY_META_LAYOUT_MIGRATED.as_bytes() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ─── Fase A: copy (legacy intacto) ──────────────────────────────────────

    /// Copia la fuente de verdad a los keyspaces v2, por lotes atómicos
    /// (`apply_multi`). El legacy NO se toca. Idempotente: re-copiar
    /// sobrescribe con los mismos bytes.
    ///
    /// NO copia derivados: adyacencia (`idx:out/in`, se reconstruye desde
    /// `edges` — las huérfanas de nodos borrados mueren aquí) ni índice ts
    /// (`ts:{n}` era un blob Vec<NodeId> sin versión; el v2 lo reconstruye
    /// des-blobeado desde `history`).
    fn layout_copy_legacy(&self) -> Result<()> {
        let mut entities = kv::WriteBatch::default();
        let mut history = kv::WriteBatch::default();
        let mut pending = 0usize;
        let mut copied = 0u64;

        for item in self.default_ks.scan_prefix(keys::NODE_PREFIX.as_bytes()) {
            let (key, value) = item?;
            match keys::classify_legacy_node_key(&key) {
                Some(LegacyNodeKey::Base(id)) => entities.insert(v2::node_key_v2(id), value),
                Some(LegacyNodeKey::Version(id, ver)) => {
                    history.insert(v2::history_version_key(id, ver), value)
                }
                Some(LegacyNodeKey::Current(id)) => {
                    history.insert(v2::history_current_key(id), value)
                }
                Some(LegacyNodeKey::Versions(id)) => {
                    history.insert(v2::history_versions_key(id), value)
                }
                // Forma desconocida: mejor no migrar que perder datos en
                // silencio (el legacy queda intacto).
                None => {
                    return Err(layout_error(format!(
                        "clave desconocida en el namespace node: {:?}",
                        String::from_utf8_lossy(&key)
                    )))
                }
            }
            pending += 1;
            copied += 1;
            if pending >= MIGRATION_BATCH_OPS {
                self.engine.apply_multi(vec![
                    (ENTITIES_TREE.to_string(), std::mem::take(&mut entities)),
                    (HISTORY_TREE.to_string(), std::mem::take(&mut history)),
                ])?;
                pending = 0;
                log::info!("migración de layout: {copied} claves node: copiadas…");
            }
        }
        if pending > 0 {
            self.engine.apply_multi(vec![
                (ENTITIES_TREE.to_string(), entities),
                (HISTORY_TREE.to_string(), history),
            ])?;
        }

        // Metas → catalog, con los nombres SIN prefijo. Los dos relojes van
        // con semántica de máximo (bump): jamás retroceden aunque el catalog
        // ya tuviera una cota mayor; el resto se copia byte a byte.
        let mut metas = 0u64;
        for item in self.default_ks.scan_prefix(keys::LEGACY_META_PREFIX.as_bytes()) {
            let (key, value) = item?;
            if key == keys::LEGACY_META_LAYOUT_MIGRATED.as_bytes() {
                continue;
            }
            let name = std::str::from_utf8(&key[keys::LEGACY_META_PREFIX.len()..])
                .map_err(|_| layout_error(format!("nombre meta no-UTF8: {key:02x?}")))?
                .to_string();
            if name == META_NEXT_TIMESTAMP || name == META_NEXT_TX_ID {
                self.bump_clock(&name, Self::decode_meta_u64(&value))?;
            } else {
                self.catalog_ks.insert(&v2::catalog_meta_key(&name), &value)?;
            }
            metas += 1;
        }
        log::info!("migración de layout: copy completo ({copied} claves node:, {metas} metas)");
        Ok(())
    }

    // ─── Fase B: verify (identidad, no solo conteos) ────────────────────────

    /// Verifica que lo copiado es IDÉNTICO al legacy re-leyendo AMBOS lados
    /// de disco. Falla ⇒ error fuerte: el open aborta, el sentinel jamás se
    /// escribe y el legacy queda intacto (la base sigue siendo una base v1
    /// válida para un binario viejo).
    fn layout_verify_copy(&self) -> Result<()> {
        // Lado legacy, normalizado (independiente de la representación de
        // la clave: uuid16 crudo + versión BE, no strings).
        let mut l_base = NamespaceDigest::default();
        let mut l_ver = NamespaceDigest::default();
        let mut l_cur = NamespaceDigest::default();
        let mut l_lst = NamespaceDigest::default();
        for item in self.default_ks.scan_prefix(keys::NODE_PREFIX.as_bytes()) {
            let (key, value) = item?;
            match keys::classify_legacy_node_key(&key) {
                Some(LegacyNodeKey::Base(id)) => l_base.add(&[id.as_bytes(), &value]),
                Some(LegacyNodeKey::Version(id, ver)) => {
                    l_ver.add(&[id.as_bytes(), &ver.to_be_bytes(), &value])
                }
                Some(LegacyNodeKey::Current(id)) => l_cur.add(&[id.as_bytes(), &value]),
                Some(LegacyNodeKey::Versions(id)) => l_lst.add(&[id.as_bytes(), &value]),
                None => {
                    return Err(layout_error(format!(
                        "clave desconocida en el namespace node: {:?}",
                        String::from_utf8_lossy(&key)
                    )))
                }
            }
        }

        // Lado v2, decodificado de las claves binarias.
        let mut n_base = NamespaceDigest::default();
        let mut n_ver = NamespaceDigest::default();
        let mut n_cur = NamespaceDigest::default();
        let mut n_lst = NamespaceDigest::default();
        for item in self.entities_ks.iter() {
            let (key, value) = item?;
            let id = v2::parse_node_key_v2(&key).ok_or_else(|| malformed_key(ENTITIES_TREE, &key))?;
            n_base.add(&[id.as_bytes(), &value]);
        }
        for item in self.history_ks.iter() {
            let (key, value) = item?;
            match key.first() {
                Some(&v2::HISTORY_VERSION_TAG) => {
                    let (id, ver) = v2::parse_history_version_key(&key)
                        .ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
                    n_ver.add(&[id.as_bytes(), &ver.to_be_bytes(), &value]);
                }
                Some(&v2::HISTORY_CURRENT_TAG) => {
                    let id = v2::parse_history_current_key(&key)
                        .ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
                    n_cur.add(&[id.as_bytes(), &value]);
                }
                Some(&v2::HISTORY_VERSIONS_TAG) => {
                    let id = v2::parse_history_versions_key(&key)
                        .ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
                    n_lst.add(&[id.as_bytes(), &value]);
                }
                _ => return Err(malformed_key(HISTORY_TREE, &key)),
            }
        }

        for (what, legacy, nuevo) in [
            ("nodos base (entities)", l_base, n_base),
            ("versiones MVCC (history v|)", l_ver, n_ver),
            ("punteros current (history c|)", l_cur, n_cur),
            ("listas de versiones (history l|)", l_lst, n_lst),
        ] {
            if legacy != nuevo {
                return Err(layout_error(format!(
                    "verificación de identidad FALLÓ en {what}: legacy {} entradas (digest {:016x}) vs v2 {} entradas (digest {:016x}); la base NO es utilizable en v2 — el legacy quedó intacto, recuperable con un binario ≤0.5.2 o con copy_database",
                    legacy.count, legacy.sum, nuevo.count, nuevo.sum
                )));
            }
        }

        // Estructura: todo puntero current apunta a una versión que existe.
        for item in self.history_ks.scan_prefix(&[v2::HISTORY_CURRENT_TAG]) {
            let (key, value) = item?;
            let id = v2::parse_history_current_key(&key)
                .ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
            let ver = u64::from_le_bytes(
                value
                    .as_slice()
                    .try_into()
                    .map_err(|_| layout_error(format!("puntero current de {id} malformado")))?,
            );
            if !self.history_ks.contains_key(&v2::history_version_key(id, ver))? {
                return Err(layout_error(format!(
                    "el puntero current de {id} apunta a la versión {ver}, que no existe"
                )));
            }
        }

        // Metas: los relojes copiados no retroceden; el resto es byte-igual.
        for item in self.default_ks.scan_prefix(keys::LEGACY_META_PREFIX.as_bytes()) {
            let (key, value) = item?;
            if key == keys::LEGACY_META_LAYOUT_MIGRATED.as_bytes() {
                continue;
            }
            let name = std::str::from_utf8(&key[keys::LEGACY_META_PREFIX.len()..])
                .map_err(|_| layout_error(format!("nombre meta no-UTF8: {key:02x?}")))?;
            if name == META_NEXT_TIMESTAMP || name == META_NEXT_TX_ID {
                let catalog = self.get_meta_u64_sync(name)?.unwrap_or(0);
                if catalog < Self::decode_meta_u64(&value) {
                    return Err(layout_error(format!(
                        "el reloj {name} retrocedió en la copia ({catalog} < {})",
                        Self::decode_meta_u64(&value)
                    )));
                }
            } else if self.catalog_ks.get(&v2::catalog_meta_key(name))?.as_deref()
                != Some(value.as_slice())
            {
                return Err(layout_error(format!("la meta {name} no es idéntica en catalog")));
            }
        }

        log::info!(
            "migración de layout: verificación de identidad OK ({} nodos, {} versiones, {} currents, {} listas)",
            n_base.count, n_ver.count, n_cur.count, n_lst.count
        );
        Ok(())
    }

    // ─── Fase C: rebuild de derivados ───────────────────────────────────────

    /// Reconstruye adyacencia (desde `edges`, la fuente de verdad — las
    /// claves legacy `idx:out/in:` jamás se leen, huérfanas incluidas) e
    /// índice ts (desde `history`, des-blobeado con la versión en la clave).
    async fn layout_rebuild_derived(&self) -> Result<()> {
        let (out, inn) = self.rebuild_indices().await?;
        log::info!(
            "migración de layout: adyacencia v2 reconstruida desde edges ({} nodos out, {} nodos in)",
            out.len(),
            inn.len()
        );
        drop((out, inn));

        // `indexes` solo contiene el namespace `t|` (F5.4); clear + rebuild
        // es determinista e idempotente.
        self.indexes_ks.clear()?;
        let mut batch = kv::WriteBatch::default();
        let mut pending = 0usize;
        let mut entries = 0u64;
        for item in self.history_ks.scan_prefix(&[v2::HISTORY_VERSION_TAG]) {
            let (key, value) = item?;
            let (id, ver) = v2::parse_history_version_key(&key)
                .ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
            let versioned: VersionedNode = deserialize(&value)?;
            batch.insert(v2::ts_index_key(versioned.timestamp, id, ver), EMPTY_VALUE);
            pending += 1;
            entries += 1;
            if pending >= MIGRATION_BATCH_OPS {
                self.indexes_ks.apply_batch(std::mem::take(&mut batch))?;
                pending = 0;
                log::info!("migración de layout: {entries} entradas ts reconstruidas…");
            }
        }
        if pending > 0 {
            self.indexes_ks.apply_batch(batch)?;
        }
        log::info!("migración de layout: índice ts reconstruido ({entries} entradas)");
        Ok(())
    }

    // ─── Fase D: activate (sentinel ANTES del cleanup) ──────────────────────

    /// Escribe el sentinel `layout_format=2` + `state=complete` y lo hace
    /// DURABLE antes de que el cleanup toque un solo byte del legacy: si el
    /// sentinel se perdiera con el legacy ya mordido, el próximo open
    /// re-copiaría un legacy incompleto. También deja la cota del reloj al
    /// máximo persistido (el índice ts recién reconstruido + historial de
    /// aristas): el reloj jamás retrocede al reabrir.
    async fn layout_activate(&self) -> Result<()> {
        let max_ts = self.max_persisted_timestamp().await?;
        self.bump_clock(META_NEXT_TIMESTAMP, max_ts.saturating_add(1))?;

        let mut batch = kv::WriteBatch::default();
        batch.insert(
            v2::catalog_meta_key(v2::META_LAYOUT_FORMAT),
            LAYOUT_FORMAT_CURRENT.to_be_bytes().to_vec(),
        );
        batch.insert(
            v2::catalog_meta_key(v2::META_LAYOUT_MIGRATION_STATE),
            MigrationState::Complete.as_bytes().to_vec(),
        );
        self.catalog_ks.apply_batch(batch)?;
        self.engine.flush()?;
        log::info!("migración de layout: layout v2 ACTIVADO (layout_format=2)");
        Ok(())
    }

    // ─── Fase E: cleanup idempotente del legacy ─────────────────────────────

    /// Borra del tree default TODO el layout v1 con predicados exactos
    /// (`keys::is_legacy_layout_key`) y deja la marca de diagnóstico
    /// `meta:layout_migrated_to=2`. Idempotente y reanudable: re-escanea por
    /// chunks hasta que no quede nada que borrar, y solo entonces escribe
    /// `legacy_cleanup_done` en catalog. Si el open encuentra
    /// `layout_format=2` sin esa marca, reanuda aquí.
    fn layout_cleanup_legacy(&self) -> Result<()> {
        // Marca de diagnóstico primero (idempotente): es lo único del
        // default que sobrevive, y debe existir aunque el cleanup se
        // interrumpa a la mitad.
        self.default_ks.insert(
            keys::LEGACY_META_LAYOUT_MIGRATED.as_bytes(),
            &LAYOUT_FORMAT_CURRENT.to_be_bytes(),
        )?;

        let mut removed = 0u64;
        loop {
            let mut batch = kv::WriteBatch::default();
            let mut in_batch = 0usize;
            for item in self.default_ks.iter() {
                let (key, _) = item?;
                if keys::is_legacy_layout_key(&key) {
                    batch.remove(key);
                    in_batch += 1;
                    if in_batch >= MIGRATION_BATCH_OPS {
                        break;
                    }
                }
            }
            if in_batch == 0 {
                break;
            }
            removed += in_batch as u64;
            self.default_ks.apply_batch(batch)?;
            log::info!("migración de layout: {removed} claves legacy purgadas…");
        }

        self.catalog_ks.insert(
            &v2::catalog_meta_key(v2::META_LEGACY_CLEANUP_DONE),
            &1u64.to_be_bytes(),
        )?;
        self.engine.flush()?;
        log::info!("migración de layout: cleanup del legacy completo ({removed} claves)");
        Ok(())
    }
}
