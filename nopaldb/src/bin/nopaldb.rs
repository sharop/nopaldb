//! `nopaldb`: herramientas de línea de comandos.
//!
//! ```text
//! nopaldb migrate <src_dir> <dst_dir> [--from auto|sled|redb] [--to auto|sled|redb] [--profile default|mobile|server]
//! nopaldb engine <dir>
//! nopaldb stats <dir>
//! ```
//!
//! Nace para la migración entre motores (#140): quien instala el crate o la
//! wheel no tenía una orden de terminal para pasar una base de sled a redb.
//! `stats` (#158) es la herramienta de "¿qué le espera al próximo open?" sin
//! escribir código. Los argumentos se parsean a mano a propósito: tres
//! subcomandos no justifican una dependencia de CLI. Códigos de salida: 0
//! bien; 1 uso o argumentos inválidos; 2 la operación falló o la
//! verificación no pasó.

use std::sync::Arc;

use nopaldb::migrate::{engine_name, migrate, parse_engine, parse_profile, render_report};
use nopaldb::{Graph, Progress, Storage, StorageEngine, StorageOptions, StorageProfile};

const USAGE: &str = "\
uso:
  nopaldb migrate <src_dir> <dst_dir> [--from auto|sled|redb] [--to auto|sled|redb] [--profile default|mobile|server]
      Copia una base a otro motor, byte a byte, y verifica el destino (conteos y checksums).
      --from auto (default) detecta el motor del origen; --to auto (default) es redb.
      El origen debe estar cerrado (abierto y cerrado al menos una vez con NopalDB); el destino, vacío.
  nopaldb engine <dir>
      Dice qué motor tiene una base: redb, sled, o ninguno.
  nopaldb stats <dir>
      Abre la base en solo lectura y muestra su estado: grafo, WAL y checkpoints, qué reprodujo
      este open y cuánto tardó cada fase, índices de usuario (tamaño, analizador), HNSW y GC.
      La base debe estar cerrada (un solo proceso por directorio). El progreso del replay
      sale por stderr.
  nopaldb --help | --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("migrate") => cmd_migrate(&args[1..]),
        Some("engine") => cmd_engine(&args[1..]),
        Some("stats") => cmd_stats(&args[1..]),
        Some("--version") | Some("-V") => {
            println!("nopaldb {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help") | Some("-h") | None => {
            println!("{USAGE}");
            if args.is_empty() { 1 } else { 0 }
        }
        Some(other) => {
            eprintln!("subcomando desconocido: {other}\n{USAGE}");
            1
        }
    };
    std::process::exit(code);
}

fn cmd_engine(args: &[String]) -> i32 {
    let [dir] = args else {
        eprintln!("{USAGE}");
        return 1;
    };
    match Storage::detect_engine(dir) {
        Some(engine) => {
            println!("{}", engine_name(engine));
            0
        }
        None => {
            println!("ninguno (no hay una base NopalDB en {dir})");
            0
        }
    }
}

fn cmd_migrate(args: &[String]) -> i32 {
    let mut positional: Vec<&str> = Vec::new();
    let mut from = StorageEngine::Auto;
    let mut to = StorageEngine::Auto;
    let mut profile = StorageProfile::Default;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = |i: &mut usize| -> Option<&str> {
            *i += 1;
            args.get(*i).map(String::as_str)
        };
        match a {
            "--from" => match value(&mut i).map(parse_engine) {
                Some(Ok(e)) => from = e,
                Some(Err(e)) => return usage_error(&e.to_string()),
                None => return usage_error("--from necesita un valor"),
            },
            "--to" => match value(&mut i).map(parse_engine) {
                Some(Ok(e)) => to = e,
                Some(Err(e)) => return usage_error(&e.to_string()),
                None => return usage_error("--to necesita un valor"),
            },
            "--profile" => match value(&mut i).map(parse_profile) {
                Some(Ok(p)) => profile = p,
                Some(Err(e)) => return usage_error(&e.to_string()),
                None => return usage_error("--profile necesita un valor"),
            },
            flag if flag.starts_with("--") => return usage_error(&format!("opción desconocida: {flag}")),
            _ => positional.push(a),
        }
        i += 1;
    }
    let [src, dst] = positional[..] else {
        return usage_error("hacen falta exactamente <src_dir> y <dst_dir>");
    };

    let detected = Storage::detect_engine(src);
    println!(
        "Migrando {src} ({}) → {dst} ({})…",
        match (from, detected) {
            (StorageEngine::Auto, Some(d)) => engine_name(d),
            (StorageEngine::Auto, None) => "sin base detectada",
            (e, _) => engine_name(e),
        },
        engine_name(to)
    );

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("no se pudo crear el runtime: {e}");
            return 2;
        }
    };
    match rt.block_on(migrate(src, from, dst, to, profile)) {
        Ok(report) => {
            print!("{}", render_report(&report));
            if report.verified { 0 } else { 2 }
        }
        Err(e) => {
            eprintln!("la migración falló: {e}");
            2
        }
    }
}

fn cmd_stats(args: &[String]) -> i32 {
    let [dir] = args else {
        return usage_error("hace falta exactamente <dir>");
    };
    if Storage::detect_engine(dir).is_none() {
        eprintln!("no hay una base NopalDB en {dir}");
        return 1;
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("no se pudo crear el runtime: {e}");
            return 2;
        }
    };
    // El replay de un WAL grande es justo lo que este comando existe para
    // medir: que se vea avanzar mientras corre.
    let on_progress = Arc::new(|p: Progress| {
        match p.total {
            Some(t) => eprintln!("  {}: {}/{}", p.phase, p.done, t),
            None => eprintln!("  {}: {}", p.phase, p.done),
        }
    });
    let outcome = rt.block_on(async {
        let graph =
            Graph::open_read_only_with_progress(dir, StorageOptions::default(), Some(on_progress)).await?;
        let report = graph.stats().await?;
        graph.close().await?;
        Ok::<_, nopaldb::NopalError>(report)
    });
    match outcome {
        Ok(report) => {
            print!("{}", nopaldb::graph::stats::render(&report));
            0
        }
        Err(e) => {
            eprintln!("no se pudo leer la base: {e}");
            2
        }
    }
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("{msg}\n{USAGE}");
    1
}
