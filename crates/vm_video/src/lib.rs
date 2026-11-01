//! Video: decodificación con GStreamer (Rust puro) y reproductores que llevan los frames
//! a texturas de Bevy.

pub mod audio;
pub mod captura;
pub mod decodificador;
pub mod dmabuf;
mod importar;
mod plugin;

pub use importar::{RanurasDmaBuf, dmabuf_deshabilitado, formato_de_imagen};
pub use plugin::{Reproductor, VideoPlugin};
