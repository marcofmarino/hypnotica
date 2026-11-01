//! Deformación de superficies: la matemática (Rust puro) y el plugin de Bevy que las dibuja.

pub mod deformacion;
pub mod homografia;
mod mascaras;
mod material;
mod patrones;
mod plugin;

pub use plugin::{MostrarGuias, Seleccion, WarpPlugin};
