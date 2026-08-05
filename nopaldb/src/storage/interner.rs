// src/storage/interner.rs
//
// Interning de tipos de arista (F5): string ↔ u32, para que la clave de
// adyacencia del layout v2 lleve el tipo en 4 bytes fijos (contrato de 53
// bytes de `keys::v2`) y neighbors-por-tipo sea un prefix scan sin N+1.
//
// Persistencia en el keyspace `catalog` (codec en `keys::v2`):
//   et|{u32 BE}  → nombre (bytes UTF-8 exactos)
//   etn|{nombre} → u32 BE
//   etc          → u32 BE (último id asignado)
//
// Invariante 7 del diseño F5: las TRES claves de una asignación se escriben
// en UN `WriteBatch` (ambos-o-ninguno: jamás un id sin su nombre ni un
// contador atrasado) y el contador nunca queda por debajo del máximo id
// asignado. Los ids JAMÁS se reciclan: sobreviven a cualquier borrado futuro
// de aristas y a los reopen.
//
// Los nombres se guardan con sus bytes UTF-8 EXACTOS, SIN normalización
// Unicode: "café" (U+00E9) y "cafe" + combining acute (U+0065 U+0301) son
// tipos DISTINTOS. Normalizar aquí fusionaría en silencio tipos que el
// usuario creó separados — la igualdad de tipos es la igualdad de `String`
// que el resto del repo ya usa.
//
// Concurrencia: la asignación de ids corre bajo el write_gate del `Graph`
// (todas las escrituras están serializadas), así que nunca hay dos `intern`
// asignando a la vez. El `RwLock` de aquí protege a los LECTORES
// concurrentes (`get`/`resolve` desde reads sin gate) contra la escritura en
// curso — no es quien serializa la asignación.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{NopalError, Result, StorageError, StorageErrorKind};

use super::keys::v2 as keys_v2;
use super::kv::{KvKeyspace, WriteBatch};

fn corrupt(msg: impl Into<String>) -> NopalError {
    StorageError::new(StorageErrorKind::InvalidData, msg.into()).into()
}

fn decode_u32(bytes: &[u8], what: &str) -> Result<u32> {
    let arr: [u8; 4] = bytes
        .try_into()
        .map_err(|_| corrupt(format!("{what}: se esperaban 4 bytes u32 BE, hay {}", bytes.len())))?;
    Ok(u32::from_be_bytes(arr))
}

#[derive(Default)]
struct Maps {
    by_name: HashMap<String, u32>,
    by_id: HashMap<u32, String>,
    /// Último id asignado (0 = ninguno); el próximo es `counter + 1`.
    /// Invariante: `counter >= max(id asignado)` — jamás decrece.
    counter: u32,
}

/// Doble mapa RAM nombre↔id de tipos de arista, respaldado por el keyspace
/// `catalog`. Se carga completo al abrir (`load`) y crece solo vía `intern`.
pub(crate) struct EdgeTypeInterner {
    inner: RwLock<Maps>,
}

impl EdgeTypeInterner {
    /// Carga el interning completo con un scan del prefijo `et` del catalog.
    /// Clasifica por estructura (ver `keys::v2`): `etc` exacto = contador,
    /// `etn…` = nombre→id, 6 bytes = id→nombre (el cap `MAX_EDGE_TYPE_ID`
    /// garantiza que las tres formas son disjuntas byte a byte). Cualquier
    /// divergencia entre los dos mapas o un contador por debajo del máximo
    /// id es corrupción (la escritura es atómica) y falla FUERTE.
    pub(crate) fn load(catalog: &Arc<dyn KvKeyspace>) -> Result<Self> {
        let mut maps = Maps::default();

        for item in catalog.scan_prefix(keys_v2::EDGE_TYPE_ID_PREFIX) {
            let (key, value) = item?;
            if key.as_slice() == keys_v2::EDGE_TYPE_COUNTER_KEY {
                maps.counter = decode_u32(&value, "contador de edge-types (`etc`)")?;
            } else if key.starts_with(keys_v2::EDGE_TYPE_NAME_PREFIX) {
                let name = std::str::from_utf8(&key[keys_v2::EDGE_TYPE_NAME_PREFIX.len()..])
                    .map_err(|_| corrupt("clave `etn` con nombre no-UTF-8 en catalog"))?;
                let id = decode_u32(&value, "valor de clave `etn`")?;
                maps.by_name.insert(name.to_string(), id);
            } else if key.len() == keys_v2::EDGE_TYPE_ID_KEY_LEN {
                let id = u32::from_be_bytes(key[2..6].try_into().expect("slice de 4 bytes"));
                let name = String::from_utf8(value)
                    .map_err(|_| corrupt(format!("clave `et|{id}` con nombre no-UTF-8 en catalog")))?;
                maps.by_id.insert(id, name);
            } else {
                return Err(corrupt(format!(
                    "clave inesperada en el namespace `et` del catalog ({} bytes: {:02x?})",
                    key.len(),
                    key
                )));
            }
        }

        if maps.by_id.len() != maps.by_name.len() {
            return Err(corrupt(format!(
                "interning inconsistente: {} entradas id→nombre vs {} nombre→id",
                maps.by_id.len(),
                maps.by_name.len()
            )));
        }
        for (id, name) in &maps.by_id {
            if maps.by_name.get(name) != Some(id) {
                return Err(corrupt(format!(
                    "interning inconsistente: `et|{id}`→{name:?} sin espejo `etn` idéntico"
                )));
            }
            if *id > maps.counter {
                return Err(corrupt(format!(
                    "contador de edge-types ({}) por debajo del id asignado {id}",
                    maps.counter
                )));
            }
        }

        Ok(Self {
            inner: RwLock::new(maps),
        })
    }

    /// Id de un tipo ya internado, o `None` si no existe. Nunca asigna.
    pub(crate) fn get(&self, name: &str) -> Option<u32> {
        self.lock_read().by_name.get(name).copied()
    }

    /// Nombre de un id internado, o `None` si no existe. (Su lector de
    /// producción — neighbors-por-tipo leyendo claves v2 desde disco — llega
    /// después del rewire F5.3; hoy lo ejercitan los tests.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn resolve(&self, id: u32) -> Option<String> {
        self.lock_read().by_id.get(&id).cloned()
    }

    /// Devuelve el id del tipo, asignando uno nuevo (contador+1) si no
    /// existía. La asignación persiste `et|id` + `etn|nombre` + contador en
    /// UN `WriteBatch` del catalog y solo entonces publica en RAM: un crash
    /// antes del batch no deja rastro; después, el reload ve el trío entero.
    pub(crate) fn intern(&self, catalog: &Arc<dyn KvKeyspace>, name: &str) -> Result<u32> {
        if let Some(id) = self.get(name) {
            return Ok(id);
        }

        let mut maps = self.inner.write().expect("EdgeTypeInterner: RwLock envenenado");
        // Re-chequeo bajo el write lock (el write_gate ya serializa las
        // asignaciones; esto cubre el caso get→intern del MISMO caller).
        if let Some(&id) = maps.by_name.get(name) {
            return Ok(id);
        }

        let id = maps
            .counter
            .checked_add(1)
            .filter(|id| *id <= keys_v2::MAX_EDGE_TYPE_ID)
            .ok_or_else(|| {
                corrupt(format!(
                    "espacio de ids de edge-type agotado (cap {:#x}; ver MAX_EDGE_TYPE_ID)",
                    keys_v2::MAX_EDGE_TYPE_ID
                ))
            })?;

        let mut batch = WriteBatch::default();
        batch.insert(keys_v2::edge_type_id_key(id), name.as_bytes());
        batch.insert(keys_v2::edge_type_name_key(name), id.to_be_bytes());
        batch.insert(keys_v2::EDGE_TYPE_COUNTER_KEY, id.to_be_bytes());
        catalog.apply_batch(batch)?;

        maps.by_name.insert(name.to_string(), id);
        maps.by_id.insert(id, name.to_string());
        maps.counter = id;
        Ok(id)
    }

    fn lock_read(&self) -> std::sync::RwLockReadGuard<'_, Maps> {
        self.inner.read().expect("EdgeTypeInterner: RwLock envenenado")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::backend::StorageProfile;
    use crate::storage::kv::KvEngine;

    fn engine() -> Arc<dyn KvEngine> {
        #[cfg(feature = "storage-sled")]
        return Arc::new(
            crate::storage::kv::sled::SledEngine::open_temporary(StorageProfile::Default).unwrap(),
        );
        #[cfg(all(not(feature = "storage-sled"), feature = "storage-redb"))]
        return Arc::new(
            crate::storage::kv::redb::RedbEngine::open_temporary(StorageProfile::Default).unwrap(),
        );
    }

    fn catalog() -> Arc<dyn KvKeyspace> {
        engine().keyspace("catalog").unwrap()
    }

    #[test]
    fn intern_nuevo_y_existente() {
        let ks = catalog();
        let interner = EdgeTypeInterner::load(&ks).unwrap();

        assert_eq!(interner.get("amigo_de"), None, "vacío al inicio");
        let a = interner.intern(&ks, "amigo_de").unwrap();
        assert_eq!(a, 1, "los ids arrancan en 1");
        assert_eq!(interner.intern(&ks, "amigo_de").unwrap(), a, "existente = mismo id");

        let b = interner.intern(&ks, "colega_de").unwrap();
        assert_eq!(b, 2);
        assert_ne!(a, b);

        // El trío quedó en disco con el codec de keys::v2.
        assert_eq!(
            ks.get(&keys_v2::edge_type_id_key(a)).unwrap().as_deref(),
            Some(b"amigo_de".as_slice())
        );
        assert_eq!(
            ks.get(&keys_v2::edge_type_name_key("colega_de")).unwrap().as_deref(),
            Some(2u32.to_be_bytes().as_slice())
        );
        assert_eq!(
            ks.get(keys_v2::EDGE_TYPE_COUNTER_KEY).unwrap().as_deref(),
            Some(2u32.to_be_bytes().as_slice()),
            "contador = último id asignado"
        );
    }

    #[test]
    fn doble_mapa_consistente() {
        let ks = catalog();
        let interner = EdgeTypeInterner::load(&ks).unwrap();
        let nombres = ["a", "b", "c", "d"];
        for name in nombres {
            let id = interner.intern(&ks, name).unwrap();
            assert_eq!(interner.get(name), Some(id));
            assert_eq!(interner.resolve(id).as_deref(), Some(name));
        }
        assert_eq!(interner.resolve(99), None);
        assert_eq!(interner.get("no_existe"), None);
    }

    #[test]
    fn persistencia_tras_reload_e_ids_no_reciclados() {
        let ks = catalog();
        let ids: Vec<u32> = {
            let interner = EdgeTypeInterner::load(&ks).unwrap();
            ["uno", "dos", "tres"]
                .iter()
                .map(|n| interner.intern(&ks, n).unwrap())
                .collect()
        };
        assert_eq!(ids, vec![1, 2, 3]);

        // Reload del MISMO keyspace: mapas y contador intactos.
        let reloaded = EdgeTypeInterner::load(&ks).unwrap();
        assert_eq!(reloaded.get("uno"), Some(1));
        assert_eq!(reloaded.get("tres"), Some(3));
        assert_eq!(reloaded.resolve(2).as_deref(), Some("dos"));
        assert_eq!(reloaded.intern(&ks, "dos").unwrap(), 2, "existente tras reload");
        assert_eq!(reloaded.intern(&ks, "cuatro").unwrap(), 4, "continúa del contador");

        // Ids jamás reciclados: aunque un tipo desaparezca del catalog (aquí
        // borrado a mano, par completo), el contador manda y su id no se
        // reasigna a un tipo nuevo.
        ks.remove(&keys_v2::edge_type_id_key(1)).unwrap();
        ks.remove(&keys_v2::edge_type_name_key("uno")).unwrap();
        let again = EdgeTypeInterner::load(&ks).unwrap();
        assert_eq!(again.get("uno"), None);
        assert_eq!(again.intern(&ks, "cinco").unwrap(), 5, "el 1 no se recicla");
    }

    #[test]
    fn nombres_no_ascii_intactos_y_sin_normalizacion() {
        let ks = catalog();
        let interner = EdgeTypeInterner::load(&ks).unwrap();

        let exotico = "amigo-de-日本語-ñ-🌵";
        let id = interner.intern(&ks, exotico).unwrap();
        assert_eq!(interner.resolve(id).as_deref(), Some(exotico));
        assert_eq!(
            ks.get(&keys_v2::edge_type_name_key(exotico)).unwrap().as_deref(),
            Some(id.to_be_bytes().as_slice()),
            "clave etn con los bytes UTF-8 exactos"
        );

        // NFC vs NFD: mismos glifos, bytes distintos → tipos DISTINTOS.
        let nfc = "caf\u{e9}";
        let nfd = "cafe\u{301}";
        let id_nfc = interner.intern(&ks, nfc).unwrap();
        let id_nfd = interner.intern(&ks, nfd).unwrap();
        assert_ne!(id_nfc, id_nfd, "normalizar fusionaría tipos distintos");

        // Y sobreviven el reload byte a byte.
        let reloaded = EdgeTypeInterner::load(&ks).unwrap();
        assert_eq!(reloaded.resolve(id).as_deref(), Some(exotico));
        assert_eq!(reloaded.get(nfc), Some(id_nfc));
        assert_eq!(reloaded.get(nfd), Some(id_nfd));
    }

    #[test]
    fn load_rechaza_interning_inconsistente() {
        let ks = catalog();
        {
            let interner = EdgeTypeInterner::load(&ks).unwrap();
            interner.intern(&ks, "sano").unwrap();
        }
        // Se rompe el espejo a mano: id sin su etn (jamás pasa con el batch
        // atómico; simula corrupción externa).
        ks.remove(&keys_v2::edge_type_name_key("sano")).unwrap();
        assert!(EdgeTypeInterner::load(&ks).is_err(), "divergencia = corrupción, falla fuerte");
    }
}
