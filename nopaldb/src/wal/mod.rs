// src/wal/mod.rs
//
// Write-Ahead Log (WAL) implementation for durability

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::{NopalError, Result};
use crate::transaction::TransactionId;
use crate::types::{Edge, EdgeId, Node, NodeId};

/// WAL Record Types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalRecord {
    /// Begin transaction
    Begin {
        tx_id: TransactionId,
        timestamp: u64,
    },

    /// Insert node
    InsertNode { tx_id: TransactionId, node: Node },

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
    InsertEdge { tx_id: TransactionId, edge: Edge },

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
    Abort { tx_id: TransactionId },

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
        let position = file.metadata()?.len();

        log::info!("WAL opened at {:?}, size: {} bytes", wal_path, position);

        Ok(Self {
            wal_path,
            file: Arc::new(Mutex::new(file)),
            position: Arc::new(Mutex::new(position)),
            last_checkpoint: Arc::new(Mutex::new(position)),
        })
    }

    pub async fn append(&self, record: WalRecord) -> Result<u64> {
        let mut file = self.file.lock().await;
        let mut position = self.position.lock().await;

        // ✅ Usar JSON en lugar de Bincode
        let data = serde_json::to_vec(&record)
            .map_err(|e| NopalError::SerializationError(e.to_string()))?;

        // Write length prefix
        let len = data.len() as u64;
        let len_bytes = len.to_le_bytes();

        file.write_all(&len_bytes)?;
        file.write_all(&data)?;
        file.sync_all()?;

        let record_position = *position;
        *position += 8 + len;

        log::debug!("WAL append: {:?} at position {}", record, record_position);

        Ok(record_position)
    }

    pub async fn read_all(&self) -> Result<Vec<WalRecord>> {
        let mut file = self.file.lock().await;
        let mut records = Vec::new();

        file.seek(SeekFrom::Start(0))?;

        loop {
            let mut len_bytes = [0u8; 8];
            match file.read_exact(&mut len_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }

            let len = u64::from_le_bytes(len_bytes);

            let mut data = vec![0u8; len as usize];
            file.read_exact(&mut data)?;

            // ✅ Usar JSON en lugar de Bincode
            let record: WalRecord = serde_json::from_slice(&data)
                .map_err(|e| NopalError::SerializationError(e.to_string()))?;

            records.push(record);
        }

        log::info!("Read {} records from WAL", records.len());

        Ok(records)
    }

    /// Crea un checkpoint en el WAL
    pub async fn checkpoint(&self, active_txs: Vec<TransactionId>) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| NopalError::Custom(format!("System clock error: {}", e)))?
            .as_millis() as u64;

        // Escribir registro de checkpoint
        self.append(WalRecord::Checkpoint {
            timestamp,
            active_transactions: active_txs.clone(),
        })
        .await?;

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
        let mut file = self.file.lock().await;
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
            });
        }

        // Analizar transacciones
        let mut active_txs = std::collections::HashMap::new();
        let mut committed_txs = std::collections::HashSet::new();
        let mut uncommitted_txs = std::collections::HashSet::new();

        for record in &records {
            match record {
                WalRecord::Begin { tx_id, .. } => {
                    active_txs.insert(*tx_id, Vec::new());
                }

                WalRecord::Commit { tx_id, .. } => {
                    committed_txs.insert(*tx_id);
                    active_txs.remove(tx_id);
                }

                WalRecord::Abort { tx_id } => {
                    active_txs.remove(tx_id);
                }

                // Agregar operaciones a tx activa
                WalRecord::InsertNode { tx_id, .. }
                | WalRecord::UpdateNode { tx_id, .. }
                | WalRecord::DeleteNode { tx_id, .. }
                | WalRecord::InsertEdge { tx_id, .. }
                | WalRecord::DeleteEdge { tx_id, .. } => {
                    if let Some(ops) = active_txs.get_mut(tx_id) {
                        ops.push(record.clone());
                    }
                }

                WalRecord::Checkpoint { .. } => {
                    // Checkpoint marca punto seguro
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
        })
    }

    /// Obtiene operaciones para replay (solo txs commiteadas)
    pub async fn get_replay_operations(&self) -> Result<Vec<WalRecord>> {
        let records = self.read_all().await?;
        let mut committed_txs = std::collections::HashSet::new();
        let mut replay_ops = Vec::new();

        // Primer pase: identificar txs commiteadas
        for record in &records {
            if let WalRecord::Commit { tx_id, .. } = record {
                committed_txs.insert(*tx_id);
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
                if committed_txs.contains(tx_id) => {
                    replay_ops.push(record);
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
        let node = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

        wal.append(WalRecord::Begin {
            tx_id: 1,
            timestamp: 100,
        })
        .await
        .unwrap();
        wal.append(WalRecord::InsertNode {
            tx_id: 1,
            node: node.clone(),
        })
        .await
        .unwrap();
        wal.append(WalRecord::Commit {
            tx_id: 1,
            timestamp: 101,
        })
        .await
        .unwrap();

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
