//! Migra una base entre motores KV (sled ↔ redb), con verificación.
//!
//! Uso:
//!   cargo run --example migrate_engine --features storage-redb -- <src_dir> <sled|redb> <dst_dir> <sled|redb>
//!
//! El origen debe estar cerrado y con su WAL aplicado (abre y cierra la base
//! con NopalDB normalmente antes de migrar). El destino debe estar vacío.

use nopaldb::{Result, Storage, StorageEngine, StorageOptions};

fn parse_engine(s: &str) -> StorageEngine {
    match s.to_ascii_lowercase().as_str() {
        "sled" => StorageEngine::Sled,
        "redb" => StorageEngine::Redb,
        other => {
            eprintln!("motor desconocido: {other} (usa sled|redb)");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("uso: migrate_engine <src_dir> <sled|redb> <dst_dir> <sled|redb>");
        std::process::exit(1);
    }

    let src_opts = StorageOptions {
        engine: parse_engine(&args[2]),
        ..StorageOptions::default()
    };
    let dst_opts = StorageOptions {
        engine: parse_engine(&args[4]),
        ..StorageOptions::default()
    };

    println!("Migrando {} ({}) → {} ({})…", args[1], args[2], args[3], args[4]);
    let report = Storage::copy_database(&args[1], src_opts, &args[3], dst_opts).await?;

    println!("\n{:<28} {:>12} {:>14}", "keyspace", "pares", "bytes");
    for (name, pairs, bytes) in &report.keyspaces {
        println!("{name:<28} {pairs:>12} {bytes:>14}");
    }
    println!(
        "\nTotal: {} pares, {} bytes. Verificación: {}",
        report.total_pairs(),
        report.total_bytes(),
        if report.verified { "OK ✅" } else { "FALLÓ ❌" }
    );
    Ok(())
}
