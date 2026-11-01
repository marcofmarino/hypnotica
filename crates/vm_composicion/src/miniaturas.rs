//! Miniaturas de los clips: un cuadro de cada video, para mostrar en la grilla.
//!
//! Sacar una miniatura implica abrir el video con GStreamer, así que se hace **en hilos
//! aparte** (nunca en el principal) y de a pocos: si se pidieran todas a la vez con un
//! proyecto grande, el disco y la CPU se saturarían. Cada frame, `gestionar_miniaturas`
//! pide las que faltan y recoge las que terminaron.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use vm_core::resolver_ruta;
use vm_proyecto::{ProyectoActual, RutaProyecto};
use vm_video::decodificador::{Frame, miniatura};

/// Ancho de las miniaturas, en píxeles. Alcanza para una celda de la grilla.
const ANCHO_MINIATURA: u32 = 240;

/// Cuántas miniaturas se sacan a la vez.
const MAXIMO_A_LA_VEZ: usize = 2;

/// En qué estado está la miniatura de un archivo.
pub enum EstadoMiniatura {
    /// Se está sacando.
    Pendiente,
    /// Lista para mostrar.
    Lista {
        imagen: Handle<Image>,
        ancho: u32,
        alto: u32,
    },
    /// No se pudo sacar; el texto dice por qué.
    Fallo(String),
}

/// Lo que devuelve un hilo: la ruta y el cuadro (o el motivo del fallo).
type Resultado = (PathBuf, Result<Frame, String>);

/// Las miniaturas ya sacadas (o en camino), por **ruta completa** del archivo.
#[derive(Resource)]
pub struct Miniaturas {
    estados: HashMap<PathBuf, EstadoMiniatura>,
    en_curso: usize,
    /// Los hilos mandan sus resultados por acá. Un `Receiver` no se puede compartir entre
    /// hilos, así que va dentro de un `Mutex`, que Bevy exige en un recurso.
    emisor: Sender<Resultado>,
    receptor: Mutex<Receiver<Resultado>>,
}

impl Default for Miniaturas {
    fn default() -> Self {
        let (emisor, receptor) = mpsc::channel();
        Miniaturas {
            estados: HashMap::new(),
            en_curso: 0,
            emisor,
            receptor: Mutex::new(receptor),
        }
    }
}

impl Miniaturas {
    /// El estado de la miniatura del archivo con esa ruta completa, si ya se pidió.
    pub fn de(&self, ruta: &Path) -> Option<&EstadoMiniatura> {
        self.estados.get(ruta)
    }
}

/// Convierte el cuadro de un hilo en una imagen de Bevy.
fn imagen_de_frame(frame: Frame) -> Image {
    Image::new(
        Extent3d {
            width: frame.ancho,
            height: frame.alto,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        frame.datos,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Pide las miniaturas de los clips que todavía no tienen y recoge las que terminaron.
pub(crate) fn gestionar_miniaturas(
    proyecto: Res<ProyectoActual>,
    ruta: Res<RutaProyecto>,
    mut miniaturas: ResMut<Miniaturas>,
    mut imagenes: ResMut<Assets<Image>>,
) {
    // Lo que terminó desde el último frame.
    let terminadas: Vec<Resultado> = match miniaturas.receptor.lock() {
        Ok(receptor) => receptor.try_iter().collect(),
        Err(_) => Vec::new(),
    };

    // Los archivos de la grilla que todavía no tienen miniatura. Se mira con acceso de solo
    // lectura: pedir acceso de escritura marcaría el recurso como modificado en cada frame.
    let carpeta = ruta.carpeta();
    let mut faltantes: Vec<PathBuf> = Vec::new();
    for capa in &proyecto.0.composicion.capas {
        for clip in capa.celdas.iter().flatten() {
            // Solo los archivos (videos e imágenes) tienen miniatura: una cámara no se puede
            // «mirar» de antemano.
            let Some(guardada) = clip.fuente.ruta_de_archivo() else {
                continue;
            };
            let completa = resolver_ruta(&carpeta, guardada);
            if !miniaturas.estados.contains_key(&completa) && !faltantes.contains(&completa) {
                faltantes.push(completa);
            }
        }
    }

    if terminadas.is_empty() && faltantes.is_empty() {
        return;
    }

    for (archivo, resultado) in terminadas {
        miniaturas.en_curso = miniaturas.en_curso.saturating_sub(1);
        let estado = match resultado {
            Ok(frame) => {
                let (ancho, alto) = (frame.ancho, frame.alto);
                EstadoMiniatura::Lista {
                    imagen: imagenes.add(imagen_de_frame(frame)),
                    ancho,
                    alto,
                }
            }
            Err(motivo) => {
                warn!(
                    "no se pudo sacar la miniatura de {}: {motivo}",
                    archivo.display()
                );
                EstadoMiniatura::Fallo(motivo)
            }
        };
        miniaturas.estados.insert(archivo, estado);
    }

    for archivo in faltantes {
        if miniaturas.en_curso >= MAXIMO_A_LA_VEZ {
            break; // el resto se pide en los próximos frames, cuando terminen estas
        }
        miniaturas.en_curso += 1;
        miniaturas
            .estados
            .insert(archivo.clone(), EstadoMiniatura::Pendiente);
        let emisor = miniaturas.emisor.clone();
        std::thread::spawn(move || {
            let resultado = miniatura(&archivo, ANCHO_MINIATURA).map_err(|e| e.to_string());
            let _ = emisor.send((archivo, resultado));
        });
    }
}
