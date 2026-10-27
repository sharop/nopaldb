// src/lock_manager.rs

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;
use crate::error::{NopalError, Result};
use crate::types::NodeId;
use crate::transaction::TransactionId;

/// Tipo de lock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    Read,
    Write,
}

/// Información sobre un lock
#[derive(Debug, Clone)]
pub struct LockInfo {
    pub tx_id: TransactionId,
    pub lock_type: LockType,
}

/// Entry de lock con notificación
#[derive(Clone)]
struct LockEntry {
    lock_info: LockInfo,
    notify: Arc<Notify>,
}

/// Wait-for Graph para detección de deadlocks
///
/// Representa dependencias entre transacciones:
/// Edge (tx1 → tx2) significa "tx1 espera que tx2 libere un lock"
#[derive(Debug, Clone)]
pub struct WaitForGraph {
    /// Aristas del grafo: tx_waiting → [tx_holding, ...]
    edges: HashMap<TransactionId, Vec<TransactionId>>,
}

impl WaitForGraph {
    /// Crea un nuevo grafo vacío
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    /// Agrega una arista: `from` espera a `to`
    pub fn add_edge(&mut self, from: TransactionId, to: TransactionId) {
        self.edges
            .entry(from)
            .or_default()
            .push(to);

        log::debug!("WaitForGraph: tx{} waits for tx{}", from, to);
    }

    /// Elimina una arista
    pub fn remove_edge(&mut self, from: TransactionId, to: TransactionId) {
        if let Some(neighbors) = self.edges.get_mut(&from) {
            neighbors.retain(|&tx| tx != to);

            if neighbors.is_empty() {
                self.edges.remove(&from);
            }
        }

        log::debug!("WaitForGraph: removed edge tx{} → tx{}", from, to);
    }

    /// Elimina todas las aristas de una transacción
    pub fn remove_transaction(&mut self, tx_id: TransactionId) {
        self.edges.remove(&tx_id);

        for neighbors in self.edges.values_mut() {
            neighbors.retain(|&tx| tx != tx_id);
        }

        self.edges.retain(|_, neighbors| !neighbors.is_empty());

        log::debug!("WaitForGraph: removed all edges for tx{}", tx_id);
    }

    /// Detecta si hay un ciclo (deadlock) en el grafo usando DFS
    pub fn has_cycle(&self) -> bool {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for &node in self.edges.keys() {
            if !visited.contains(&node)
                && self.has_cycle_dfs(node, &mut visited, &mut rec_stack)
            {
                log::warn!("Deadlock detected in wait-for graph!");
                return true;
            }
        }

        false
    }

    /// DFS recursivo para detectar ciclos
    fn has_cycle_dfs(
        &self,
        node: TransactionId,
        visited: &mut HashSet<TransactionId>,
        rec_stack: &mut HashSet<TransactionId>,
    ) -> bool {
        visited.insert(node);
        rec_stack.insert(node);

        if let Some(neighbors) = self.edges.get(&node) {
            for &neighbor in neighbors {
                if !visited.contains(&neighbor) {
                    if self.has_cycle_dfs(neighbor, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(&neighbor) {
                    log::warn!("Cycle detected: tx{} → tx{}", node, neighbor);
                    return true;
                }
            }
        }

        rec_stack.remove(&node);
        false
    }

    /// Encuentra la "víctima" para abortar (tx más reciente)
    pub fn find_victim(&self) -> Option<TransactionId> {
        let mut all_txs = HashSet::new();

        for (&tx, neighbors) in &self.edges {
            all_txs.insert(tx);
            for &neighbor in neighbors {
                all_txs.insert(neighbor);
            }
        }

        all_txs.into_iter().max()
    }
}

impl Default for WaitForGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WaitForGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "WaitForGraph:")?;

        for (from, tos) in &self.edges {
            for to in tos {
                writeln!(f, "  tx{} → tx{}", from, to)?;
            }
        }

        if self.edges.is_empty() {
            writeln!(f, "  (empty)")?;
        }

        Ok(())
    }
}

/// Lock Manager - gestiona locks y detecta deadlocks
///
/// # Por qué `std::sync::Mutex` y no `tokio::sync::RwLock`
///
/// Una transacción tiene que poder soltar sus candados desde `Drop` y desde
/// `rollback()` síncrono (#296), donde no se puede hacer `.await`. Con el
/// `RwLock` de tokio las salidas eran malas: `blocking_write()` entra en
/// pánico dentro del runtime, `try_write()` falla con contención y el
/// release se perdería, y lanzar una task (`tokio::spawn`) exige un runtime
/// vivo y deja una ventana en la que el nodo sigue tomado — o la task nunca
/// corre si el runtime se está apagando.
///
/// Un `Mutex` síncrono es seguro aquí porque ninguna sección crítica cruza un
/// `.await`: cada una es una consulta/inserción/borrado en un `HashMap`. Así
/// nunca bloquea el runtime más que esas operaciones, y el release es
/// inmediato desde cualquier contexto. Orden de adquisición: `node_locks`
/// antes que `wait_for_graph`, y nunca al revés (no hay ciclo posible).
pub struct LockManager {
    /// Locks actuales: NodeId → LockEntry
    node_locks: Mutex<HashMap<NodeId, LockEntry>>,

    /// Wait-for graph para detección de deadlocks
    wait_for_graph: Mutex<WaitForGraph>,

    /// Timeout para esperar locks (evita esperas infinitas)
    lock_timeout: Duration,
}

impl LockManager {
    /// Crea un nuevo lock manager
    pub fn new() -> Self {
        Self {
            node_locks: Mutex::new(HashMap::new()),
            wait_for_graph: Mutex::new(WaitForGraph::new()),
            lock_timeout: Duration::from_secs(5),
        }
    }

    /// Configura el timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// Toma el mapa de candados. Un pánico ajeno con el guard tomado no debe
    /// impedir soltar candados (eso pasa en `Drop`): se recupera el contenido,
    /// que cada sección crítica deja siempre consistente.
    fn node_locks(&self) -> MutexGuard<'_, HashMap<NodeId, LockEntry>> {
        self.node_locks.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Toma el wait-for graph (misma política de envenenamiento).
    fn wait_for_graph(&self) -> MutexGuard<'_, WaitForGraph> {
        self.wait_for_graph.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Determina si dos locks son compatibles
    ///
    /// Matriz de compatibilidad (2PL):
    /// ```text
    ///           | Read  | Write |
    /// ----------|-------|-------|
    /// Read      | ✅    | ❌    |
    /// Write     | ❌    | ❌    |
    /// ```
    ///
    /// Excepción: Misma transacción → siempre compatible
    fn is_lock_compatible(
        &self,
        requested: LockType,
        existing: LockType,
        requesting_tx: TransactionId,
        holding_tx: TransactionId,
    ) -> bool {
        // Misma tx → siempre compatible (lock upgrade/downgrade)
        if requesting_tx == holding_tx {
            return true;
        }

        // Diferentes tx → consultar matriz de compatibilidad
        match (requested, existing) {
            // Múltiples lectores OK
            (LockType::Read, LockType::Read) => true,

            // No leer mientras se escribe (dirty read)
            (LockType::Read, LockType::Write) => false,

            // No escribir mientras se lee (non-repeatable read)
            (LockType::Write, LockType::Read) => false,

            // Solo un escritor (lost update)
            (LockType::Write, LockType::Write) => false,
        }
    }

    /// Maneja conflicto de lock: agrega al wait-for graph y detecta deadlock
    fn handle_lock_conflict(
        &self,
        waiting_tx: TransactionId,
        holding_tx: TransactionId,
    ) -> Result<()> {
        let mut graph = self.wait_for_graph();
        graph.add_edge(waiting_tx, holding_tx);

        if graph.has_cycle() {
            let victim = graph.find_victim();

            if victim == Some(waiting_tx) {
                // Somos la víctima → abortar
                graph.remove_transaction(waiting_tx);
                return Err(NopalError::Deadlock(format!(
                    "Deadlock detected: tx{} aborted (victim), was waiting for tx{}",
                    waiting_tx, holding_tx
                )));
            } else {
                // Otra tx es víctima → continuar esperando
                log::debug!(
                    "Deadlock detected, but tx{} is not the victim (victim: {:?})",
                    waiting_tx, victim
                );
            }
        }

        Ok(())
    }

    /// Limpia arista del wait-for graph
    fn cleanup_wait_for_edge(&self, from: TransactionId, to: TransactionId) {
        let mut graph = self.wait_for_graph();
        graph.remove_edge(from, to);
    }

    /// Lógica común para adquirir locks
    ///
    /// # Flujo
    /// 1. Loop con timeout global (5s)
    /// 2. Verificar compatibilidad con lock existente
    /// 3. Si incompatible:
    ///    - Detectar deadlock
    ///    - Esperar con timeout (500ms)
    ///    - Reintentar
    /// 4. Si compatible o no existe → Adquirir
    async fn acquire_lock_internal(
        &self,
        node_id: NodeId,
        tx_id: TransactionId,
        lock_type: LockType,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        loop {
            // Timeout global
            if start.elapsed() > self.lock_timeout {
                // Error semántico (no Custom): permite a los callers
                // distinguir contención de locks y reintentar la transacción.
                return Err(NopalError::ConcurrencyError(format!(
                    "Lock timeout: tx{} waiting for {:?} lock on node {} (waited {}s)",
                    tx_id, lock_type, node_id, self.lock_timeout.as_secs()
                )));
            }

            // La sección crítica va en un bloque sin `.await`: el guard del
            // `Mutex` síncrono no puede vivir a través de un punto de espera.
            let (existing_tx_id, existing_lock_type, notify) = {
                let mut locks = self.node_locks();

                // ¿Hay lock existente?
                match locks.get(&node_id) {
                    Some(existing)
                        if !self.is_lock_compatible(
                            lock_type,
                            existing.lock_info.lock_type,
                            tx_id,
                            existing.lock_info.tx_id,
                        ) =>
                    {
                        // CONFLICTO → esperar fuera del guard
                        (
                            existing.lock_info.tx_id,
                            existing.lock_info.lock_type,
                            existing.notify.clone(),
                        )
                    }
                    // CASO ESPECIAL: Read + Read (compartido)
                    Some(existing)
                        if lock_type == LockType::Read
                            && existing.lock_info.lock_type == LockType::Read =>
                    {
                        return Ok(());
                    }
                    _ => {
                        // No hay lock o es compatible → ADQUIRIR
                        locks.insert(
                            node_id,
                            LockEntry {
                                lock_info: LockInfo { tx_id, lock_type },
                                notify: Arc::new(Notify::new()),
                            },
                        );

                        log::debug!("tx{} acquired {:?} lock on node {}", tx_id, lock_type, node_id);
                        return Ok(());
                    }
                }
            };

            log::debug!(
                "tx{} waiting for {:?} lock on node {} (held by tx{} with {:?})",
                tx_id, lock_type, node_id, existing_tx_id, existing_lock_type
            );

            // Deadlock detection
            self.handle_lock_conflict(tx_id, existing_tx_id)?;

            // Esperar con timeout (500ms)
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                notify.notified()
            ).await;

            // Cleanup
            self.cleanup_wait_for_edge(tx_id, existing_tx_id);

            log::debug!("tx{} retrying {:?} lock on node {}", tx_id, lock_type, node_id);
        }
    }

    /// Intenta adquirir un lock de lectura
    pub async fn acquire_read_lock(
        &self,
        node_id: NodeId,
        tx_id: TransactionId,
    ) -> Result<()> {
        self.acquire_lock_internal(node_id, tx_id, LockType::Read).await
    }

    /// Intenta adquirir un lock de escritura
    pub async fn acquire_write_lock(
        &self,
        node_id: NodeId,
        tx_id: TransactionId,
    ) -> Result<()> {
        self.acquire_lock_internal(node_id, tx_id, LockType::Write).await
    }

    /// Libera todos los locks de una transacción.
    ///
    /// Se conserva `async` por compatibilidad de API; delega en
    /// [`release_locks_sync`](Self::release_locks_sync).
    pub async fn release_locks(&self, tx_id: TransactionId) {
        self.release_locks_sync(tx_id);
    }

    /// Libera todos los locks de una transacción sin `.await`: usable desde
    /// `Drop` y desde `rollback()` síncrono (#296).
    ///
    /// Idempotente: liberar dos veces (p. ej. commit y luego `Drop`) no hace
    /// nada la segunda vez, porque solo borra entradas con este `tx_id`.
    pub fn release_locks_sync(&self, tx_id: TransactionId) {
        let mut locks = self.node_locks();

        // Notificar a todos los waiters ANTES de remover
        for (node_id, entry) in locks.iter() {
            if entry.lock_info.tx_id == tx_id {
                log::debug!("tx{} releasing lock on node {}", tx_id, node_id);
                entry.notify.notify_waiters();
            }
        }

        // Remover locks de esta tx
        locks.retain(|_, entry| entry.lock_info.tx_id != tx_id);

        // Limpiar del wait-for graph
        drop(locks);
        self.wait_for_graph().remove_transaction(tx_id);

        log::debug!("Released all locks for tx{}", tx_id);
    }
}

impl Default for LockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_for_graph_no_cycle() {
        let mut graph = WaitForGraph::new();
        graph.add_edge(1, 2);
        graph.add_edge(2, 3);
        assert!(!graph.has_cycle());
    }

    #[test]
    fn test_wait_for_graph_simple_cycle() {
        let mut graph = WaitForGraph::new();
        graph.add_edge(1, 2);
        graph.add_edge(2, 1);
        assert!(graph.has_cycle());
    }

    #[test]
    fn test_wait_for_graph_complex_cycle() {
        let mut graph = WaitForGraph::new();
        graph.add_edge(1, 2);
        graph.add_edge(2, 3);
        graph.add_edge(3, 1);
        assert!(graph.has_cycle());
    }

    #[test]
    fn test_find_victim() {
        let mut graph = WaitForGraph::new();
        graph.add_edge(1, 2);
        graph.add_edge(2, 3);
        graph.add_edge(3, 1);
        assert_eq!(graph.find_victim(), Some(3));
    }

    #[test]
    fn test_remove_transaction() {
        let mut graph = WaitForGraph::new();
        graph.add_edge(1, 2);
        graph.add_edge(2, 3);
        graph.add_edge(3, 1);
        assert!(graph.has_cycle());

        graph.remove_transaction(3);
        assert!(!graph.has_cycle());
    }

    #[test]
    fn test_lock_compatibility_same_tx() {
        let lm = LockManager::new();

        // Misma tx → siempre compatible
        assert!(lm.is_lock_compatible(LockType::Read, LockType::Read, 1, 1));
        assert!(lm.is_lock_compatible(LockType::Read, LockType::Write, 1, 1));
        assert!(lm.is_lock_compatible(LockType::Write, LockType::Read, 1, 1));
        assert!(lm.is_lock_compatible(LockType::Write, LockType::Write, 1, 1));
    }

    /// #296: el release síncrono funciona sin runtime (como en `Drop`), deja
    /// el nodo libre para otra tx y liberar dos veces es inocuo.
    #[test]
    fn test_release_locks_sync_es_idempotente_y_no_requiere_runtime() {
        let lm = LockManager::new().with_timeout(Duration::from_millis(200));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let nodo = NodeId::new_v4();

        rt.block_on(lm.acquire_write_lock(nodo, 1)).unwrap();
        assert!(rt.block_on(lm.acquire_write_lock(nodo, 2)).is_err());

        lm.release_locks_sync(1);
        lm.release_locks_sync(1);
        assert!(lm.wait_for_graph().edges.is_empty());

        rt.block_on(lm.acquire_write_lock(nodo, 2)).unwrap();
        // Liberar una tx sin candados no toca los de otra.
        lm.release_locks_sync(1);
        assert_eq!(lm.node_locks().get(&nodo).map(|e| e.lock_info.tx_id), Some(2));
    }

    #[test]
    fn test_lock_compatibility_different_tx() {
        let lm = LockManager::new();

        // Read + Read → Compatible
        assert!(lm.is_lock_compatible(LockType::Read, LockType::Read, 1, 2));

        // Resto → Incompatible
        assert!(!lm.is_lock_compatible(LockType::Read, LockType::Write, 1, 2));
        assert!(!lm.is_lock_compatible(LockType::Write, LockType::Read, 1, 2));
        assert!(!lm.is_lock_compatible(LockType::Write, LockType::Write, 1, 2));
    }
}