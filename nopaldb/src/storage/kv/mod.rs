// Capa KV por motor de almacenamiento.
//
// Hoy solo contiene las conversiones de error de cada motor a los tipos
// neutrales de `crate::error` (`StorageError`/`StorageErrorKind`). El
// contrato KV completo (`KvEngine`/`KvKeyspace`) se introduce en la fase 2
// del desacople; este módulo es su casa para que el gating por feature
// quede en un solo lugar.

#[cfg(feature = "storage-sled")]
pub(crate) mod sled;
