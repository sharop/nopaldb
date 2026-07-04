// src/graph/applier.rs
//
// Single-writer apply: TODAS las mutaciones de estructuras derivadas
// (adyacencia persistida, índice de propiedades `idx:prop:`, cadenas de
// versiones de aristas) pasan por un único embudo serializado.
//
// Motivación: cada operación lógica de escritura abarca varias llamadas a
// storage (read-modify-write). Sin serialización, escritores concurrentes
// directos pierden actualizaciones (ver tests/concurrent_writers_test.rs).
//
// Diseño actual: un write-gate (Mutex async) que serializa la fase de
// aplicación física. Es agnóstico al runtime — funciona aunque el caller
// use runtimes efímeros (p. ej. los bindings Python actuales crean un
// runtime por llamada). Cuando exista un runtime compartido en los bindings
// (roadmap: GIL/runtime) este embudo puede evolucionar a una task dedicada
// con canal mpsc para hacer group-commit y batching sin cambiar callers:
// `WriteOp` ya enumera todas las escrituras y `submit_write` es el único
// punto de entrada.

use crate::types::{Edge, Node, NodeId, EdgeId};

/// Operación de escritura física. Único vocabulario que acepta el embudo:
/// cualquier mutación nueva de estructuras derivadas debe agregarse aquí.
#[derive(Debug)]
pub(crate) enum WriteOp {
    /// Alta/actualización del registro current de un nodo + adyacencia + índices.
    AddNode { node: Node, skip_indexing: bool },
    /// Alta de arista con timestamp MVCC (current + versión + adyacencia).
    AddEdgeAt { edge: Edge, timestamp: u64 },
    /// Baja de nodo (limpia índices, aristas incidentes y adyacencia).
    DeleteNode { id: NodeId },
    /// Baja de arista con timestamp MVCC (cierra versión + adyacencia).
    DeleteEdgeAt { id: EdgeId, timestamp: u64 },
    /// Indexación de propiedades de un nodo (listas RMW bajo `idx:prop:`).
    IndexNodeProperties { node: Node },
    /// Alta puntual en el índice de propiedades (usada por NQL UPDATE).
    AddPropertyIndexEntry {
        property: String,
        value: crate::types::PropertyValue,
        node_id: NodeId,
    },
    /// Baja puntual del índice de propiedades (usada por NQL UPDATE).
    RemovePropertyIndexEntry {
        property: String,
        value: crate::types::PropertyValue,
        node_id: NodeId,
    },
}
