// Fuzz del parser NQL: cualquier input debe producir Ok o Err — nunca panic.
// El parser es puro (pest → AST), sin IO ni estado global.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let _ = nopaldb::parse(data);
    let _ = nopaldb::parse_query(data);
});
