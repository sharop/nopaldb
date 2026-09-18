//! Migra una base entre motores KV (sled ↔ redb), con verificación.
//!
//! Uso:
//!   cargo run --example migrate_engine --features storage-sled -- <src_dir> <auto|sled|redb> <dst_dir> <auto|sled|redb>
//!
//! Misma implementación que el binario `nopaldb migrate` (feature `cli`),
//! que es la forma recomendada para quien instala el crate: ver
//! docs/MIGRATION_0.6.md. `auto` en el origen detecta el motor por los
//! archivos del directorio; en el destino es el motor por defecto del build.
//!
//! El origen debe estar cerrado y con su WAL aplicado (abre y cierra la base
//! con NopalDB normalmente antes de migrar). El destino debe estar vacío.

use nopaldb::migrate::{migrate, parse_engine, render_report};
use nopaldb::{Result, StorageProfile};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("uso: migrate_engine <src_dir> <auto|sled|redb> <dst_dir> <auto|sled|redb>");
        std::process::exit(1);
    }
    let from = parse_engine(&args[2])?;
    let to = parse_engine(&args[4])?;

    println!("Migrando {} ({}) → {} ({})…", args[1], args[2], args[3], args[4]);
    let report = migrate(&args[1], from, &args[3], to, StorageProfile::Default).await?;
    print!("{}", render_report(&report));
    if !report.verified {
        std::process::exit(2);
    }
    Ok(())
}
