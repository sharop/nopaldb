use memmap2::MmapMut;
use nopaldb::{Graph, error::NopalError};
use serde_json;
use std::fs::OpenOptions;
use uuid::Uuid;

/// Ejecuta una consulta NQL masiva y exporta el resultado a un archivo
/// mapeado en memoria (Zero-Copy vía mmap).
///
/// Actualmente serializa a JSON. Cuando el core de NopalDB implemente
/// `QueryResult::to_arrow_ipc()`, esta función migrará a Arrow IPC nativo
/// para lograr el verdadero formato columnar zero-copy.
pub async fn export_to_mmap(graph: &Graph, query: &str) -> Result<String, NopalError> {
    // 1. Ejecutar la consulta en el motor local
    let result = graph.execute_nql(query).await?;

    // 2. Serializar a bytes JSON via el conversor existente en tools.rs
    // TODO: Reemplazar con nativo `.to_arrow_ipc()` cuando el core NopalDB lo soporte.
    let json_val = crate::tools::query_result_to_value(&result);
    let export_bytes = serde_json::to_vec(&json_val)
        .map_err(|e| NopalError::custom(format!("Serialization Error: {}", e)))?;

    if export_bytes.is_empty() {
        return Err(NopalError::custom("La exportación devolvió un bloque vacío."));
    }

    // 3. Ubicación del archivo de memoria temporal (único por petición)
    let path = format!("/tmp/nopaldb_ipc_{}.arrow", Uuid::new_v4());
    
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| NopalError::custom(format!("FS Error: {}", e)))?;

    // 4. Preparar el espacio en el sistema operativo
    file.set_len(export_bytes.len() as u64)
        .map_err(|e| NopalError::custom(format!("FS Size Error: {}", e)))?;

    // 5. Mapear a memoria de forma nativa
    let mut mmap = unsafe { MmapMut::map_mut(&file) }
        .map_err(|e| NopalError::custom(format!("Mmap Error: {}", e)))?;
        
    mmap.copy_from_slice(&export_bytes);
    mmap.flush().map_err(|e| NopalError::custom(format!("Flush Error: {}", e)))?;

    Ok(path)
}
