// Suite de conformidad del contrato KV.
//
// UN solo conjunto de invariantes, corrido contra CADA engine compilado.
// Ningún backend entra al dispatch de `open_engine` sin pasarla completa —
// es lo que convierte agregar un motor (F3) de apuesta en trámite.
//
// Vive como unit tests (no en tests/) a propósito: el contrato es
// pub(crate) y no se expone públicamente antes de estabilizar.

use std::sync::Arc;

use super::{KvEngine, KvKeyspace, WriteBatch};
use crate::storage::backend::StorageProfile;

/// Engines efímeros a conformar. Un backend nuevo agrega su línea aquí.
fn engines() -> Vec<(&'static str, Arc<dyn KvEngine>)> {
    let mut v: Vec<(&'static str, Arc<dyn KvEngine>)> = Vec::new();
    #[cfg(feature = "storage-sled")]
    v.push((
        "sled",
        Arc::new(super::sled::SledEngine::open_temporary(StorageProfile::Default).unwrap()),
    ));
    #[cfg(feature = "storage-redb")]
    v.push((
        "redb",
        Arc::new(super::redb::RedbEngine::open_temporary(StorageProfile::Default).unwrap()),
    ));
    v
}

fn ks(engine: &Arc<dyn KvEngine>) -> Arc<dyn KvKeyspace> {
    engine.keyspace("conformance").unwrap()
}

#[test]
fn get_ausente_es_none_y_roundtrip_de_tamanos() {
    for (name, engine) in engines() {
        let ks = ks(&engine);
        assert_eq!(ks.get(b"nope").unwrap(), None, "[{name}]");

        // 0 B, 1 B, 4 KiB, 1 MiB — y claves con 0x00 / 0xFF adentro.
        for (i, size) in [0usize, 1, 4096, 1024 * 1024].into_iter().enumerate() {
            let key = [b"k\x00".as_slice(), &[i as u8], b"\xff"].concat();
            let value = vec![0xAB; size];
            ks.insert(&key, &value).unwrap();
            assert_eq!(ks.get(&key).unwrap().as_deref(), Some(value.as_slice()), "[{name}] size={size}");
            assert!(ks.contains_key(&key).unwrap(), "[{name}]");
        }

        let key = b"gone";
        ks.insert(key, b"x").unwrap();
        ks.remove(key).unwrap();
        assert_eq!(ks.get(key).unwrap(), None, "[{name}] remove");
        assert!(!ks.contains_key(key).unwrap(), "[{name}]");
    }
}

#[test]
fn orden_lexicografico_y_enteros_be_ordenan_numericamente() {
    for (name, engine) in engines() {
        let ks = ks(&engine);
        // Insertados en desorden a propósito.
        for n in [300u64, 5, 1_000_000, 0, 77] {
            ks.insert(&n.to_be_bytes(), b"").unwrap();
        }
        let keys: Vec<u64> = ks
            .iter()
            .map(|item| u64::from_be_bytes(item.unwrap().0.try_into().unwrap()))
            .collect();
        assert_eq!(keys, vec![0, 5, 77, 300, 1_000_000], "[{name}]");
    }
}

#[test]
fn scan_prefix_no_cruza_el_limite_del_prefijo() {
    for (name, engine) in engines() {
        let ks = ks(&engine);
        // "ab" < "ab\xff..." < "ac": el prefijo "ab" NO debe producir "ac",
        // ni el prefijo 0xff desbordar el final del espacio de claves.
        ks.insert(b"ab", b"1").unwrap();
        ks.insert(b"ab\xff\xff", b"2").unwrap();
        ks.insert(b"ac", b"3").unwrap();
        ks.insert(b"\xff\x01", b"4").unwrap();

        let hits: Vec<Vec<u8>> = ks
            .scan_prefix(b"ab")
            .map(|i| i.unwrap().0)
            .collect();
        assert_eq!(hits, vec![b"ab".to_vec(), b"ab\xff\xff".to_vec()], "[{name}]");

        let hits: Vec<Vec<u8>> = ks
            .scan_prefix(b"\xff")
            .map(|i| i.unwrap().0)
            .collect();
        assert_eq!(hits, vec![b"\xff\x01".to_vec()], "[{name}] prefijo 0xff");

        // Prefijo vacío = keyspace completo.
        assert_eq!(ks.scan_prefix(b"").count(), 4, "[{name}]");
    }
}

#[test]
fn range_from_es_inclusivo_y_ordenado() {
    for (name, engine) in engines() {
        let ks = ks(&engine);
        for k in [b"a".as_slice(), b"b", b"c", b"d"] {
            ks.insert(k, b"").unwrap();
        }
        let keys: Vec<Vec<u8>> = ks.range_from(b"b").map(|i| i.unwrap().0).collect();
        assert_eq!(
            keys,
            vec![b"b".to_vec(), b"c".to_vec(), b"d".to_vec()],
            "[{name}] range_from incluye el start; el caller pagina filtrando el cursor"
        );
    }
}

#[test]
fn batch_aplica_todo_y_la_ultima_escritura_gana() {
    for (name, engine) in engines() {
        let ks = ks(&engine);
        ks.insert(b"pre", b"old").unwrap();

        let mut batch = WriteBatch::default();
        batch.insert(b"a".to_vec(), b"1".to_vec());
        batch.insert(b"b".to_vec(), b"2".to_vec());
        batch.remove(b"pre".to_vec());
        // Última escritura gana dentro del batch:
        batch.insert(b"dup".to_vec(), b"first".to_vec());
        batch.remove(b"dup".to_vec());
        batch.insert(b"dup2".to_vec(), b"x".to_vec());
        batch.insert(b"dup2".to_vec(), b"final".to_vec());
        ks.apply_batch(batch).unwrap();

        assert_eq!(ks.get(b"a").unwrap().as_deref(), Some(b"1".as_slice()), "[{name}]");
        assert_eq!(ks.get(b"b").unwrap().as_deref(), Some(b"2".as_slice()), "[{name}]");
        assert_eq!(ks.get(b"pre").unwrap(), None, "[{name}] delete en batch");
        assert_eq!(ks.get(b"dup").unwrap(), None, "[{name}] insert+remove misma clave");
        assert_eq!(ks.get(b"dup2").unwrap().as_deref(), Some(b"final".as_slice()), "[{name}]");
    }
}

#[test]
fn rmw_concurrente_conserva_el_maximo() {
    // 8 hilos empujando máximos monótonos sobre la MISMA clave: el patrón
    // exacto de los relojes lógicos (put_meta_u64_max). Si rmw no es
    // atómico, un máximo se pierde y el reloj retrocede.
    for (name, engine) in engines() {
        let ks = ks(&engine);
        let key = b"clock";
        let mut handles = Vec::new();
        for t in 0..8u64 {
            let ks = Arc::clone(&ks);
            handles.push(std::thread::spawn(move || {
                for i in 0..200u64 {
                    let candidate = t * 1000 + i;
                    ks.rmw(key, &mut |old| {
                        let current = old
                            .and_then(|b| b.try_into().ok().map(u64::from_be_bytes))
                            .unwrap_or(0);
                        Some(current.max(candidate).to_be_bytes().to_vec())
                    })
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let end = u64::from_be_bytes(ks.get(key).unwrap().unwrap().try_into().unwrap());
        assert_eq!(end, 7 * 1000 + 199, "[{name}] se perdió un máximo bajo concurrencia");
    }
}

#[test]
fn keyspaces_independientes_y_clear_local() {
    for (name, engine) in engines() {
        let a = engine.keyspace("ks_a").unwrap();
        let b = engine.keyspace("ks_b").unwrap();
        a.insert(b"k", b"from_a").unwrap();
        b.insert(b"k", b"from_b").unwrap();
        assert_eq!(a.get(b"k").unwrap().as_deref(), Some(b"from_a".as_slice()), "[{name}]");
        assert_eq!(b.get(b"k").unwrap().as_deref(), Some(b"from_b".as_slice()), "[{name}]");

        a.clear().unwrap();
        assert_eq!(a.get(b"k").unwrap(), None, "[{name}] clear vació a");
        assert_eq!(b.get(b"k").unwrap().as_deref(), Some(b"from_b".as_slice()), "[{name}] clear NO tocó b");
    }
}

#[cfg(feature = "storage-sled")]
#[test]
fn persistencia_tras_reabrir_sled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conf_reopen");
    {
        let engine = super::sled::SledEngine::open(&path, StorageProfile::Default).unwrap();
        engine.keyspace("p").unwrap().insert(b"k", b"v").unwrap();
        engine.flush().unwrap();
    }
    let engine = super::sled::SledEngine::open(&path, StorageProfile::Default).unwrap();
    assert_eq!(
        engine.keyspace("p").unwrap().get(b"k").unwrap().as_deref(),
        Some(b"v".as_slice())
    );
}

#[cfg(feature = "storage-redb")]
#[test]
fn persistencia_tras_reabrir_redb() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conf_reopen_redb");
    {
        let engine = super::redb::RedbEngine::open(&path, StorageProfile::Default).unwrap();
        engine.keyspace("p").unwrap().insert(b"k", b"v").unwrap();
        engine.flush().unwrap();
    }
    let engine = super::redb::RedbEngine::open(&path, StorageProfile::Default).unwrap();
    assert_eq!(
        engine.keyspace("p").unwrap().get(b"k").unwrap().as_deref(),
        Some(b"v".as_slice())
    );
}

#[cfg(all(feature = "storage-sled", feature = "storage-redb"))]
#[test]
fn sentinel_de_engine_bloquea_la_apertura_cruzada() {
    // Abrir con el motor equivocado debe fallar con error explícito, no
    // crear una base vacía junto a la existente (ningún lock del OS cubre
    // el cruce: cada motor lockea archivos distintos).
    let dir = tempfile::tempdir().unwrap();

    let sled_path = dir.path().join("base_sled");
    drop(super::sled::SledEngine::open(&sled_path, StorageProfile::Default).unwrap());
    let err = super::redb::RedbEngine::open(&sled_path, StorageProfile::Default)
        .err()
        .expect("redb sobre dir sled debe fallar");
    assert!(err.to_string().contains("base sled"), "{err}");

    let redb_path = dir.path().join("base_redb");
    drop(super::redb::RedbEngine::open(&redb_path, StorageProfile::Default).unwrap());
    let err = super::sled::SledEngine::open(&redb_path, StorageProfile::Default)
        .err()
        .expect("sled sobre dir redb debe fallar");
    assert!(err.to_string().contains("base redb"), "{err}");
}

#[cfg(feature = "storage-sled")]
#[test]
fn los_tres_perfiles_abren() {
    // Regresión del bug 0.4.x: el perfil Server pedía compresión a un sled
    // sin la feature `compression` (inactivable por conflicto de links con
    // el zstd de parquet) y Graph::open FALLABA. El knob ahora se ignora
    // con warning; abrir jamás debe fallar por tuning.
    for profile in [
        StorageProfile::Default,
        StorageProfile::Mobile,
        StorageProfile::Server,
    ] {
        let engine = super::sled::SledEngine::open_temporary(profile)
            .unwrap_or_else(|e| panic!("el perfil {profile:?} no abre: {e}"));
        engine.keyspace("smoke").unwrap().insert(b"k", b"v").unwrap();
    }
}
