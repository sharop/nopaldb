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
//
// ESCRITURAS DIRECTAS EN EL WAL (0.5.23): cada `WriteOp` de nodo o arista se
// registra como una transacción automática (Begin + registro + Commit) en el
// MISMO lote que los commits, antes de aplicarse. Hasta entonces no dejaban
// rastro en el log: una caída del proceso perdía hasta `flush_every_ms` de
// escrituras directas, y en redb incluso lo que ya estaba en la caché del
// SO (descarta los commits no durables al reabrir). Si el lote solo trae
// escrituras directas y la base está en `DirectWriteDurability::ProcessCrash`
// (default), el lote se escribe sin fsync — el sincronizador periódico y
// `close()` lo hacen durable; con `Immediate`, o si el lote lleva algún
// commit transaccional, se fsynca como siempre.

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
/// aplicación necesita, capturado ANTES de encolar (los nodos y aristas
/// borrados van prefetcheados porque los registros WAL DeleteNode/DeleteEdge
/// llevan la entidad entera).
#[derive(Debug)]
pub(crate) struct CommitSet {
    pub tx_id: TransactionId,
    /// Timestamp de inicio de la transacción (registro WAL Begin).
    pub begin_timestamp: u64,
    pub deleted_nodes: Vec<(NodeId, Node)>,
    pub deleted_edges: Vec<(EdgeId, Edge)>,
    pub pending_nodes: Vec<Node>,
    pub pending_edges: Vec<Edge>,
}

impl CommitSet {
    /// Registros WAL del commit con el timestamp asignado por el applier.
    /// El orden es ESPEJO del apply (`apply_commit_set`): DeleteNode →
    /// DeleteEdge → InsertNode → InsertEdge — así el redo en orden-de-log
    /// reproduce exactamente el apply.
    pub(crate) fn wal_records(&self, commit_timestamp: u64) -> Vec<WalRecord> {
        let mut records = Vec::with_capacity(
            2 + self.deleted_nodes.len()
                + self.deleted_edges.len()
                + self.pending_nodes.len()
                + self.pending_edges.len(),
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
        for (edge_id, edge) in &self.deleted_edges {
            records.push(WalRecord::DeleteEdge {
                tx_id: self.tx_id,
                edge_id: *edge_id,
                edge: edge.clone(),
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

/// Prevalidación del write-set ANTES de escribir sus registros al WAL (H3,
/// #65): un commit cuyo apply va a fallar de forma previsible (arista hacia
/// un endpoint inexistente, o borrado por la propia tx sin re-alta) se
/// rechaza aquí y NUNCA toca el log — `commit()` devuelve Err sin dejar un
/// write-set fsynced que el redo del próximo open pudiera materializar.
///
/// `created_in_batch`/`deleted_in_batch` reflejan lo que commits ANTERIORES
/// del mismo lote (ya validados: van antes en el log y en el apply) habrán
/// creado/borrado cuando este set se aplique.
pub(crate) async fn validate_commit_set(
    graph: &super::Graph,
    set: &CommitSet,
    created_in_batch: &std::collections::HashSet<NodeId>,
    deleted_in_batch: &std::collections::HashSet<NodeId>,
) -> Result<()> {
    if set.pending_edges.is_empty() {
        return Ok(());
    }
    let own_nodes: std::collections::HashSet<NodeId> =
        set.pending_nodes.iter().map(|n| n.id).collect();
    let own_deleted: std::collections::HashSet<NodeId> =
        set.deleted_nodes.iter().map(|(id, _)| *id).collect();

    for edge in &set.pending_edges {
        for endpoint in [edge.source, edge.target] {
            // Un upsert de la propia tx (o de un commit previo del lote)
            // garantiza el endpoint: los upserts se aplican antes que las
            // aristas (`apply_commit_set`).
            if own_nodes.contains(&endpoint) {
                continue;
            }
            let doomed = own_deleted.contains(&endpoint)
                || deleted_in_batch.contains(&endpoint);
            if doomed {
                return Err(crate::error::NopalError::NodeNotFound(endpoint.to_string()));
            }
            if created_in_batch.contains(&endpoint) {
                continue;
            }
            if !graph.storage.node_exists(endpoint).await? {
                return Err(crate::error::NopalError::NodeNotFound(endpoint.to_string()));
            }
        }
    }
    Ok(())
}

/// Plan de la FASE 1 para cada mensaje del lote.
enum Plan {
    /// Operación directa. `logged` = la transacción automática con que se
    /// registró en el WAL (`None` para las ops que no dejan registro: las
    /// entradas puntuales del índice de propiedades, que derivan del nodo y
    /// el redo recalcula, y los borrados de entidades que ya no existen).
    Op { logged: Option<(TransactionId, u64)> },
    /// Commit validado: sus registros van en el grupo con este timestamp.
    Commit { ts: u64 },
    /// Commit rechazado por la prevalidación: sin registros WAL, ack Err.
    Rejected(crate::error::NopalError),
}

/// Registros WAL de una escritura directa como transacción automática, con
/// su `(tx_id, timestamp)`. Los borrados llevan la entidad entera (como en
/// los commits): se lee aquí, bajo el gate, para que el redo no dependa del
/// estado posterior.
async fn direct_op_records(
    graph: &super::Graph,
    op: &WriteOp,
) -> Option<(TransactionId, u64, Vec<WalRecord>)> {
    // Durante el replay del log las escrituras son re-derivaciones de
    // registros que YA están en el WAL: no se vuelven a registrar.
    if graph.replaying.load(std::sync::atomic::Ordering::Acquire) {
        return None;
    }
    let (ts, body) = match op {
        WriteOp::AddNode { node, .. } => (
            graph.next_logical_timestamp(),
            WalRecord::InsertNode { tx_id: 0, node: node.clone() },
        ),
        WriteOp::AddEdgeAt { edge, timestamp } => {
            (*timestamp, WalRecord::InsertEdge { tx_id: 0, edge: edge.clone() })
        }
        WriteOp::DeleteNode { id } => {
            let node = graph.storage.get_node(*id).await.ok()?;
            (
                graph.next_logical_timestamp(),
                WalRecord::DeleteNode { tx_id: 0, node_id: *id, node },
            )
        }
        WriteOp::DeleteEdgeAt { id, timestamp } => {
            let edge = graph.storage.get_edge(*id).await.ok()?;
            (*timestamp, WalRecord::DeleteEdge { tx_id: 0, edge_id: *id, edge })
        }
        WriteOp::AddPropertyIndexEntry { .. } | WriteOp::RemovePropertyIndexEntry { .. } => {
            return None
        }
    };
    let tx_id = graph.next_transaction_id();
    let body = match body {
        WalRecord::InsertNode { node, .. } => WalRecord::InsertNode { tx_id, node },
        WalRecord::InsertEdge { edge, .. } => WalRecord::InsertEdge { tx_id, edge },
        WalRecord::DeleteNode { node_id, node, .. } => WalRecord::DeleteNode { tx_id, node_id, node },
        WalRecord::DeleteEdge { edge_id, edge, .. } => WalRecord::DeleteEdge { tx_id, edge_id, edge },
        other => other,
    };
    Some((
        tx_id,
        ts,
        vec![
            WalRecord::Begin { tx_id, timestamp: ts },
            body,
            WalRecord::Commit { tx_id, timestamp: ts },
        ],
    ))
}

/// Procesa un lote: group-fsync del WAL de todos los commits, luego apply en
/// orden FIFO. Todos los mensajes de un canal pertenecen a la MISMA base
/// (cada Graph::open crea su canal), así que comparten write_gate/WAL/relojes.
///
/// Semántica de fallo (#65): `commit()` con Err = transacción ABORTADA
/// definitivamente. Si el apply falla tras el fsync del grupo, se escribe un
/// registro `Abort` (backstop H3) para que el redo salte ese write-set. Se
/// descartó el esquema two-phase (registro `Intent` + `Committed` tras el
/// apply) porque duplica los fsyncs por commit — rompe el group commit de
/// 1 fsync — y, sin undo físico, tampoco da atomicidad: un crash entre el
/// apply y el `Committed` dejaría estado durable que el log declararía
/// no-commiteado.
async fn process_batch(batch: Vec<ApplierMsg>) {
    let anchor = batch[0].graph.clone();
    let gate = anchor.write_gate();
    let _gate = gate.lock().await;

    // FASE 1 — prevalidación + WAL agrupado: timestamps de commit asignados
    // EN ORDEN de cola (garantiza orden-de-log == orden-de-apply) y UN fsync
    // para el grupo. Los commits rechazados no consumen timestamp ni log.
    let mut plans: Vec<Plan> = Vec::with_capacity(batch.len());
    let mut group_records: Vec<WalRecord> = Vec::new();
    let mut commits_in_group = 0usize;
    let mut created_in_batch: std::collections::HashSet<NodeId> = Default::default();
    let mut deleted_in_batch: std::collections::HashSet<NodeId> = Default::default();

    for msg in &batch {
        match &msg.work {
            Work::Commit(set) => {
                match validate_commit_set(&msg.graph, set, &created_in_batch, &deleted_in_batch)
                    .await
                {
                    Ok(()) => {
                        let ts = msg.graph.next_logical_timestamp();
                        group_records.extend(set.wal_records(ts));
                        for (id, _) in &set.deleted_nodes {
                            created_in_batch.remove(id);
                            deleted_in_batch.insert(*id);
                        }
                        for node in &set.pending_nodes {
                            deleted_in_batch.remove(&node.id);
                            created_in_batch.insert(node.id);
                        }
                        plans.push(Plan::Commit { ts });
                        commits_in_group += 1;
                    }
                    Err(e) => plans.push(Plan::Rejected(e)),
                }
            }
            Work::Op(op) => {
                let logged = match direct_op_records(&msg.graph, op).await {
                    Some((tx_id, ts, records)) => {
                        group_records.extend(records);
                        Some((tx_id, ts))
                    }
                    None => None,
                };
                plans.push(Plan::Op { logged });
            }
        }
    }

    // fsync si el lote lleva algún commit transaccional o la base pide
    // durabilidad inmediata para las escrituras directas.
    let sync = commits_in_group > 0
        || anchor.direct_write_durability == crate::storage::DirectWriteDurability::Immediate;
    let wal_ok = if group_records.is_empty() {
        Ok(())
    } else {
        anchor
            .wal()
            .append_batch_with(&group_records, sync)
            .await
            .map(|_| ())
    };

    if commits_in_group > 1 {
        log::debug!(
            "Group commit: {} transactions, {} WAL records, 1 fsync",
            commits_in_group,
            group_records.len()
        );
    }

    // FASE 2 — apply en orden FIFO; ack de cada mensaje tras SU resultado.
    //
    // Marca de progreso (H4): `settled_upto` avanza sobre el PREFIJO de
    // commits resueltos — aplicados OK o abortados con registro fsynced.
    // Un commit no resuelto (apply Err + Abort no escribible) corta el
    // prefijo: la marca no puede saltarlo o el redo perdería su reintento.
    let mut settled_upto: Option<u64> = None;
    let mut prefix_intact = true;

    for (msg, plan) in batch.into_iter().zip(plans) {
        let result = match msg.work {
            Work::Op(op) => match plan {
                // Sin registro (índice de propiedades, borrado de algo que
                // no existe): se aplica igual que siempre.
                Plan::Op { logged: None } => msg.graph.apply_write_op(op).await,
                Plan::Op { logged: Some((tx_id, ts)) } => match &wal_ok {
                    // Si el log no se pudo escribir, la op no se aplica: un
                    // estado que el WAL no conoce no es recuperable.
                    Err(e) => {
                        prefix_intact = false;
                        Err(crate::error::NopalError::custom(format!(
                            "WAL append failed, direct write not applied: {}",
                            e
                        )))
                    }
                    Ok(()) => match msg.graph.apply_write_op(op).await {
                        Ok(()) => {
                            if prefix_intact {
                                settled_upto = Some(ts);
                            }
                            Ok(())
                        }
                        Err(apply_err) => {
                            // Mismo backstop que los commits: el registro ya
                            // está en el log con su Commit; sin Abort el redo
                            // lo materializaría aunque la op reportó Err.
                            match anchor.wal().append(WalRecord::Abort { tx_id }).await {
                                Ok(_) => {
                                    if prefix_intact {
                                        settled_upto = Some(ts);
                                    }
                                }
                                Err(abort_err) => {
                                    prefix_intact = false;
                                    log::warn!(
                                        "applier: direct write {} failed AND its Abort record could not be written ({}); \
                                         the next open's redo will retry it",
                                        tx_id,
                                        abort_err
                                    );
                                }
                            }
                            Err(apply_err)
                        }
                    },
                },
                _ => unreachable!("plan de commit para un Work::Op"),
            },
            Work::Commit(set) => match plan {
                Plan::Rejected(e) => Err(e),
                Plan::Commit { ts } => match &wal_ok {
                    // Si el fsync del grupo falló, NINGÚN commit es durable:
                    // no se aplica ninguno (y la marca no avanza).
                    Err(e) => {
                        prefix_intact = false;
                        Err(crate::error::NopalError::custom(format!(
                            "group WAL fsync failed, commit aborted before apply: {}",
                            e
                        )))
                    }
                    Ok(()) => match msg.graph.apply_commit_set(&set, ts).await {
                        Ok(()) => {
                            if prefix_intact {
                                settled_upto = Some(ts);
                            }
                            Ok(())
                        }
                        Err(apply_err) => {
                            // Backstop H3: el write-set ya está fsynced con su
                            // Commit; sin este registro el redo del próximo
                            // open materializaría un commit que reportó Err.
                            // Ventana residual: un crash ENTRE el Err del
                            // apply y este fsync deja Commit sin Abort y el
                            // redo aplica el set parcial — idéntico al
                            // comportamiento previo, y alcanzable solo si el
                            // apply falló pese a la prevalidación (IO/bug).
                            match anchor
                                .wal()
                                .append(WalRecord::Abort { tx_id: set.tx_id })
                                .await
                            {
                                Ok(_) => {
                                    if prefix_intact {
                                        settled_upto = Some(ts);
                                    }
                                }
                                Err(abort_err) => {
                                    prefix_intact = false;
                                    log::warn!(
                                        "applier: apply failed for tx {} AND its Abort record could not be written ({}); \
                                         the next open's redo will retry the write-set",
                                        set.tx_id,
                                        abort_err
                                    );
                                }
                            }
                            Err(apply_err)
                        }
                    },
                },
                Plan::Op { .. } => unreachable!("Plan::Op para un Work::Commit"),
            },
        };
        let _ = msg.ack.send(result);
    }

    // Marca de progreso del redo (best effort: si no se persiste, las
    // guardas heurísticas de `replay_wal` siguen cubriendo el sufijo).
    if let Some(upto) = settled_upto
        && let Err(e) = anchor
            .storage
            .put_meta_u64_max(crate::storage::META_WAL_APPLIED_UPTO, upto)
            .await
    {
        log::warn!("applier: failed to advance WAL applied-upto marker: {}", e);
    }

    // Cota de relojes una vez por lote (CAS-max, best effort: el WAL del
    // grupo ya garantiza la recuperación del máximo en un crash).
    if let Err(e) = anchor.persist_clocks().await {
        log::warn!("applier: failed to persist logical clocks after batch: {}", e);
    }
}
