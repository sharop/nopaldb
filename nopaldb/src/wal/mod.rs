// src/wal/mod.rs
//
// Write-Ahead Log (WAL) implementation for durability

use std::path::{Path, PathBuf};
use std::fs::{File, OpenOptions};
use std::io::{Write, Read, Seek, SeekFrom};
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Serialize, Deserialize};

use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, NodeId, EdgeId};
use crate::transaction::TransactionId;

/// WAL Record Types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalRecord {
    /// Begin transaction
    Begin {
        tx_id: TransactionId,
        timestamp: u64,
    },

    /// Insert node
    InsertNode {
        tx_id: TransactionId,
        node: Node,
    },

    /// Update node
    UpdateNode {
        tx_id: TransactionId,
        node_id: NodeId,
        old_node: Node,
        new_node: Node,
    },

    /// Delete node
    DeleteNode {
        tx_id: TransactionId,
        node_id: NodeId,
        node: Node,
    },

    /// Insert edge
    InsertEdge {
        tx_id: TransactionId,
        edge: Edge,
    },

    /// Delete edge
    DeleteEdge {
        tx_id: TransactionId,
        edge_id: EdgeId,
        edge: Edge,
    },

    /// Commit transaction
    Commit {
        tx_id: TransactionId,
        timestamp: u64,
    },

    /// Abort transaction
    Abort {
        tx_id: TransactionId,
    },

    /// Checkpoint marker
    Checkpoint {
        timestamp: u64,
        active_transactions: Vec<TransactionId>,
    },
}

/// Información de recuperación
#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub total_records: usize,
    pub committed_txs: Vec<u64>,
    pub uncommitted_txs: Vec<u64>,
    pub operations_replayed: usize,
    /// Máximo timestamp lógico observado en el WAL (0 si no hay registros).
    /// Se usa al abrir para que los relojes nunca retrocedan por debajo
    /// de lo que ya quedó registrado en el log.
    pub max_timestamp: u64,
    /// Máximo transaction id observado en el WAL (0 si no hay registros).
    pub max_tx_id: u64,
}

/// WAL Manager - handles log writing and recovery
pub struct WalManager {
    /// Path to WAL file
    #[allow(dead_code)]
    wal_path: PathBuf,

    /// WAL file handle
    file: Arc<Mutex<File>>,

    /// Current WAL position
    position: Arc<Mutex<u64>>,

    /// Last checkpoint timestamp
    last_checkpoint: Arc<Mutex<u64>>,
}

impl WalManager {
    /// Create new WAL manager
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        let wal_path = path.as_ref().to_path_buf();

        // Open or create WAL file (append mode)
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&wal_path)?;

        // Get current size
        let file_len = file.metadata()?.len();

        // Un SIGKILL a mitad de append deja una cola rasgada (length prefix o
        // payload incompletos). Ese registro nunca fue confirmado (hay fsync
        // por registro), así que se descarta truncando el archivo al último
        // registro válido; de lo contrario el log queda ilegible y los
        // appends posteriores caerían después de basura.
        let mut file = file;
        let (_, valid_len) = Self::scan_valid_records(&mut file)?;
        if valid_len < file_len {
            log::warn!(
                "WAL has a torn tail ({} bytes past the last valid record) — truncating (crash during append)",
                file_len - valid_len
            );
            file.set_len(valid_len)?;
        }
        let position = valid_len;

        log::info!("WAL opened at {:?}, size: {} bytes", wal_path, position);

        Ok(Self {
            wal_path,
            file: Arc::new(Mutex::new(file)),
            position: Arc::new(Mutex::new(position)),
            last_checkpoint: Arc::new(Mutex::new(position)),
        })
    }

    pub async fn append(&self, record: WalRecord) -> Result<u64> {
        self.append_batch(std::slice::from_ref(&record)).await
    }

    /// Escribe un lote de registros con UN solo fsync al final (group commit
    /// a nivel de transacción). Un commit pequeño pasaba de N+2 fsyncs — uno
    /// por registro Begin/ops/Commit — a exactamente 1: el costo dominante
    /// del commit. Durabilidad intacta: el lote completo está en disco antes
    /// de retornar; si un crash rasga el lote, el Commit no aparece en el log
    /// y el recovery trata la transacción como no confirmada.
    ///
    /// Retorna la posición del primer registro del lote.
    pub async fn append_batch(&self, records: &[WalRecord]) -> Result<u64> {
        if records.is_empty() {
            let position = self.position.lock().await;
            return Ok(*position);
        }

        // Serializar todo el lote a un solo buffer (un write, un fsync)
        let mut buffer: Vec<u8> = Vec::new();
        for record in records {
            let data = serde_json::to_vec(record)
                .map_err(|e| NopalError::SerializationError(e.to_string()))?;
            buffer.extend_from_slice(&(data.len() as u64).to_le_bytes());
            buffer.extend_from_slice(&data);
        }

        let mut file = self.file.lock().await;
        let mut position = self.position.lock().await;

        file.write_all(&buffer)?;
        file.sync_all()?;

        let batch_position = *position;
        *position += buffer.len() as u64;

        log::debug!(
            "WAL append_batch: {} record(s), {} bytes at position {}",
            records.len(),
            buffer.len(),
            batch_position
        );

        Ok(batch_position)
    }

    pub async fn read_all(&self) -> Result<Vec<WalRecord>> {
        let mut file = self.file.lock().await;
        let (records, _) = Self::scan_valid_records(&mut file)?;
        log::info!("Read {} records from WAL", records.len());
        Ok(records)
    }

    /// Escanea el log tolerando una cola rasgada por crash: devuelve los
    /// registros válidos y la longitud en bytes hasta el final del último
    /// registro completo. Un prefijo de longitud incompleto, un payload
    /// truncado o JSON corrupto al final marcan el fin del log válido.
    fn scan_valid_records(file: &mut std::fs::File) -> Result<(Vec<WalRecord>, u64)> {
        let file_len = file.metadata()?.len();
        let mut records = Vec::new();
        let mut valid_len: u64 = 0;

        file.seek(SeekFrom::Start(0))?;

        loop {
            let mut len_bytes = [0u8; 8];
            match file.read_exact(&mut len_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }

            let len = u64::from_le_bytes(len_bytes);

            // Longitud absurda = prefijo rasgado/corrupto: fin del log válido.
            if valid_len + 8 + len > file_len {
                log::warn!("WAL record length ({}) exceeds file — torn tail, stopping scan", len);
                break;
            }

            let mut data = vec![0u8; len as usize];
            match file.read_exact(&mut data) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    log::warn!("WAL payload truncated — torn tail, stopping scan");
                    break;
                }
                Err(e) => return Err(e.into()),
            }

            match serde_json::from_slice::<WalRecord>(&data) {
                Ok(record) => {
                    records.push(record);
                    valid_len += 8 + len;
                }
                Err(e) => {
                    log::warn!("WAL tail record undecodable ({}) — torn tail, stopping scan", e);
                    break;
                }
            }
        }

        Ok((records, valid_len))
    }

    /// Crea un checkpoint en el WAL
    pub async fn checkpoint(&self, active_txs: Vec<TransactionId>) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e|NopalError::Custom(format!("System clock error: {}", e)))?
            .as_millis() as u64;

        // Escribir registro de checkpoint
        self.append(WalRecord::Checkpoint {
            timestamp,
            active_transactions: active_txs.clone(),
        }).await?;

        // Actualizar timestamp del último checkpoint
        let mut last_checkpoint = self.last_checkpoint.lock().await;
        *last_checkpoint = timestamp;

        log::info!(
            "Checkpoint created at t={}, active_txs: {:?}",
            timestamp,
            active_txs
        );

        Ok(())
    }

    /// Trunca el WAL después de un checkpoint exitoso
    pub async fn truncate_after_checkpoint(&self) -> Result<()> {
        // Solo truncar si no hay transacciones activas
        let records = self.read_all().await?;

        // Encontrar último checkpoint
        let mut last_checkpoint_pos = None;
        for (i, record) in records.iter().enumerate() {
            if matches!(record, WalRecord::Checkpoint { .. }) {
                last_checkpoint_pos = Some(i);
            }
        }

        if let Some(checkpoint_pos) = last_checkpoint_pos {
            // Verificar que todas las txs antes del checkpoint están commiteadas
            let mut safe_to_truncate = true;
            let mut active_txs = std::collections::HashSet::new();

            for record in &records[..checkpoint_pos] {
                match record {
                    WalRecord::Begin { tx_id, .. } => {
                        active_txs.insert(*tx_id);
                    }
                    WalRecord::Commit { tx_id, .. } | WalRecord::Abort { tx_id } => {
                        active_txs.remove(tx_id);
                    }
                    _ => {}
                }
            }

            if !active_txs.is_empty() {
                safe_to_truncate = false;
                log::warn!(
                    "Cannot truncate: {} active transactions before checkpoint",
                    active_txs.len()
                );
            }

            if safe_to_truncate {
                // Mantener solo registros después del checkpoint
                let records_to_keep: Vec<_> = records[checkpoint_pos + 1..].to_vec();
                let count = records_to_keep.len();

                // Reescribir WAL
                self.truncate().await?;

                for record in records_to_keep {
                    self.append(record).await?;
                }

                log::info!("WAL truncated, kept {} records", count);
            }
        } else {
            log::debug!("No checkpoint found, skipping truncation");
        }

        Ok(())
    }

    /// Truncate completo (para tests o limpieza)
    pub async fn truncate(&self) -> Result<()> {
        let mut  file = self.file.lock().await;
        let mut position = self.position.lock().await;

        file.set_len(0)?;
        file.sync_all()?;

        // Reabrir en modo append
        file.seek(SeekFrom::Start(0))?;

        *position = 0;

        log::info!("WAL truncated");

        Ok(())
    }


    /// Recupera el estado desde WAL
    pub async fn recover(&self) -> Result<RecoveryInfo> {
        log::info!("Starting WAL recovery...");

        let records = self.read_all().await?;

        if records.is_empty() {
            log::info!("WAL is empty, nothing to recover");
            return Ok(RecoveryInfo {
                total_records: 0,
                committed_txs: Vec::new(),
                uncommitted_txs: Vec::new(),
                operations_replayed: 0,
                max_timestamp: 0,
                max_tx_id: 0,
            });
        }

        // Analizar transacciones
        let mut active_txs = std::collections::HashMap::new();
        let mut committed_txs = std::collections::HashSet::new();
        let mut uncommitted_txs = std::collections::HashSet::new();
        let mut max_timestamp = 0u64;
        let mut max_tx_id = 0u64;

        for record in &records {
            match record {
                WalRecord::Begin { tx_id, timestamp } => {
                    max_tx_id = max_tx_id.max(*tx_id);
                    max_timestamp = max_timestamp.max(*timestamp);
                    active_txs.insert(*tx_id, Vec::new());
                }

                WalRecord::Commit { tx_id, timestamp } => {
                    max_tx_id = max_tx_id.max(*tx_id);
                    max_timestamp = max_timestamp.max(*timestamp);
                    committed_txs.insert(*tx_id);
                    active_txs.remove(tx_id);
                }

                WalRecord::Abort { tx_id } => {
                    max_tx_id = max_tx_id.max(*tx_id);
                    active_txs.remove(tx_id);
                }

                // Agregar operaciones a tx activa
                WalRecord::InsertNode { tx_id, .. }
                | WalRecord::UpdateNode { tx_id, .. }
                | WalRecord::DeleteNode { tx_id, .. }
                | WalRecord::InsertEdge { tx_id, .. }
                | WalRecord::DeleteEdge { tx_id, .. } => {
                    max_tx_id = max_tx_id.max(*tx_id);
                    if let Some(ops) = active_txs.get_mut(tx_id) {
                        ops.push(record.clone());
                    }
                }

                WalRecord::Checkpoint { timestamp, .. } => {
                    // Checkpoint marca punto seguro
                    max_timestamp = max_timestamp.max(*timestamp);
                    log::debug!("Found checkpoint in WAL");
                }
            }
        }

        // Transacciones sin commit = uncommitted
        for tx_id in active_txs.keys() {
            uncommitted_txs.insert(*tx_id);
        }

        log::info!(
            "Recovery analysis: {} total records, {} committed txs, {} uncommitted txs",
            records.len(),
            committed_txs.len(),
            uncommitted_txs.len()
        );

        Ok(RecoveryInfo {
            total_records: records.len(),
            committed_txs: committed_txs.into_iter().collect(),
            uncommitted_txs: uncommitted_txs.into_iter().collect(),
            operations_replayed: 0, // Se llenará durante replay
            max_timestamp,
            max_tx_id,
        })
    }

    /// Obtiene operaciones para replay (solo txs commiteadas)
    pub async fn get_replay_operations(&self) -> Result<Vec<WalRecord>> {
        Ok(self
            .get_replay_operations_with_ts()
            .await?
            .into_iter()
            .map(|(record, _)| record)
            .collect())
    }

    /// Como `get_replay_operations`, pero cada operación viene acompañada del
    /// timestamp lógico del Commit de su transacción — necesario para que el
    /// replay reconstruya cadenas de versiones MVCC con los timestamps
    /// originales del commit, no con relojes nuevos.
    pub async fn get_replay_operations_with_ts(&self) -> Result<Vec<(WalRecord, u64)>> {
        let records = self.read_all().await?;
        let mut committed_txs = std::collections::HashMap::new();
        let mut replay_ops = Vec::new();

        // Primer pase: identificar txs commiteadas y su timestamp de commit
        for record in &records {
            if let WalRecord::Commit { tx_id, timestamp } = record {
                committed_txs.insert(*tx_id, *timestamp);
            }
        }

        // Segundo pase: recolectar operaciones de txs commiteadas
        for record in records {
            match &record {
                WalRecord::InsertNode { tx_id, .. }
                | WalRecord::UpdateNode { tx_id, .. }
                | WalRecord::DeleteNode { tx_id, .. }
                | WalRecord::InsertEdge { tx_id, .. }
                | WalRecord::DeleteEdge { tx_id, .. }
                    if committed_txs.contains_key(tx_id) =>
                {
                    let ts = committed_txs[tx_id];
                    replay_ops.push((record, ts));
                }
                _ => {}
            }
        }

        log::info!("Found {} operations to replay", replay_ops.len());

        Ok(replay_ops)
    }

    /// Flush the Write-Ahead Log to disk
    ///
    /// Ensures all buffered WAL entries are written to disk.
    pub async fn flush(&self) -> Result<()> {
        let mut file = self.file.lock().await;
        file.flush()
            .map_err(|e| NopalError::custom(format!("WAL flush failed: {}", e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    #[tokio::test]
    async fn test_wal_append_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        let wal = WalManager::new(&wal_path).await.unwrap();

        // Append records
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()));

        wal.append(WalRecord::Begin { tx_id: 1, timestamp: 100 }).await.unwrap();
        wal.append(WalRecord::InsertNode { tx_id: 1, node: node.clone() }).await.unwrap();
        wal.append(WalRecord::Commit { tx_id: 1, timestamp: 101 }).await.unwrap();

        // Read back
        let records = wal.read_all().await.unwrap();

        assert_eq!(records.len(), 3);

        match &records[0] {
            WalRecord::Begin { tx_id, .. } => assert_eq!(*tx_id, 1),
            _ => panic!("Expected Begin"),
        }

        match &records[1] {
            WalRecord::InsertNode { node: n, .. } => assert_eq!(n.label, "Person"),
            _ => panic!("Expected InsertNode"),
        }

        match &records[2] {
            WalRecord::Commit { tx_id, .. } => assert_eq!(*tx_id, 1),
            _ => panic!("Expected Commit"),
        }
    }
}