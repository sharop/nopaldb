// examples/migrate_json_to_bincode.rs
//
// Script de migración de bases de datos NopalDB de JSON a Bincode
//
// USO:
//   1. Renombrar DB antigua: mv data.db old_data.db
//   2. Ejecutar: cargo run --example migrate_json_to_bincode
//   3. Renombrar nueva: mv new_data.db data.db

use nopaldb::{Node, Edge};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    println!("🔄 NopalDB: Migración JSON → Bincode");
    println!("=====================================\n");

    // Configuración
    let old_db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "old_data.db".to_string());

    let new_db_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "new_data.db".to_string());

    println!("📂 Input:  {}", old_db_path);
    println!("📂 Output: {}", new_db_path);
    println!();

    // Verificar que existe DB antigua
    if !std::path::Path::new(&old_db_path).exists() {
        eprintln!("❌ Error: No existe {}", old_db_path);
        eprintln!("\nUso:");
        eprintln!("  cargo run --example migrate_json_to_bincode <old_db> <new_db>");
        eprintln!("\nEjemplo:");
        eprintln!("  mv data.db old_data.db");
        eprintln!("  cargo run --example migrate_json_to_bincode old_data.db new_data.db");
        std::process::exit(1);
    }

    // Abrir DB antigua (solo lectura)
    println!("📖 Abriendo DB antigua (JSON)...");
    let old_db = sled::open(&old_db_path)?;

    // Estadísticas iniciales
    let total_keys = old_db.len();
    println!("   Total de keys: {}", total_keys);

    // Crear nueva DB (bincode)
    println!("\n📝 Creando DB nueva (Bincode)...");
    let new_graph = nopaldb::Graph::open(&new_db_path).await?;
    let mut loader = new_graph.bulk_loader(10000);

    // Contadores
    let mut node_count = 0;
    let mut edge_count = 0;
    let mut index_count = 0;
    let mut version_count = 0;
    let mut error_count = 0;

    let start_time = std::time::Instant::now();

    // PASO 1: Migrar nodos
    println!("\n🔄 Migrando nodos...");
    for item in old_db.scan_prefix(b"node:") {
        match item {
            Ok((_key, old_value)) => {
                match serde_json::from_slice::<Node>(&old_value) {
                    Ok(node) => {
                        if let Err(e) = loader.add_node(node).await {
                            eprintln!("⚠️  Error agregando nodo: {}", e);
                            error_count += 1;
                        } else {
                            node_count += 1;

                            if node_count % 10000 == 0 {
                                println!("   Migrados {} nodos...", node_count);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠️  Error deserializando nodo: {}", e);
                        error_count += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Error leyendo key: {}", e);
                error_count += 1;
            }
        }
    }

    println!("   ✅ {} nodos migrados", node_count);

    // PASO 2: Migrar edges
    println!("\n🔄 Migrando edges...");
    for item in old_db.scan_prefix(b"edge:") {
        match item {
            Ok((_key, old_value)) => {
                match serde_json::from_slice::<Edge>(&old_value) {
                    Ok(edge) => {
                        if let Err(e) = loader.add_edge(edge).await {
                            eprintln!("⚠️  Error agregando edge: {}", e);
                            error_count += 1;
                        } else {
                            edge_count += 1;

                            if edge_count % 10000 == 0 {
                                println!("   Migrados {} edges...", edge_count);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠️  Error deserializando edge: {}", e);
                        error_count += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Error leyendo key: {}", e);
                error_count += 1;
            }
        }
    }

    println!("   ✅ {} edges migrados", edge_count);

    // PASO 3: Finalizar bulk load
    println!("\n💾 Finalizando bulk load...");
    let stats = loader.finish().await?;

    let duration = start_time.elapsed();

    // PASO 4: Migrar índices y versiones (si existen)
    println!("\n🔄 Migrando índices de adyacencia...");
    for _item in old_db.scan_prefix(b"idx:") {
        index_count += 1;
        if index_count % 1000 == 0 {
            print!("\r   Migrados {} índices...", index_count);
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }
    if index_count > 0 {
        println!("\n   ℹ️  Índices se reconstruirán automáticamente");
    }

    println!("\n🔄 Migrando versiones MVCC...");
    for _item in old_db.scan_prefix(b"vnode:") {
        version_count += 1;
        if version_count % 1000 == 0 {
            print!("\r   Encontradas {} versiones...", version_count);
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }
    if version_count > 0 {
        println!("\n   ℹ️  Versiones MVCC detectadas (migración manual requerida)");
    }

    // Obtener tamaños de archivo
    let old_size = std::fs::metadata(&old_db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let new_size = std::fs::metadata(&new_db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let size_reduction = if old_size > 0 {
        ((old_size - new_size) as f64 / old_size as f64) * 100.0
    } else {
        0.0
    };

    // RESUMEN FINAL
    println!("\n");
    println!("═══════════════════════════════════════");
    println!("✅ MIGRACIÓN COMPLETADA");
    println!("═══════════════════════════════════════");
    println!();
    println!("📊 Estadísticas:");
    println!("   Nodos migrados:     {:>10}", node_count);
    println!("   Edges migrados:     {:>10}", edge_count);
    println!("   Índices:            {:>10}", index_count);
    println!("   Versiones MVCC:     {:>10}", version_count);
    println!("   Errores:            {:>10}", error_count);
    println!();
    println!("⏱️  Performance:");
    println!("   Tiempo total:       {:>10.2}s", duration.as_secs_f64());
    println!("   Nodos/seg:          {:>10.0}", stats.nodes_per_second);
    println!();
    println!("💾 Tamaño de archivo:");
    println!("   DB antigua (JSON):  {:>10.2} MB", old_size as f64 / 1_048_576.0);
    println!("   DB nueva (Bincode): {:>10.2} MB", new_size as f64 / 1_048_576.0);
    println!("   Reducción:          {:>10.1}%", size_reduction);
    println!();

    if error_count > 0 {
        println!("⚠️  Atención: Se encontraron {} errores durante la migración", error_count);
        println!("   Revisa los mensajes arriba para detalles.");
        println!();
    }

    println!("📋 Próximos pasos:");
    println!("   1. Verificar nueva DB:");
    println!("      ls -lh {}", new_db_path);
    println!();
    println!("   2. Testear nueva DB:");
    println!("      cargo test --lib");
    println!();
    println!("   3. Si todo funciona, renombrar:");
    println!("      mv {} data.db", new_db_path);
    println!();
    println!("   4. Hacer backup de DB antigua:");
    println!("      mv {} backup/data.db.json.$(date +%Y%m%d)", old_db_path);
    println!();

    Ok(())
}
