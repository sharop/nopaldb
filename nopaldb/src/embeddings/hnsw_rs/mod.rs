//! Copia interna de [`hnsw_rs`](https://crates.io/crates/hnsw_rs) 0.3.4
//! (Jean-Pierre Both, MIT o Apache-2.0: `LICENSE-MIT`, `LICENSE-APACHE`),
//! con tres cambios de comportamiento: el orden de los candados en
//! `Hnsw::reverse_update_neighborhood_simple` (#288), el punto de entrada
//! con inserciones concurrentes (#343) y la capa del enlace inverso (#343).
//!
//! En 0.3.4 (y 0.3.5) la actualización inversa recorre la lista de vecinos
//! del punto nuevo con su read guard tomado y, dentro del bucle, toma el
//! write lock de cada vecino. Dos inserciones simultáneas de `parallel_insert`
//! que quedan como vecinas mutuas se esperan para siempre: A tiene `P.read`
//! y pide `Q.write`, B tiene `Q.read` y pide `P.write`. Aquí la lista se
//! copia primero y el read guard se suelta antes de tomar ningún write lock,
//! así que cada hilo sostiene a lo sumo un candado de vecinos a la vez.
//!
//! En 0.3.4 cada inserción lee el punto de entrada una vez, sin el write
//! lock: con `parallel_insert`, los hilos que lo leían vacío antes de que el
//! primero lo fijara volvían sin enlazar su punto, y el primer punto
//! generado volvía sin enlazar si otro hilo había fijado el punto de entrada
//! antes. Aquí solo el punto que lo encuentra vacío bajo el write lock se
//! vuelve punto de entrada sin vecinos; los demás se enlazan (#343).
//!
//! En 0.3.4 la actualización inversa agrega el punto nuevo a la lista de
//! cada vecino en la capa del nivel máximo del punto nuevo, no en la capa
//! donde son vecinos. Un punto de nivel ≥ 1 está también en las capas de
//! abajo, pero ahí no recibía enlaces inversos: solo le llegaba un enlace
//! en la capa 0 si una inserción posterior lo elegía, y ninguna búsqueda
//! encontraba al ~0.8% de los puntos (los huérfanos de #184/#201; 20k
//! puntos agrupados: ~150 por build, 0 con el arreglo). Aquí el enlace va a
//! la capa recorrida (#343).
//!
//! Se descartó insertar en serie: con 16 núcleos la construcción es ~10×
//! más lenta (50k × 384: 33 s → 318 s). Un fork por git no sirve porque un
//! crate publicado en crates.io no puede depender de él.
//!
//! Lo demás es el código de 0.3.4 sin tocar, para que el formato de los
//! volcados (`hnswio`) sea idéntico. Fuera quedan `libext` (FFI de C),
//! `flatten`, el logger global y los tests propios del crate (los cubren
//! los de `embeddings::index` y `embeddings::persistence`). Cuando upstream
//! publique el arreglo, esta copia se retira y se vuelve a la dependencia.
#![allow(warnings, clippy::all, clippy::pedantic)]

pub mod api;
pub mod datamap;
pub mod filter;
pub mod hnsw;
pub mod hnswio;

pub mod prelude {
    pub use super::api::*;
    pub use super::filter::*;
    pub use super::hnsw::*;
    pub use super::hnswio::*;
    pub use anndists::dist::distances::*;
}
