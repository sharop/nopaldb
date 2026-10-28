// src/storage/clock_marks.rs
//
// Marcas reloj-de-pared → reloj-lógico, para traducir los cortes de GC por
// antigüedad (`GCConfig::older_than_ms`) a la escala de `valid_to`.
//
// Una marca `(unix_ms, ts)` afirma: "todo timestamp lógico < ts se asignó a
// más tardar en el instante de pared `unix_ms`". Quien la registra lee
// PRIMERO el reloj lógico y DESPUÉS el de pared, así que la afirmación es
// cierta por construcción. De ahí sale la traducción conservadora: para un
// instante de pared `c`, el corte lógico es el mayor `ts` entre las marcas
// con `unix_ms <= c`; toda versión con `valid_to` por debajo se cerró antes
// de `c`. Sin marcas aplicables el corte es 0 (no se borra nada).
//
// Cualquier SUBCONJUNTO de marcas válidas sigue siendo válido (solo hace la
// traducción más conservadora), así que perder la última marca en un crash,
// o adelgazar las viejas para acotar el tamaño, nunca borra historia de más.
//
// Se descartó guardar un mapa `ts → unix_ms` por commit: una escritura extra
// por commit para obtener una precisión de milisegundos que un corte de
// "más de N horas" no necesita. Las marcas se registran como mucho una vez
// por `MARK_INTERVAL_MS` desde `Graph::persist_clocks` (una vez por lote del
// applier), en una sola meta de catálogo de tamaño acotado.

/// Intervalo mínimo de pared entre dos marcas registradas.
pub(crate) const MARK_INTERVAL_MS: u64 = 1_000;

/// Máximo de marcas guardadas. Al superarlo se descarta una de cada dos
/// marcas de la mitad más vieja: la resolución decae con la edad (≈ 16 KiB
/// de meta como mucho).
pub(crate) const MAX_MARKS: usize = 1_024;

/// Nombre de la meta (bytes) en `catalog`: pares `(unix_ms, ts)` u64 BE,
/// 16 bytes cada uno, en orden de `unix_ms` creciente.
pub(crate) const META_CLOCK_MARKS: &str = "clock_marks";

pub(crate) fn decode(bytes: &[u8]) -> Vec<(u64, u64)> {
    bytes
        .chunks_exact(16)
        .map(|c| {
            let mut w = [0u8; 8];
            let mut t = [0u8; 8];
            w.copy_from_slice(&c[..8]);
            t.copy_from_slice(&c[8..]);
            (u64::from_be_bytes(w), u64::from_be_bytes(t))
        })
        .collect()
}

pub(crate) fn encode(marks: &[(u64, u64)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(marks.len() * 16);
    for (w, t) in marks {
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&t.to_be_bytes());
    }
    out
}

/// Añade la marca `(unix_ms, ts)` respetando el orden por `unix_ms` y el
/// tope de tamaño. Una marca con reloj de pared anterior a la última (reloj
/// del sistema que retrocede) se descarta: la traducción no la necesita.
pub(crate) fn push(marks: &mut Vec<(u64, u64)>, unix_ms: u64, ts: u64) {
    if let Some(&(last_w, last_t)) = marks.last()
        && (unix_ms < last_w || (unix_ms == last_w && ts <= last_t))
    {
        return;
    }
    marks.push((unix_ms, ts));
    if marks.len() > MAX_MARKS {
        let half = marks.len() / 2;
        let mut i = 0usize;
        marks.retain(|_| {
            let keep = i >= half || i.is_multiple_of(2);
            i += 1;
            keep
        });
    }
}

/// Corte lógico para "cerrado antes del instante de pared `wall_cutoff_ms`":
/// el mayor `ts` de las marcas con `unix_ms <= wall_cutoff_ms`, o 0.
pub(crate) fn logical_cutoff(marks: &[(u64, u64)], wall_cutoff_ms: u64) -> u64 {
    marks
        .iter()
        .filter(|(w, _)| *w <= wall_cutoff_ms)
        .map(|(_, t)| *t)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_lookup() {
        let mut m = Vec::new();
        push(&mut m, 1_000, 5);
        push(&mut m, 2_000, 9);
        push(&mut m, 3_000, 20);
        let m = decode(&encode(&m));
        assert_eq!(m, vec![(1_000, 5), (2_000, 9), (3_000, 20)]);
        assert_eq!(logical_cutoff(&m, 999), 0, "sin marca aplicable no se borra nada");
        assert_eq!(logical_cutoff(&m, 1_000), 5);
        assert_eq!(logical_cutoff(&m, 2_500), 9);
        assert_eq!(logical_cutoff(&m, u64::MAX), 20);
    }

    #[test]
    fn backwards_wall_clock_is_ignored() {
        let mut m = Vec::new();
        push(&mut m, 2_000, 9);
        push(&mut m, 1_500, 12);
        assert_eq!(m, vec![(2_000, 9)]);
    }

    #[test]
    fn thinning_keeps_newest_and_bounds_size() {
        let mut m = Vec::new();
        for i in 0..(MAX_MARKS as u64 * 3) {
            push(&mut m, 1_000 + i * 1_000, 10 + i);
        }
        assert!(m.len() <= MAX_MARKS);
        let last = *m.last().unwrap();
        assert_eq!(last, (1_000 + (MAX_MARKS as u64 * 3 - 1) * 1_000, 10 + MAX_MARKS as u64 * 3 - 1));
        // Sigue ordenado: la traducción es monótona en el corte de pared.
        assert!(m.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
    }
}
