// src/easter_eggs.rs
//
// 🌵 NopalDB Easter Eggs
// Hidden features for the curious developer

/// Konami Code easter egg
pub fn konami_code() {
    println!(r#"
╔════════════════════════════════════════════════════════════╗
║          🌵 ¡MODO POWER DE NOPALDB DESBLOQUEADO! 🌵        ║
╠════════════════════════════════════════════════════════════╣
║                                                            ║
║  ¡Felicidades! ¡Encontraste el Código Konami!            ║
║                                                            ║
║  🎮 Logro: Maestro de Grafos                              ║
║  🚀 Bonus: +100 Transacciones/seg (en tu corazón)        ║
║  🌮 Recompensa: Respeto eterno del equipo NopalDB        ║
║                                                            ║
║  Estadísticas de NopalDB:                                 ║
║    • Versión: v{}                                         ║
║    • Tests pasando: ¡Todos! ✅                            ║
║    • Deadlocks evitados: ∞                                ║
║    • Tacos consumidos: También ∞                          ║
║    • Cafés tomados: Incontables ☕                        ║
║                                                            ║
║  No solo estás usando una base de datos...               ║
║  Estás usando ARTE. 🎨                                    ║
║                                                            ║
║  Creada con Rust 🦀, Amor ❤️  y Café ☕                   ║
║  en las hermosas tierras de 🇲🇽                           ║
║                                                            ║
║  "¡Dale que es mole de olla!" - Sharop                   ║
║                                                            ║
╚════════════════════════════════════════════════════════════╝
    "#, env!("CARGO_PKG_VERSION"));
}


/// Random motivational message
pub fn motivational_message() -> &'static str {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();

    let messages = [
        "🌵 ¡Tus grafos están floreciendo hermosamente!",
        "🚀 ¡Performance tan bueno que desafía la física!",
        "💪 Transacciones ACID más fuertes que tu café matutino!",
        "🎮 ¿Listo para construir el próximo gran juego con grafos?",
        "⏰ Time-travel queries: ¡Porque mirar al pasado ayuda!",
        "🔥 ¡Tu código está en llamas! (en el buen sentido)",
        "🌮 ¡Dale que es mole de olla!",
        "🏆 ¡Desarrollador campeón detectado!",
        "🦀 Rust + Grafos = ❤️",
        "📊 ¡Tu estructura de datos está más organizada que Marie Kondo!",
        "🎯 ¡Tiro al blanco! Tus queries son precisos!",
        "🌟 ¡Desarrollador estrella en formación!",
        "💚 NopalDB sabe que eres especial!",
        "🧠 Mentes en movimiento = Grafos en acción!",
        "🎨 Tu código es una obra de arte!",
        "⚡ Velocidad de rayo en cada transacción!",
    ];

    messages.choose(&mut rng).unwrap_or(&messages[0])
}


/// Fun facts about NopalDB
pub fn fun_facts() {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();

    let facts = [
        "🌵 El nopal es resiliente, ¡como esta base de datos!",
        "🌮 El nopal sirve para todo, ¡incluyendo bases de datos!",
        "🇲🇽 NopalDB fue creada con ingenio mexicano!",
        "🦀 Escrita en Rust porque no le tememos a los memory bugs!",
        "⏰ MVCC te permite viajar en el tiempo a través de tus datos!",
        "🔒 La detección de deadlocks mantiene tus transacciones fluyendo!",
        "📝 WAL asegura que nunca pierdas datos, ¡ni en crashes!",
        "🎮 Perfecta para desarrollo de videojuegos (quests RPG, skill trees, etc)!",
        "🚀 Benchmarked a más de 140K operaciones por segundo!",
        "🎨 Calidad de código: Production-ready desde el día uno!",
        "☕ Powered by sesiones nocturnas de código y café!",
        "📚 Inspirada en Datomic, Neo4j y PostgreSQL!",
        "🧠 Creada por Sharop de Lugus - Mentes en Movimiento!",
        "🎯 Zero deadlocks desde que implementamos detección automática!",
        "💾 Cada commit es durable gracias a WAL + fsync!",
        "🔄 MVCC = Multi-Version Concurrency Control (magia de inmutabilidad)!",
        "🏆 Más de 40 tests pasando, cero warnings!",
        "🌵 Como el nopal: puntiaguda pero confiable!",
    ];

    let fact = facts.choose(&mut rng).unwrap_or(&facts[0]);
    println!("\n💡 Dato curioso: {}\n", fact);
}

/// Credits easter egg (bilingüe)
pub fn show_credits() {
    println!(r#"
🌵 ═══════════════════════════════════════════════════════ 🌵

                    CRÉDITOS DE NOPALDB

    Creado por: Sharop & Claude (AI Pair Programming)

    Tecnologías:
      • Rust 🦀 (Porque somos valientes)
      • Sled (Magia de almacenamiento)
      • Tokio (Superpoderes asíncronos)
      • Serde (Hechicería de serialización)

    Inspirado en:
      • Datomic (Sueños de viaje en el tiempo)
      • Neo4j (Excelencia en grafos)
      • PostgreSQL (Confiabilidad ACID)
      • Tu imaginación (Posibilidades infinitas)

    Agradecimientos especiales:
      • Café ☕ (El verdadero MVP)
      • Sesiones nocturnas de código 🌙
      • La comunidad de Rust 🦀
      • ¡A ti, por usar NopalDB! 💚

    Negocio: Lugus - Mentes en Movimiento 🧠
    Instagram: @lugus.mx
    Creador: Sharop (polímata y apasionado por los grafos)

    Intereses del creador:
      • Grafos, ML/DL/RL, NLP 🤖
      • Videojuegos y diseño de juguetes 🎮
      • Caligrafía y plumas fuente ✍️
      • Bajo eléctrico 🎸
      • Investment Banking & Private Equity 📊

    Versión: {}
    Licencia: MIT (¡Úsala, ámala, compártela!)

    "El nopal sirve para todo, incluso para bases de datos" 🌵

🌵 ═══════════════════════════════════════════════════════ 🌵
    "#, env!("CARGO_PKG_VERSION"));
}

/// ASCII Art welcome (español)
pub fn welcome_art() {
    println!(r#"

    ¡Bienvenido a NopalDB! 🌵

         _  _
        ( \/ )
         \  /
          )(
         /  \
        (    )
         \__/
        🌵🌵🌵

    Tu base de datos de grafos con:
    ✓ Transacciones ACID
    ✓ Viajes en el tiempo con MVCC
    ✓ Detección de Deadlocks
    ✓ Durabilidad con WAL
    ✓ Apache Arrow (¡próximamente!)

    Hecha con 🦀 Rust & ❤️ en 🇲🇽

    ¡Escribe graph.konami() para una sorpresa! 🎮

    "#);
}

/// Secret achievement message (español)
pub fn achievement_unlocked(name: &str) {
    println!("\n🏆 ═══════════════════════════════════════ 🏆");
    println!("     ¡LOGRO DESBLOQUEADO!");
    println!("     {}", name);
    println!("🏆 ═══════════════════════════════════════ 🏆\n");
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_motivational_message() {
        let msg = motivational_message();

        // Verificar que el mensaje no está vacío
        assert!(!msg.is_empty(), "Message should not be empty");

        // Verificar que contiene al menos UN emoji (cualquiera)
        let has_emoji = msg.chars().any(|c| {
            // Rango de emojis Unicode (ampliado)
            matches!(c,
                '\u{1F300}'..='\u{1F9FF}' |  // Misc Symbols, Emoticons, Supplemental
                '\u{2300}'..='\u{23FF}' |     // Miscellaneous Technical (⏰ U+23F0, etc.)
                '\u{2600}'..='\u{26FF}' |     // Misc symbols
                '\u{2700}'..='\u{27BF}'       // Dingbats (includes ❤ U+2764)
            )
        });

        assert!(has_emoji, "Message should contain at least one emoji");

        // Verificar longitud razonable
        assert!(msg.len() > 10, "Message should be substantial");
        assert!(msg.len() < 200, "Message should not be too long");
    }
}