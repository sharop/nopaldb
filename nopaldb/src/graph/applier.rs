// src/graph/applier.rs
//
// Single-writer apply: TODAS las mutaciones de estructuras derivadas
// (adyacencia persistida, índice de propiedades `idx:prop:`, cadenas de
// versiones) pasan por un único embudo serializado.
//
// Diseño: una TASK dedicada por base de datos, spawneada al abrir el Graph,
// que recibe trabajo por un canal mpsc. Cada mensaje lleva su propio clone
// de `Graph`, así la task no retiene estado entre mensajes: cuando el usuario
// suelta el último handle, los senders caen, el canal se cierra y la task
// termina sola (sin ciclos de liveness).
//
// La task toma el `write_gate` por lote — el gate sigue siendo LA exclusión
// mutua, de modo que los paths que lo usan directo (GC, flush_indices, batch
// loaders y el fallback inline) conservan su corrección sin cambios.
//
// GROUP COMMIT: al drenar la cola, los registros WAL de TODOS los commits
// encolados se escriben con UN solo fsync, y después cada write-set se aplica
// en orden FIFO (orden-de-log == orden-de-apply, la invariante de la que
// depende el redo del crash recovery). Con N committers concurrentes, el
// costo de fsync se amortiza entre todos.
//
// Fallback: si la task murió (p. ej. el runtime que abrió el Graph fue
// destruido y se usa desde otro), `submit_*` aplica inline bajo el gate con
// la misma semántica — un fsync por commit, sin agrupamiento.
//
// Cancelación: si un caller abandona el await del ack cuando su mensaje ya
// fue encolado, la operación se aplica de todas formas (es atómica y válida);
// el ack se descarta. Es la misma clase de semántica "committed but
// unacknowledged" documentada en docs/DURABILITY.md.

use crate::error::Result;
use crate::transaction::TransactionId;
use crate::types::{Edge, Node, NodeId, EdgeId};
use crate::wal::WalRecord;

/// Máximo de mensajes por ciclo de drenado (acota la latencia del primero).
const MAX_BATCH: usize = 16;
/// Capacidad del canal: backpressure para escritores muy por delante del disco.
const CHANNEL_CAPACITY: usize = 256;

/// Operación de escritura física directa. Único vocabulario que acepta el
/// embudo: cualquier mutación nueva de estructuras derivadas debe ir aquí.
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

/// Write-set completo de un commit transaccional: todo lo que la fase de
/// aplicación necesita, capturado ANTES de encolar (los nodos borrados van
/// prefetcheados porque el registro WAL DeleteNode lleva el nodo entero).
#[derive(Debug)]
pub(crate) struct CommitSet {
    pub tx_id: TransactionId,
    /// Timestamp de inicio de la transacción (registro WAL Begin).
    pub begin_timestamp: u64,
    pub deleted_nodes: Vec<(NodeId, Node)>,
    pub deleted_edges: Vec<EdgeId>,
    pub pending_nodes: Vec<Node>,
    pub pending_edges: Vec<Edge>,
}

impl CommitSet {
    /// Registros WAL del commit con el timestamp asignado por el applier.
    pub(crate) fn wal_records(&self, commit_timestamp: u64) -> Vec<WalRecord> {
        let mut records = Vec::with_capacity(
            2 + self.deleted_nodes.len() + self.pending_nodes.len() + self.pending_edges.len(),
        );
        records.push(WalRecord::Begin {
            tx_id: self.tx_id,
            timestamp: self.begin_timestamp,
        });
        for (node_id, node) in &self.deleted_nodes {
            records.push(WalRecord::DeleteNode {
                tx_id: self.tx_id,
                node_id: *node_id,
                node: node.clone(),
            });
        }
        for node in &self.pending_nodes {
            records.push(WalRecord::InsertNode {
                tx_id: self.tx_id,
                node: node.clone(),
            });
        }
        for edge in &self.pending_edges {
            records.push(WalRecord::InsertEdge {
                tx_id: self.tx_id,
                edge: edge.clone(),
            });
        }
        records.push(WalRecord::Commit {
            tx_id: self.tx_id,
            timestamp: commit_timestamp,
        });
        records
    }
}

/// Trabajo que viaja por el canal.
pub(crate) enum Work {
    Op(WriteOp),
    Commit(CommitSet),
}

pub(crate) struct ApplierMsg {
    pub graph: super::Graph,
    pub work: Work,
    pub ack: tokio::sync::oneshot::Sender<Result<()>>,
}

/// Spawnea la task del applier y retorna el extremo de envío.
/// Requiere contexto de runtime Tokio (Graph::open es async; los bindings
/// Python usan el runtime compartido del proceso).
pub(crate) fn spawn_applier() -> tokio::sync::mpsc::Sender<ApplierMsg> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ApplierMsg>(CHANNEL_CAPACITY);

    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Drenar lo encolado preservando orden FIFO
            let mut batch = vec![first];
            while batch.len() < MAX_BATCH {
                match rx.try_recv() {
                    Ok(msg) => batch.push(msg),
                    Err(_) => break,
                }
            }
            process_batch(batch).await;
        }
        log::debug!("Write applier task exited (all graph handles dropped)");
    });

    tx
}

/// Procesa un lote: group-fsync del WAL de todos los commits, luego apply en
/// orden FIFO. Todos los mensajes de un canal pertenecen a la MISMA base
/// (cada Graph::open crea su canal), así que comparten write_gate/WAL/relojes.
async fn process_batch(batch: Vec<ApplierMsg>) {
    let anchor = batch[0].graph.clone();
    let gate = anchor.write_gate();
    let _gate = gate.lock().await;

    // FASE 1 — WAL agrupado: timestamps de commit asignados EN ORDEN de cola
    // (garantiza orden-de-log == orden-de-apply) y UN fsync para el grupo.
    let mut commit_timestamps: Vec<Option<u64>> = Vec::with_capacity(batch.len());
    let mut group_records: Vec<WalRecord> = Vec::new();
    let mut commits_in_group = 0usize;

    for msg in &batch {
        match &msg.work {
            Work::Commit(set) => {
                let ts = msg.graph.next_logical_timestamp();
                group_records.extend(set.wal_records(ts));
                commit_timestamps.push(Some(ts));
                commits_in_group += 1;
            }
            Work::Op(_) => commit_timestamps.push(None),
        }
    }

    let wal_ok = if group_records.is_empty() {
        Ok(())
    } else {
        anchor.wal().append_batch(&group_records).await.map(|_| ())
    };

    if commits_in_group > 1 {
        log::debug!(
            "Group commit: {} transactions, {} WAL records, 1 fsync",
            commits_in_group,
            group_records.len()
        );
    }

    // FASE 2 — apply en orden FIFO; ack de cada mensaje tras SU resultado.
    for (msg, commit_ts) in batch.into_iter().zip(commit_timestamps) {
        let result = match (&wal_ok, msg.work) {
            // Si el fsync del grupo falló, NINGÚN commit es durable: no se
            // aplica ninguno. Las ops directas no dependen del WAL y se
            // aplican de todas formas (misma semántica que hoy).
            (Err(e), Work::Commit(_)) => Err(crate::error::NopalError::custom(format!(
                "group WAL fsync failed, commit aborted before apply: {}",
                e
            ))),
            (_, Work::Op(op)) => msg.graph.apply_write_op(op).await,
            (Ok(()), Work::Commit(set)) => {
                msg.graph
                    .apply_commit_set(&set, commit_ts.expect("commit has timestamp"))
                    .await
            }
        };
        let _ = msg.ack.send(result);
    }

    // Cota de relojes una vez por lote (CAS-max, best effort: el WAL del
    // grupo ya garantiza la recuperación del máximo en un crash).
    if let Err(e) = anchor.persist_clocks().await {
        log::warn!("applier: failed to persist logical clocks after batch: {}", e);
    }
}
