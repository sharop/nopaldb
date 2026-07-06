// src/python/runtime.rs
//
// Runtime Tokio compartido por proceso para los bindings Python.
//
// Antes, casi cada método creaba un `tokio::runtime::Runtime` nuevo y hacía
// `block_on` SIN soltar el GIL: cada llamada pagaba la construcción de un
// runtime completo y los hilos Python se serializaban aunque el motor sea
// concurrente. Ahora hay UN runtime multi-thread por proceso (inicialización
// perezosa) y todo `block_on` suelta el GIL mientras el future corre, así
// otros hilos Python avanzan en paralelo.
//
// El runtime es estable durante toda la vida del proceso, lo que además
// permite que el motor mantenga tasks de fondo (auto-GC, y en el futuro el
// applier como task) sin que mueran al final de una llamada.

use pyo3::prelude::*;
use std::future::Future;
use std::sync::OnceLock;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Runtime Tokio del proceso (multi-thread, inicialización perezosa).
pub(crate) fn shared_runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("nopaldb-rt")
            .build()
            .expect("failed to build the NopalDB Tokio runtime")
    })
}

/// Ejecuta un future en el runtime compartido, SOLTANDO el GIL mientras
/// corre: los demás hilos Python siguen ejecutando en paralelo.
pub(crate) fn block_on<F>(py: Python<'_>, fut: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    py.detach(|| shared_runtime().block_on(fut))
}
