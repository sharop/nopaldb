// Conversión de errores de sled a los tipos neutrales del crate.
//
// Es la ÚNICA frontera por la que `sled::Error` entra a `NopalError`: la
// variante pública es `StorageError` (kind + mensaje), sin tipos del motor.
// Rutas `::sled` absolutas a propósito: dejan claro que es el crate externo.

use crate::error::{NopalError, StorageError, StorageErrorKind};

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
}
