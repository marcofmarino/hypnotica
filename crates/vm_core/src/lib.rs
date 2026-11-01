//! Modelo de datos del proyecto. Rust puro: no depende de Bevy.
//!
//! El proyecto es la **fuente de verdad**: la interfaz, el teclado y el mouse
//! lo modifican, y la pantalla solo lo refleja. Eso permite guardarlo, cargarlo
//! y más adelante deshacer cambios o controlarlo desde afuera (MIDI/OSC).

mod audio;
mod color;
mod composicion;
mod control;
mod malla;
mod midi;
mod modulacion;
mod patrones;
mod poligono;

use std::fmt;
use std::path::Path;

use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

pub use audio::*;
pub use color::*;
pub use composicion::*;
pub use control::*;
pub use malla::*;
pub use midi::*;
pub use modulacion::*;
pub use patrones::*;
pub use poligono::*;

/// Versión del formato de archivo. Subila cuando cambie la estructura de los
/// datos, para poder migrar (o rechazar) proyectos guardados con versiones viejas.
///
/// - 1: las salidas se identificaban por su orden de aparición.
/// - 2: cada salida guarda el identificador de su monitor y si está activa.
/// - 3: cada superficie guarda su *entrada*: qué rectángulo de la composición muestra.
/// - 4: el proyecto guarda la composición (grilla de capas y clips, y su resolución).
/// - 5: cada superficie tiene nombre y visibilidad (varias superficies por salida).
/// - 6: cada superficie puede tener una *forma* (un polígono que la recorta).
/// - 7: cada superficie elige su *origen*: la composición o una capa.
/// - 8: cada capa tiene visibilidad, opacidad, modo de mezcla y ajuste del video.
/// - 9: cada superficie tiene un borde suave (edge blending).
/// - 10: cada superficie puede tener una malla de puntos de control (mesh warp).
/// - 17: las capturas de pantalla pueden traer audio (de una aplicación o del escritorio).
/// - 18: la composición tiene una duración de fundido al cambiar de clip.
/// - 19: las capturas de pantalla pueden ocultar el cursor del mouse.
/// - 20: un clip puede ser una imagen fija (`Fuente::Imagen`).
/// - 21: la opacidad de una capa puede moverse con una señal (`Modulacion`).
/// - 22: asignaciones de controles MIDI a destinos (`AsignacionMidi`).
/// - 23: cada modulación guarda su propia fuente de sonido (`Modulacion::fuente`).
pub const VERSION_ACTUAL: u32 = 23;

/// Todo lo que se guarda en un archivo de proyecto.
///
/// `derive` le pide a Rust que escriba el código por nosotros:
/// `Serialize`/`Deserialize` (de `serde`) convierten el struct a texto y viceversa,
/// `Clone` permite copiarlo y `PartialEq` compararlo con `==` (lo usan los tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proyecto {
    pub version: u32,
    /// La grilla de capas y clips. Los proyectos de la versión 3 no la traen: empiezan
    /// con una composición vacía.
    #[serde(default)]
    pub composicion: Composicion,
    /// Una configuración por salida (proyector), en el orden en que se abrieron.
    pub salidas: Vec<ConfigSalida>,
    /// Qué control MIDI maneja qué cosa (se arman con «MIDI learn»). Van en el proyecto porque
    /// apuntan a sus capas y clips.
    #[serde(default)]
    pub asignaciones_midi: Vec<AsignacionMidi>,
}

/// Configuración de una salida: qué superficies se dibujan en ese proyector.
///
/// `#[serde(default)]` en un campo significa "si el archivo no lo trae, usá el valor
/// por defecto". Así los archivos de la versión 1 (sin estos campos) se siguen leyendo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSalida {
    /// Identificador del monitor al que pertenece (ver `id_de_monitor` en `vm_output`).
    /// Vacío en proyectos viejos: ese monitor se "adopta" al primer arranque.
    #[serde(default)]
    pub monitor: String,
    /// Si la salida está encendida (con ventana fullscreen en ese monitor).
    #[serde(default = "verdadero")]
    pub activa: bool,
    pub superficies: Vec<ConfigSuperficie>,
    /// Calibración de color de este proyector: se aplica a todas sus superficies.
    #[serde(default)]
    pub color: CorreccionColor,
    /// Máscaras de salida: polígonos, en píxeles del proyector (como las esquinas), que se
    /// dibujan en negro **encima de todo**. Sirven para tapar lo que no debe recibir luz (un
    /// marco, una ventana, un objeto delante de la pared).
    #[serde(default)]
    pub mascaras: Vec<Vec<Punto>>,
}

fn verdadero() -> bool {
    true
}

/// Rectángulo de la composición que muestra una superficie: `[x_min, y_min, x_max, y_max]`,
/// con valores de 0 a 1 y la `y` creciendo hacia abajo (como una imagen).
/// `[0, 0, 1, 1]` es la composición entera.
pub type Entrada = [f32; 4];

/// La composición entera.
pub const ENTRADA_COMPLETA: Entrada = [0.0, 0.0, 1.0, 1.0];

/// Tamaño mínimo de la entrada en cada eje (fracción de la composición): evita rectángulos
/// de tamaño cero, que dejarían la superficie sin imagen.
pub const ENTRADA_TAMANO_MINIMO: f32 = 0.02;

fn entrada_completa() -> Entrada {
    ENTRADA_COMPLETA
}

/// Deja un rectángulo de entrada en un estado válido: ordenado (mínimo antes que máximo),
/// dentro de 0..1 y con al menos `ENTRADA_TAMANO_MINIMO` de ancho y de alto.
pub fn normalizar_entrada(r: Entrada) -> Entrada {
    let (x0, x1) = ajustar_tramo(r[0], r[2]);
    let (y0, y1) = ajustar_tramo(r[1], r[3]);
    [x0, y0, x1, y1]
}

/// Ordena, limita a 0..1 y garantiza el tamaño mínimo de un tramo `[a, b]`.
fn ajustar_tramo(a: f32, b: f32) -> (f32, f32) {
    let (mut min, mut max) = (a.min(b).clamp(0.0, 1.0), a.max(b).clamp(0.0, 1.0));
    if max - min < ENTRADA_TAMANO_MINIMO {
        // Se agranda hacia el lado que tenga lugar, sin salirse de 0..1.
        if min + ENTRADA_TAMANO_MINIMO <= 1.0 {
            max = min + ENTRADA_TAMANO_MINIMO;
        } else {
            max = 1.0;
            min = 1.0 - ENTRADA_TAMANO_MINIMO;
        }
    }
    (min, max)
}

/// El máximo ancho de un borde suave: la mitad de la superficie (dos bordes opuestos no
/// pueden pasarse uno del otro).
pub const BORDE_MAXIMO: f32 = 0.5;

/// Límites de la potencia de la curva del borde.
pub const POTENCIA_MINIMA: f32 = 0.2;
pub const POTENCIA_MAXIMA: f32 = 5.0;

fn potencia_neutra() -> f32 {
    1.0
}

/// El **borde suave** de una superficie (*edge blending*): sus lados se van apagando hacia
/// el borde para que, al solaparse con la imagen de otro proyector, la unión no se note.
///
/// Los anchos son fracciones de la superficie (0 = sin borde, 0,5 = la mitad). Se aplican
/// en las coordenadas de la superficie, así que **se deforman junto con ella**.
///
/// La curva es suave (`smoothstep`), que cumple `rampa(x) + rampa(1 − x) = 1`: si en el
/// solape un proyector se apaga con una rampa y el otro se enciende con la espejada, la luz
/// que suman es constante. Esto vale en luz lineal, que es donde mezcla la GPU.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BordeSuave {
    pub izquierda: f32,
    pub derecha: f32,
    pub arriba: f32,
    pub abajo: f32,
    /// Ajusta la curva de todo el borde: 1 es la neutra; más de 1 lo oscurece (la rampa
    /// arranca más tarde), menos de 1 lo aclara. Sirve para compensar la respuesta real de
    /// un proyector. Con valores distintos de 1 las rampas espejadas ya no suman exactamente 1.
    #[serde(default = "potencia_neutra")]
    pub potencia: f32,
}

impl Default for BordeSuave {
    fn default() -> Self {
        BordeSuave {
            izquierda: 0.0,
            derecha: 0.0,
            arriba: 0.0,
            abajo: 0.0,
            potencia: 1.0,
        }
    }
}

impl BordeSuave {
    /// La rampa de un borde: 0 en el extremo, 1 a `ancho` de distancia, con curva suave.
    /// Un ancho nulo (o negativo) significa "sin borde": vale 1 siempre.
    fn rampa(distancia_al_borde: f32, ancho: f32) -> f32 {
        if ancho <= 0.0 {
            return 1.0;
        }
        let t = (distancia_al_borde / ancho).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Cuánta luz queda en el punto `(u, v)` de la superficie (de 0 a 1). Es la misma cuenta
    /// que hace el shader.
    pub fn atenuacion(&self, u: f32, v: f32) -> f32 {
        let producto = Self::rampa(u, self.izquierda)
            * Self::rampa(1.0 - u, self.derecha)
            * Self::rampa(v, self.arriba)
            * Self::rampa(1.0 - v, self.abajo);
        producto.powf(self.potencia)
    }

    /// ¿Tiene algún borde activo?
    pub fn activo(&self) -> bool {
        self.izquierda > 0.0 || self.derecha > 0.0 || self.arriba > 0.0 || self.abajo > 0.0
    }

    /// Deja los valores dentro de lo permitido (por ejemplo, tras leer un archivo editado a mano).
    pub fn normalizar(&mut self) {
        for lado in [
            &mut self.izquierda,
            &mut self.derecha,
            &mut self.arriba,
            &mut self.abajo,
        ] {
            *lado = if lado.is_nan() {
                0.0
            } else {
                lado.clamp(0.0, BORDE_MAXIMO)
            };
        }
        self.potencia = if self.potencia.is_nan() {
            1.0
        } else {
            self.potencia.clamp(POTENCIA_MINIMA, POTENCIA_MAXIMA)
        };
    }
}

/// De dónde toma la imagen una superficie.
///
/// Con `Composicion`, la superficie muestra una parte de la imagen que resulta de **mezclar
/// todas las capas**. Con `Capa(n)`, muestra directamente el clip que suena en esa capa
/// (sin mezclar con las demás): así cada salida puede mostrar un clip distinto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OrigenSuperficie {
    #[default]
    Composicion,
    /// El clip de la capa con ese índice (la primera es la 0).
    Capa(usize),
}

impl OrigenSuperficie {
    /// El nombre para mostrar en la interfaz: «Composición» o **solo el nombre de la capa**
    /// (poner además su número daba cosas como «Capa 1: Capa 2» después de reordenar). Si la
    /// capa no tiene nombre, «Capa N».
    pub fn nombre(&self, comp: &Composicion) -> String {
        match self {
            OrigenSuperficie::Composicion => "Composición".to_string(),
            OrigenSuperficie::Capa(i) => match comp.capas.get(*i) {
                Some(capa) if !capa.nombre.trim().is_empty() => capa.nombre.clone(),
                Some(_) => format!("Capa {}", i + 1),
                None => format!("Capa {} (no existe)", i + 1),
            },
        }
    }
}

/// Una superficie: un cuadrilátero con imagen, definido por sus 4 esquinas en coordenadas
/// de pantalla. Una salida puede tener varias (cada una muestra una parte distinta de la
/// composición y se deforma por separado).
/// Orden de las esquinas: sup-izq, sup-der, inf-der, inf-izq (igual que `Homografia::desde_esquinas`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSuperficie {
    /// Nombre para reconocerla en la interfaz. Vacío en proyectos viejos: ver `nombre_visible`.
    #[serde(default)]
    pub nombre: String,
    /// Si se dibuja. Una superficie oculta sigue existiendo y guardada.
    #[serde(default = "verdadero")]
    pub visible: bool,
    pub esquinas: [[f32; 2]; 4],
    /// Qué parte de la composición se muestra (ver `Entrada`). Los proyectos de la
    /// versión 2, que no lo traían, muestran la composición entera.
    #[serde(default = "entrada_completa")]
    pub entrada: Entrada,
    /// Un polígono, en coordenadas de la superficie (de 0 a 1, con la `y` hacia abajo), que
    /// la recorta: solo se dibuja lo que queda adentro. `None` es la superficie entera.
    ///
    /// Como se define en la superficie y no en la pantalla, **se deforma junto con ella**:
    /// un recorte hexagonal sobre un cuadrilátero con perspectiva sale como un hexágono con
    /// perspectiva. Siempre es un polígono simple (`es_simple`); ver `normalizar`.
    #[serde(default)]
    pub forma: Option<Vec<Punto>>,
    /// De dónde toma la imagen (ver `OrigenSuperficie`). Antes de la versión 7, siempre la
    /// composición.
    #[serde(default)]
    pub origen: OrigenSuperficie,
    /// El borde suave de la superficie (ver `BordeSuave`). Antes de la versión 9, sin borde.
    #[serde(default)]
    pub borde: BordeSuave,
    /// La malla de puntos de control (*mesh warp*), si la tiene: curva la superficie más
    /// allá de lo que permite el corner pin. Ver `Malla`.
    #[serde(default)]
    pub malla: Option<Malla>,
    /// Huecos dentro de la superficie: polígonos, en coordenadas de la superficie (como
    /// `forma`), que **no se dibujan**. Como la forma, se deforman junto con la superficie
    /// (esquinas y malla): un agujero circular sobre una pared inclinada sale como una elipse.
    #[serde(default)]
    pub huecos: Vec<Vec<Punto>>,
}

impl ConfigSuperficie {
    /// Rectángulo centrado en el origen que ocupa la `fraccion` (0 a 1) de un
    /// área de `ancho` × `alto`.
    pub fn rectangulo_centrado(ancho: f32, alto: f32, fraccion: f32) -> Self {
        let (mx, my) = (ancho * fraccion / 2.0, alto * fraccion / 2.0);
        ConfigSuperficie {
            nombre: String::new(),
            visible: true,
            esquinas: [[-mx, my], [mx, my], [mx, -my], [-mx, -my]],
            entrada: ENTRADA_COMPLETA,
            forma: None,
            origen: OrigenSuperficie::Composicion,
            borde: BordeSuave::default(),
            malla: None,
            huecos: Vec::new(),
        }
    }

    /// Empieza una forma a partir de un rectángulo que ocupa toda la superficie. No hace
    /// nada si ya tiene una.
    pub fn empezar_forma(&mut self) {
        if self.forma.is_none() {
            self.forma = Some(rectangulo_unitario());
        }
    }

    /// Deja un archivo leído en un estado válido: una forma que no sea un polígono simple
    /// (por ejemplo, editada a mano) se descarta, y sus puntos se limitan a la superficie.
    pub fn normalizar(&mut self) {
        self.borde.normalizar();
        // Una malla que no se puede arreglar se descarta.
        if let Some(malla) = &mut self.malla
            && !malla.normalizar()
        {
            self.malla = None;
        }
        if let Some(forma) = &mut self.forma {
            for punto in forma.iter_mut() {
                punto[0] = punto[0].clamp(0.0, 1.0);
                punto[1] = punto[1].clamp(0.0, 1.0);
            }
            if !es_simple(forma) {
                self.forma = None;
            }
        }
        // Los huecos: dentro de la superficie y sin lados que se crucen.
        for hueco in &mut self.huecos {
            for punto in hueco.iter_mut() {
                punto[0] = punto[0].clamp(0.0, 1.0);
                punto[1] = punto[1].clamp(0.0, 1.0);
            }
        }
        self.huecos.retain(|h| {
            h.len() >= 3 && h.iter().all(|p| p[0].is_finite() && p[1].is_finite()) && es_simple(h)
        });
    }

    /// Agrega un hueco rectangular en el centro de la superficie (un tercio de su tamaño) y
    /// devuelve su posición.
    pub fn agregar_hueco(&mut self) -> usize {
        self.huecos.push(vec![
            [1.0 / 3.0, 1.0 / 3.0],
            [2.0 / 3.0, 1.0 / 3.0],
            [2.0 / 3.0, 2.0 / 3.0],
            [1.0 / 3.0, 2.0 / 3.0],
        ]);
        self.huecos.len() - 1
    }

    /// El nombre para mostrar: el propio, o "Superficie N" si no tiene (N cuenta desde 1).
    pub fn nombre_visible(&self, indice: usize) -> String {
        if self.nombre.trim().is_empty() {
            format!("Superficie {}", indice + 1)
        } else {
            self.nombre.clone()
        }
    }
}

impl ConfigSalida {
    /// Descarta las máscaras que no sirven (menos de 3 puntos, números inválidos o lados que
    /// se cruzan): un archivo editado a mano puede traer cualquier cosa.
    pub fn normalizar_mascaras(&mut self) {
        self.mascaras.retain(|m| {
            m.len() >= 3 && m.iter().all(|p| p[0].is_finite() && p[1].is_finite()) && es_simple(m)
        });
    }

    /// Agrega una máscara rectangular centrada, de `fraccion` del tamaño del proyector
    /// (`ancho` × `alto`), y devuelve su posición.
    pub fn agregar_mascara(&mut self, ancho: f32, alto: f32, fraccion: f32) -> usize {
        let (mx, my) = (ancho * fraccion / 2.0, alto * fraccion / 2.0);
        self.mascaras
            .push(vec![[-mx, my], [mx, my], [mx, -my], [-mx, -my]]);
        self.mascaras.len() - 1
    }

    /// Salida nueva, activa, con una sola superficie que ocupa el 60 % de la pantalla.
    pub fn nueva(monitor: String, ancho: f32, alto: f32) -> Self {
        let mut salida = ConfigSalida {
            monitor,
            activa: true,
            superficies: vec![ConfigSuperficie::rectangulo_centrado(ancho, alto, 0.6)],
            color: CorreccionColor::default(),
            mascaras: Vec::new(),
        };
        salida.normalizar_nombres();
        salida
    }

    /// El primer «Superficie N» (N desde 1) que ninguna otra superficie de esta salida usa.
    pub fn nombre_nuevo(&self) -> String {
        let usados: std::collections::HashSet<&str> =
            self.superficies.iter().map(|s| s.nombre.as_str()).collect();
        (1..)
            .map(|n| format!("Superficie {n}"))
            .find(|candidato| !usados.contains(candidato.as_str()))
            .unwrap_or_default()
    }

    /// Les da un nombre propio y **fijo** a las superficies que no tienen. Con el nombre
    /// vacío la interfaz lo inventaba según la posición («Superficie 3» por ser la tercera), y
    /// al reordenar los nombres cambiaban y se repetían.
    pub fn normalizar_nombres(&mut self) {
        for i in 0..self.superficies.len() {
            if self.superficies[i].nombre.trim().is_empty() {
                // Se limpia antes de buscar, para que el vacío no cuente como usado.
                self.superficies[i].nombre = String::new();
                self.superficies[i].nombre = self.nombre_nuevo();
            }
        }
    }

    /// Agrega una superficie nueva, más chica que la salida (para que se distinga de las
    /// demás), y devuelve su posición. `ancho` y `alto` son el tamaño de la salida.
    pub fn agregar_superficie(&mut self, ancho: f32, alto: f32) -> usize {
        let mut nueva = ConfigSuperficie::rectangulo_centrado(ancho, alto, 0.4);
        nueva.nombre = self.nombre_nuevo();
        self.superficies.push(nueva);
        self.superficies.len() - 1
    }

    /// Duplica una superficie, corrida un poco para que no quede exactamente encima.
    /// Devuelve la posición de la copia, que queda justo después de la original.
    pub fn duplicar_superficie(&mut self, indice: usize) -> Option<usize> {
        let mut copia = self.superficies.get(indice)?.clone();
        copia.nombre = format!("{} (copia)", copia.nombre_visible(indice));
        for esquina in &mut copia.esquinas {
            esquina[0] += DESPLAZAMIENTO_DE_COPIA;
            esquina[1] -= DESPLAZAMIENTO_DE_COPIA;
        }
        self.superficies.insert(indice + 1, copia);
        Some(indice + 1)
    }

    /// Borra una superficie. Siempre queda al menos una: devuelve `false` (y no hace nada)
    /// si es la última o no existe.
    pub fn borrar_superficie(&mut self, indice: usize) -> bool {
        if self.superficies.len() <= 1 || indice >= self.superficies.len() {
            return false;
        }
        self.superficies.remove(indice);
        true
    }

    /// Mueve una superficie `delta` lugares en la lista (+1 hacia adelante, −1 hacia
    /// atrás). Las últimas se dibujan **encima** de las primeras. Devuelve su nueva
    /// posición, o `None` si el movimiento se saldría de la lista.
    pub fn mover_superficie(&mut self, indice: usize, delta: isize) -> Option<usize> {
        let destino = indice.checked_add_signed(delta)?;
        if indice >= self.superficies.len() || destino >= self.superficies.len() {
            return None;
        }
        self.superficies.swap(indice, destino);
        Some(destino)
    }
}

/// Cuánto se corre una superficie duplicada (en unidades de la salida).
const DESPLAZAMIENTO_DE_COPIA: f32 = 40.0;

impl Default for Proyecto {
    fn default() -> Self {
        Proyecto {
            version: VERSION_ACTUAL,
            composicion: Composicion::default(),
            salidas: Vec::new(),
            asignaciones_midi: Vec::new(),
        }
    }
}

/// Lo que puede salir mal al guardar o cargar. Un `enum` es un tipo que vale
/// *una* de varias opciones; así quien lo reciba puede distinguir los casos.
#[derive(Debug)]
pub enum ErrorProyecto {
    /// No se pudo leer o escribir el archivo.
    Archivo(std::io::Error),
    /// El texto no tiene el formato esperado.
    Formato(String),
    /// El archivo es de una versión más nueva que la que entiende este programa.
    VersionNoSoportada { encontrada: u32, maxima: u32 },
}

impl fmt::Display for ErrorProyecto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorProyecto::Archivo(e) => write!(f, "error de archivo: {e}"),
            ErrorProyecto::Formato(e) => write!(f, "formato inválido: {e}"),
            ErrorProyecto::VersionNoSoportada { encontrada, maxima } => write!(
                f,
                "el proyecto es de la versión {encontrada}, este programa entiende hasta la {maxima}"
            ),
        }
    }
}

impl std::error::Error for ErrorProyecto {}

/// Con este `From`, el operador `?` convierte solo un `std::io::Error` en un
/// `ErrorProyecto::Archivo`, sin escribir `map_err` en cada llamada.
impl From<std::io::Error> for ErrorProyecto {
    fn from(e: std::io::Error) -> Self {
        ErrorProyecto::Archivo(e)
    }
}

impl Proyecto {
    /// Copia la superficie `indice` de la salida `origen` al final de la salida `destino`.
    /// Las posiciones (esquinas y deformación de la malla) se multiplican por `escala` (el
    /// tamaño del proyector de destino dividido por el del de origen, en `x` y en `y`), así la
    /// copia ocupa el mismo lugar relativo aunque los proyectores tengan otra resolución.
    /// Devuelve la posición de la copia, o `None` si la salida o la superficie no existen.
    pub fn copiar_superficie(
        &mut self,
        origen: &str,
        indice: usize,
        destino: &str,
        escala: [f32; 2],
    ) -> Option<usize> {
        let mut copia = self
            .salidas
            .iter()
            .find(|s| s.monitor == origen)?
            .superficies
            .get(indice)?
            .clone();
        let nombre = copia.nombre_visible(indice);
        copia.nombre = if origen == destino {
            format!("{nombre} (copia)")
        } else {
            format!("{nombre} ({origen})")
        };
        for esquina in &mut copia.esquinas {
            esquina[0] *= escala[0];
            esquina[1] *= escala[1];
        }
        if let Some(malla) = &mut copia.malla {
            for d in &mut malla.desplazamientos {
                d[0] *= escala[0];
                d[1] *= escala[1];
            }
        }
        let salida = self.salidas.iter_mut().find(|s| s.monitor == destino)?;
        salida.superficies.push(copia);
        Some(salida.superficies.len() - 1)
    }

    /// Mueve la capa `desde` a la posición `hasta` (la 0 es la de más arriba) y **corrige las
    /// referencias**: una superficie que toma una capa la refiere por su posición, y al
    /// reordenar tiene que seguir tomando *la misma capa* aunque cambie de lugar.
    ///
    /// Devuelve `false` (y no hace nada) si alguna posición no existe o no hay movimiento.
    pub fn mover_capa(&mut self, desde: usize, hasta: usize) -> bool {
        let cantidad = self.composicion.capas.len();
        if desde >= cantidad || hasta >= cantidad || desde == hasta {
            return false;
        }
        let capa = self.composicion.capas.remove(desde);
        self.composicion.capas.insert(hasta, capa);
        for sup in self
            .salidas
            .iter_mut()
            .flat_map(|s| s.superficies.iter_mut())
        {
            if let OrigenSuperficie::Capa(n) = sup.origen {
                sup.origen = OrigenSuperficie::Capa(nueva_posicion(n, desde, hasta));
            }
        }
        true
    }

    /// Quita la capa `indice` de la composición, y **corrige las referencias a las capas**.
    ///
    /// Una superficie que toma una capa (`OrigenSuperficie::Capa(n)`) la refiere por su
    /// posición. Al quitar una capa del medio, las siguientes corren un lugar: sin este
    /// arreglo, esas superficies pasarían a mostrar otra capa sin avisar. Las que
    /// apuntaban a la capa quitada vuelven a mostrar la composición; las que apuntaban a
    /// una posterior se renumeran.
    ///
    /// Siempre queda al menos una capa: devuelve `false` (y no hace nada) si es la única o
    /// no existe.
    pub fn quitar_capa(&mut self, indice: usize) -> bool {
        if self.composicion.capas.len() <= 1 || indice >= self.composicion.capas.len() {
            return false;
        }
        self.composicion.capas.remove(indice);
        for sup in self
            .salidas
            .iter_mut()
            .flat_map(|s| s.superficies.iter_mut())
        {
            if let OrigenSuperficie::Capa(n) = sup.origen {
                sup.origen = match n.cmp(&indice) {
                    std::cmp::Ordering::Equal => OrigenSuperficie::Composicion,
                    std::cmp::Ordering::Greater => OrigenSuperficie::Capa(n - 1),
                    std::cmp::Ordering::Less => OrigenSuperficie::Capa(n),
                };
            }
        }
        true
    }

    /// Convierte el proyecto a texto RON.
    pub fn a_texto(&self) -> Result<String, ErrorProyecto> {
        ron::ser::to_string_pretty(self, PrettyConfig::default())
            .map_err(|e| ErrorProyecto::Formato(e.to_string()))
    }

    /// Lee un proyecto desde texto RON y verifica su versión.
    pub fn desde_texto(texto: &str) -> Result<Self, ErrorProyecto> {
        let mut proyecto: Proyecto =
            ron::from_str(texto).map_err(|e| ErrorProyecto::Formato(e.to_string()))?;
        if proyecto.version > VERSION_ACTUAL {
            return Err(ErrorProyecto::VersionNoSoportada {
                encontrada: proyecto.version,
                maxima: VERSION_ACTUAL,
            });
        }
        // Migración: los campos nuevos ya se completaron con sus valores por defecto,
        // así que alcanza con marcar el proyecto como de la versión actual.
        proyecto.version = VERSION_ACTUAL;
        proyecto.composicion.normalizar();
        for salida in &mut proyecto.salidas {
            salida.color.normalizar();
            salida.normalizar_mascaras();
            salida.normalizar_nombres();
            for sup in &mut salida.superficies {
                sup.normalizar();
            }
        }
        Ok(proyecto)
    }

    /// Guarda el proyecto en un archivo.
    pub fn guardar(&self, ruta: impl AsRef<Path>) -> Result<(), ErrorProyecto> {
        std::fs::write(ruta, self.a_texto()?)?;
        Ok(())
    }

    /// Carga un proyecto desde un archivo.
    pub fn cargar(ruta: impl AsRef<Path>) -> Result<Self, ErrorProyecto> {
        Self::desde_texto(&std::fs::read_to_string(ruta)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proyecto_de_ejemplo() -> Proyecto {
        let mut composicion = Composicion {
            ancho: 3840,
            alto: 1080,
            ..Default::default()
        };
        composicion.poner_clip(0, 1, ConfigClip::de_video("videos/fondo.mp4".into()));
        Proyecto {
            version: VERSION_ACTUAL,
            composicion,
            asignaciones_midi: vec![AsignacionMidi {
                entrada: EntradaMidi {
                    canal: 0,
                    tipo: TipoMidi::Nota,
                    numero: 36,
                },
                destino: DestinoDeControl::Clip {
                    capa: 0,
                    columna: 1,
                },
            }],
            salidas: vec![
                ConfigSalida::nueva("HDMI-A-1".into(), 1920.0, 1080.0),
                ConfigSalida {
                    monitor: "DP-2".into(),
                    activa: false,
                    superficies: vec![ConfigSuperficie {
                        nombre: "Fachada".into(),
                        visible: false,
                        esquinas: [
                            [-100.5, 80.25],
                            [90.0, 70.0],
                            [95.0, -60.0],
                            [-110.0, -75.5],
                        ],
                        entrada: [0.25, 0.1, 0.75, 0.9],
                        forma: Some(vec![[0.0, 0.0], [1.0, 0.2], [0.8, 1.0], [0.1, 0.9]]),
                        origen: OrigenSuperficie::Capa(1),
                        malla: Some({
                            let mut m = Malla::nueva(3, 3);
                            m.poner(1, 1, [12.5, -8.0]);
                            m
                        }),
                        borde: BordeSuave {
                            izquierda: 0.1,
                            derecha: 0.2,
                            arriba: 0.0,
                            abajo: 0.05,
                            potencia: 1.5,
                        },
                        huecos: vec![vec![[0.4, 0.4], [0.6, 0.4], [0.5, 0.6]]],
                    }],
                    color: CorreccionColor {
                        rojo: 0.9,
                        brillo: -0.05,
                        gamma: 1.2,
                        ..Default::default()
                    },
                    mascaras: vec![vec![[-50.0, 40.0], [60.0, 30.0], [10.0, -70.0]]],
                },
            ],
        }
    }

    #[test]
    fn un_proyecto_viejo_sin_color_se_lee_con_la_correccion_neutra() {
        let texto = r#"Proyecto(version: 10, composicion: (ancho: 1920, alto: 1080, columnas: [(nombre: "1")], capas: []),
            salidas: [(monitor: "DP-1", superficies: [])])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        assert!(p.salidas[0].color.es_neutra());
        assert_eq!(p.version, VERSION_ACTUAL);
    }

    #[test]
    fn ida_y_vuelta_por_texto() {
        let original = proyecto_de_ejemplo();
        let texto = original.a_texto().unwrap();
        let leido = Proyecto::desde_texto(&texto).unwrap();
        assert_eq!(original, leido);
    }

    #[test]
    fn ida_y_vuelta_por_archivo() {
        let original = proyecto_de_ejemplo();
        let ruta = std::env::temp_dir().join("vm_core_prueba_ida_y_vuelta.ron");
        original.guardar(&ruta).unwrap();
        let leido = Proyecto::cargar(&ruta).unwrap();
        let _ = std::fs::remove_file(&ruta);
        assert_eq!(original, leido);
    }

    #[test]
    fn rechaza_version_futura() {
        let mut p = proyecto_de_ejemplo();
        p.version = VERSION_ACTUAL + 1;
        let texto = p.a_texto().unwrap();
        match Proyecto::desde_texto(&texto) {
            Err(ErrorProyecto::VersionNoSoportada { encontrada, maxima }) => {
                assert_eq!(encontrada, VERSION_ACTUAL + 1);
                assert_eq!(maxima, VERSION_ACTUAL);
            }
            otro => panic!("se esperaba VersionNoSoportada, salió {otro:?}"),
        }
    }

    #[test]
    fn migra_un_proyecto_de_la_version_1() {
        // Así se guardaba en la versión 1: sin `monitor` ni `activa`, con `nombre`.
        let viejo = r#"Proyecto(version: 1, salidas: [(nombre: "Salida 0", superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas.len(), 1);
        assert_eq!(p.salidas[0].monitor, ""); // sin dueño: se adopta al arrancar
        assert!(p.salidas[0].activa);
        assert_eq!(p.salidas[0].superficies[0].esquinas[0], [-10.0, 5.0]);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_2() {
        // La versión 2 ya tenía monitor y activa, pero no `entrada`.
        let viejo = r#"Proyecto(version: 2, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas[0].monitor, "DP-1");
        assert_eq!(p.salidas[0].superficies[0].entrada, ENTRADA_COMPLETA);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_4() {
        // La versión 4 tenía composición pero las superficies no tenían nombre ni visibilidad.
        let viejo = r#"Proyecto(version: 4, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             entrada: (0.0, 0.0, 1.0, 1.0))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        let sup = &p.salidas[0].superficies[0];
        assert_eq!(p.version, VERSION_ACTUAL);
        assert!(sup.visible);
        // El proyecto viejo no tenía nombres: al cargarlo recibe uno propio y fijo.
        assert_eq!(sup.nombre, "Superficie 1");
        assert_eq!(sup.nombre_visible(0), "Superficie 1");
    }

    #[test]
    fn migra_un_proyecto_de_la_version_5() {
        // La versión 5 tenía nombre y visibilidad, pero no `forma`.
        let viejo = r#"Proyecto(version: 5, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (nombre: "A", visible: true,
             esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             entrada: (0.0, 0.0, 1.0, 1.0))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas[0].superficies[0].forma, None);
    }

    #[test]
    fn una_forma_invalida_en_un_archivo_se_descarta_al_leer() {
        // Un "moño" (lados cruzados): no se puede rellenar, así que se descarta.
        let texto = r#"Proyecto(version: 6, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             forma: Some([(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]))])])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        assert_eq!(p.salidas[0].superficies[0].forma, None);
    }

    #[test]
    fn una_forma_con_puntos_fuera_de_la_superficie_se_limita() {
        let texto = r#"Proyecto(version: 6, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             forma: Some([(-0.5, 0.0), (1.5, 0.0), (1.0, 1.0), (0.0, 1.0)]))])])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        let forma = p.salidas[0].superficies[0].forma.as_ref().unwrap();
        assert_eq!(forma[0], [0.0, 0.0]);
        assert_eq!(forma[1], [1.0, 0.0]);
    }

    #[test]
    fn empezar_una_forma_parte_de_un_rectangulo_y_no_pisa_una_existente() {
        let mut sup = ConfigSuperficie::rectangulo_centrado(100.0, 100.0, 0.5);
        sup.empezar_forma();
        assert_eq!(sup.forma, Some(rectangulo_unitario()));
        sup.forma.as_mut().unwrap()[0] = [0.2, 0.2];
        sup.empezar_forma(); // ya tiene una: no se reinicia
        assert_eq!(sup.forma.as_ref().unwrap()[0], [0.2, 0.2]);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_7() {
        // La versión 7 tenía capas sin visibilidad, opacidad, modo ni ajuste.
        let viejo = r#"Proyecto(version: 7, composicion: (ancho: 1920, alto: 1080,
            columnas: [(nombre: "1")],
            capas: [(nombre: "Fondo", celdas: [None])]), salidas: [])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        let capa = &p.composicion.capas[0];
        assert_eq!(p.version, VERSION_ACTUAL);
        assert!(capa.visible);
        assert_eq!(capa.opacidad, 1.0);
        assert_eq!(capa.modo, ModoMezcla::Normal);
        assert_eq!(capa.ajuste, Ajuste::Ajustar);
    }

    #[test]
    fn la_configuracion_de_una_capa_se_guarda_y_se_lee() {
        let mut p = Proyecto::default();
        p.composicion.capas[0].visible = false;
        p.composicion.capas[0].opacidad = 0.4;
        p.composicion.capas[0].modo = ModoMezcla::Multiplicar;
        p.composicion.capas[0].ajuste = Ajuste::Llenar;
        let leido = Proyecto::desde_texto(&p.a_texto().unwrap()).unwrap();
        assert_eq!(leido.composicion.capas[0], p.composicion.capas[0]);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_6() {
        // La versión 6 no tenía `origen`: toda superficie mostraba la composición.
        let viejo = r#"Proyecto(version: 6, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(
            p.salidas[0].superficies[0].origen,
            OrigenSuperficie::Composicion
        );
    }

    #[test]
    fn el_origen_se_guarda_y_se_lee() {
        let mut p = Proyecto::default();
        let mut salida = ConfigSalida::nueva("DP-1".into(), 100.0, 100.0);
        salida.superficies[0].origen = OrigenSuperficie::Capa(2);
        p.salidas.push(salida);
        let leido = Proyecto::desde_texto(&p.a_texto().unwrap()).unwrap();
        assert_eq!(
            leido.salidas[0].superficies[0].origen,
            OrigenSuperficie::Capa(2)
        );
    }

    #[test]
    fn el_nombre_del_origen_incluye_la_capa_o_avisa_que_no_existe() {
        let mut comp = Composicion::default();
        comp.capas[0].nombre = "Fondo".into();
        assert_eq!(OrigenSuperficie::Composicion.nombre(&comp), "Composición");
        // Solo el nombre de la capa, sin su número.
        assert_eq!(OrigenSuperficie::Capa(0).nombre(&comp), "Fondo");
        assert_eq!(
            OrigenSuperficie::Capa(5).nombre(&comp),
            "Capa 6 (no existe)"
        );
    }

    /// Un proyecto con 4 capas y 4 superficies, cada una tomando una capa distinta
    /// (o la composición, la última).
    fn proyecto_con_capas_y_origenes() -> Proyecto {
        let mut p = Proyecto::default();
        for _ in 0..3 {
            p.composicion.agregar_capa();
        }
        for (i, capa) in p.composicion.capas.iter_mut().enumerate() {
            capa.nombre = format!("C{i}");
        }
        let mut salida = ConfigSalida::nueva("DP-1".into(), 100.0, 100.0);
        for _ in 0..3 {
            salida.agregar_superficie(100.0, 100.0);
        }
        salida.superficies[0].origen = OrigenSuperficie::Capa(0);
        salida.superficies[1].origen = OrigenSuperficie::Capa(1);
        salida.superficies[2].origen = OrigenSuperficie::Capa(3);
        salida.superficies[3].origen = OrigenSuperficie::Composicion;
        p.salidas.push(salida);
        p
    }

    fn origenes(p: &Proyecto) -> Vec<OrigenSuperficie> {
        p.salidas[0].superficies.iter().map(|s| s.origen).collect()
    }

    #[test]
    fn quitar_una_capa_del_medio_corrige_las_referencias_de_las_superficies() {
        let mut p = proyecto_con_capas_y_origenes();
        assert!(p.quitar_capa(1));
        let nombres: Vec<_> = p
            .composicion
            .capas
            .iter()
            .map(|c| c.nombre.as_str())
            .collect();
        assert_eq!(nombres, ["C0", "C2", "C3"]);
        assert_eq!(
            origenes(&p),
            vec![
                OrigenSuperficie::Capa(0),     // anterior a la quitada: igual
                OrigenSuperficie::Composicion, // apuntaba a la quitada
                OrigenSuperficie::Capa(2),     // era la 3: ahora es la 2 (sigue siendo "C3")
                OrigenSuperficie::Composicion, // no usaba capas
            ]
        );
    }

    #[test]
    fn mover_una_capa_hace_que_cada_superficie_siga_tomando_la_misma_capa() {
        let mut p = proyecto_con_capas_y_origenes();
        // Las capas son C0, C1, C2, C3 y las superficies toman C0, C1, C3 y la composición.
        assert!(p.mover_capa(3, 0)); // C3 pasa al principio: C3, C0, C1, C2
        let nombres: Vec<_> = p
            .composicion
            .capas
            .iter()
            .map(|c| c.nombre.as_str())
            .collect();
        assert_eq!(nombres, ["C3", "C0", "C1", "C2"]);
        assert_eq!(
            origenes(&p),
            vec![
                OrigenSuperficie::Capa(1), // sigue tomando C0, que ahora está en la posición 1
                OrigenSuperficie::Capa(2), // C1, ahora en la 2
                OrigenSuperficie::Capa(0), // C3, ahora en la 0
                OrigenSuperficie::Composicion,
            ]
        );
    }

    #[test]
    fn mover_una_capa_hacia_atras_tambien_corrige_las_referencias() {
        let mut p = proyecto_con_capas_y_origenes();
        assert!(p.mover_capa(0, 2)); // C0 pasa a la posición 2: C1, C2, C0, C3
        let nombres: Vec<_> = p
            .composicion
            .capas
            .iter()
            .map(|c| c.nombre.as_str())
            .collect();
        assert_eq!(nombres, ["C1", "C2", "C0", "C3"]);
        assert_eq!(
            origenes(&p),
            vec![
                OrigenSuperficie::Capa(2), // C0
                OrigenSuperficie::Capa(0), // C1
                OrigenSuperficie::Capa(3), // C3 no se movió
                OrigenSuperficie::Composicion,
            ]
        );
    }

    #[test]
    fn mover_una_capa_a_donde_ya_esta_o_fuera_de_rango_no_hace_nada() {
        let mut p = proyecto_con_capas_y_origenes();
        let antes = p.clone();
        assert!(!p.mover_capa(1, 1));
        assert!(!p.mover_capa(9, 0));
        assert!(!p.mover_capa(0, 9));
        assert_eq!(p, antes);
    }

    #[test]
    fn quitar_la_ultima_capa_deja_sin_origen_a_las_que_la_usaban() {
        let mut p = proyecto_con_capas_y_origenes();
        assert!(p.quitar_capa(3));
        assert_eq!(origenes(&p)[2], OrigenSuperficie::Composicion);
        assert_eq!(origenes(&p)[0], OrigenSuperficie::Capa(0));
    }

    #[test]
    fn quitar_la_primera_capa_renumera_todas_las_demas() {
        let mut p = proyecto_con_capas_y_origenes();
        assert!(p.quitar_capa(0));
        assert_eq!(
            origenes(&p),
            vec![
                OrigenSuperficie::Composicion,
                OrigenSuperficie::Capa(0),
                OrigenSuperficie::Capa(2),
                OrigenSuperficie::Composicion,
            ]
        );
    }

    #[test]
    fn la_unica_capa_no_se_puede_quitar_ni_una_que_no_existe() {
        let mut p = Proyecto::default();
        assert!(!p.quitar_capa(0));
        let mut p = proyecto_con_capas_y_origenes();
        assert!(!p.quitar_capa(9));
        assert_eq!(p.composicion.capas.len(), 4);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_9() {
        let viejo = r#"Proyecto(version: 9, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas[0].superficies[0].malla, None);
    }

    #[test]
    fn una_malla_invalida_de_un_archivo_se_descarta_y_una_valida_se_arregla() {
        // Cantidad de puntos que no coincide con la grilla: se descarta.
        let mala = r#"Proyecto(version: 10, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             malla: Some((columnas: 3, filas: 3, desplazamientos: [(0.0, 0.0)])))])])"#;
        let p = Proyecto::desde_texto(mala).unwrap();
        assert_eq!(p.salidas[0].superficies[0].malla, None);

        // Válida pero con un valor en una esquina: se pone en cero.
        let buena = r#"Proyecto(version: 10, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             malla: Some((columnas: 2, filas: 2, desplazamientos: [(9.0, 9.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)])))])])"#;
        let p = Proyecto::desde_texto(buena).unwrap();
        let malla = p.salidas[0].superficies[0].malla.as_ref().unwrap();
        assert_eq!(malla.desplazamientos[0], [0.0, 0.0]);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_8() {
        let viejo = r#"Proyecto(version: 8, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas[0].superficies[0].borde, BordeSuave::default());
    }

    #[test]
    fn sin_borde_no_se_atenua_nada() {
        let b = BordeSuave::default();
        assert!(!b.activo());
        for (u, v) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (0.01, 0.99)] {
            assert_eq!(b.atenuacion(u, v), 1.0);
        }
    }

    #[test]
    fn la_rampa_va_de_cero_en_el_borde_a_uno_al_terminar_el_ancho() {
        let b = BordeSuave {
            izquierda: 0.2,
            ..Default::default()
        };
        assert!(b.activo());
        assert_eq!(b.atenuacion(0.0, 0.5), 0.0);
        assert!((b.atenuacion(0.1, 0.5) - 0.5).abs() < 1e-6); // la mitad del ancho: 0,5
        assert_eq!(b.atenuacion(0.2, 0.5), 1.0);
        assert_eq!(b.atenuacion(0.7, 0.5), 1.0);
    }

    #[test]
    fn la_luz_nunca_baja_al_alejarse_del_borde() {
        let b = BordeSuave {
            derecha: 0.3,
            ..Default::default()
        };
        let mut anterior = -1.0_f32;
        // Desde el borde derecho (u = 1) hacia adentro: la luz sube de 0 a 1 sin bajar.
        for i in 0..=40 {
            let u = 1.0 - i as f32 / 100.0;
            let luz = b.atenuacion(u, 0.5);
            assert!(luz >= anterior - 1e-6, "baja en u={u}: {anterior} -> {luz}");
            anterior = luz;
        }
        assert_eq!(anterior, 1.0);
    }

    #[test]
    fn dos_proyectores_con_rampas_espejadas_suman_luz_constante() {
        // Proyector A: se apaga hacia su derecha. Proyector B: se enciende desde su
        // izquierda. En la zona de solape (ancho `s` de cada uno) las rampas son espejadas.
        let a = BordeSuave {
            derecha: 0.25,
            ..Default::default()
        };
        let b = BordeSuave {
            izquierda: 0.25,
            ..Default::default()
        };
        for i in 0..=100 {
            let t = i as f32 / 100.0; // posición dentro del solape: 0 a 1
            // En A, el solape ocupa u de 0,75 a 1; en B, de 0 a 0,25.
            let luz_a = a.atenuacion(0.75 + 0.25 * t, 0.5);
            let luz_b = b.atenuacion(0.25 * t, 0.5);
            assert!(
                (luz_a + luz_b - 1.0).abs() < 1e-5,
                "t={t}: A={luz_a} B={luz_b} suman {}",
                luz_a + luz_b
            );
        }
    }

    #[test]
    fn los_bordes_se_combinan_en_las_esquinas() {
        let b = BordeSuave {
            izquierda: 0.2,
            arriba: 0.2,
            ..Default::default()
        };
        let en_la_esquina = b.atenuacion(0.1, 0.1);
        assert!((en_la_esquina - 0.25).abs() < 1e-6); // 0,5 × 0,5
        assert_eq!(b.atenuacion(0.0, 0.5), 0.0);
    }

    #[test]
    fn la_potencia_ajusta_la_curva() {
        let neutra = BordeSuave {
            izquierda: 0.2,
            ..Default::default()
        };
        let oscura = BordeSuave {
            potencia: 2.0,
            ..neutra
        };
        // A mitad de la rampa: 0,5 con potencia 1, y 0,25 con potencia 2.
        assert!((neutra.atenuacion(0.1, 0.5) - 0.5).abs() < 1e-6);
        assert!((oscura.atenuacion(0.1, 0.5) - 0.25).abs() < 1e-6);
        // En los extremos no cambia nada.
        assert_eq!(oscura.atenuacion(0.0, 0.5), 0.0);
        assert_eq!(oscura.atenuacion(0.5, 0.5), 1.0);
    }

    #[test]
    fn normalizar_limita_los_anchos_y_la_potencia() {
        let mut b = BordeSuave {
            izquierda: 2.0,
            derecha: -1.0,
            arriba: f32::NAN,
            abajo: 0.3,
            potencia: 99.0,
        };
        b.normalizar();
        assert_eq!(b.izquierda, BORDE_MAXIMO);
        assert_eq!(b.derecha, 0.0);
        assert_eq!(b.arriba, 0.0);
        assert_eq!(b.abajo, 0.3);
        assert_eq!(b.potencia, POTENCIA_MAXIMA);
    }

    fn salida_con_tres() -> ConfigSalida {
        let mut s = ConfigSalida::nueva("DP-1".into(), 1920.0, 1080.0);
        s.superficies[0].nombre = "A".into();
        s.agregar_superficie(1920.0, 1080.0);
        s.agregar_superficie(1920.0, 1080.0);
        s.superficies[1].nombre = "B".into();
        s.superficies[2].nombre = "C".into();
        s
    }

    fn nombres(s: &ConfigSalida) -> Vec<&str> {
        s.superficies.iter().map(|x| x.nombre.as_str()).collect()
    }

    #[test]
    fn agregar_una_superficie_la_pone_al_final_con_nombre_y_mas_chica() {
        let mut s = ConfigSalida::nueva("DP-1".into(), 1920.0, 1080.0);
        let i = s.agregar_superficie(1920.0, 1080.0);
        assert_eq!(i, 1);
        assert_eq!(s.superficies.len(), 2);
        assert_eq!(s.superficies[1].nombre, "Superficie 2");
        assert!(s.superficies[1].visible);
        // El 40 % de 1920 de ancho: de -384 a 384.
        assert!((s.superficies[1].esquinas[1][0] - 384.0).abs() < 1e-3);
    }

    #[test]
    fn duplicar_copia_la_superficie_corrida_y_justo_despues() {
        let mut s = salida_con_tres();
        let original = s.superficies[1].clone();
        let i = s.duplicar_superficie(1).unwrap();
        assert_eq!(i, 2);
        assert_eq!(nombres(&s), ["A", "B", "B (copia)", "C"]);
        let copia = &s.superficies[2];
        assert_eq!(copia.entrada, original.entrada);
        assert!((copia.esquinas[0][0] - (original.esquinas[0][0] + 40.0)).abs() < 1e-3);
        assert!((copia.esquinas[0][1] - (original.esquinas[0][1] - 40.0)).abs() < 1e-3);
    }

    #[test]
    fn duplicar_una_superficie_sin_nombre_usa_el_nombre_por_defecto() {
        let mut s = ConfigSalida::nueva("DP-1".into(), 100.0, 100.0);
        s.duplicar_superficie(0);
        assert_eq!(s.superficies[1].nombre, "Superficie 1 (copia)");
    }

    #[test]
    fn duplicar_una_que_no_existe_no_hace_nada() {
        let mut s = salida_con_tres();
        assert_eq!(s.duplicar_superficie(9), None);
        assert_eq!(s.superficies.len(), 3);
    }

    #[test]
    fn borrar_quita_la_superficie_pero_siempre_queda_una() {
        let mut s = salida_con_tres();
        assert!(s.borrar_superficie(1));
        assert_eq!(nombres(&s), ["A", "C"]);
        assert!(s.borrar_superficie(0));
        // La última no se puede borrar.
        assert!(!s.borrar_superficie(0));
        assert_eq!(nombres(&s), ["C"]);
        assert!(!s.borrar_superficie(5));
    }

    #[test]
    fn mover_una_superficie_la_intercambia_con_su_vecina() {
        let mut s = salida_con_tres();
        assert_eq!(s.mover_superficie(0, 1), Some(1));
        assert_eq!(nombres(&s), ["B", "A", "C"]);
        assert_eq!(s.mover_superficie(2, -1), Some(1));
        assert_eq!(nombres(&s), ["B", "C", "A"]);
    }

    #[test]
    fn mover_mas_alla_de_los_extremos_no_hace_nada() {
        let mut s = salida_con_tres();
        assert_eq!(s.mover_superficie(0, -1), None);
        assert_eq!(s.mover_superficie(2, 1), None);
        assert_eq!(s.mover_superficie(9, 1), None);
        assert_eq!(nombres(&s), ["A", "B", "C"]);
    }

    #[test]
    fn migra_un_proyecto_de_la_version_3() {
        // La versión 3 ya tenía `entrada`, pero no la composición.
        let viejo = r#"Proyecto(version: 3, salidas: [(monitor: "DP-1", activa: true, superficies: [
            (esquinas: ((-10.0, 5.0), (10.0, 5.0), (10.0, -5.0), (-10.0, -5.0)),
             entrada: (0.1, 0.2, 0.9, 0.8))])])"#;
        let p = Proyecto::desde_texto(viejo).unwrap();
        assert_eq!(p.version, VERSION_ACTUAL);
        assert_eq!(p.salidas[0].superficies[0].entrada, [0.1, 0.2, 0.9, 0.8]);
        assert_eq!(p.composicion, Composicion::default());
    }

    #[test]
    fn al_leer_se_reparan_las_celdas_que_faltan() {
        // Una capa con una sola celda aunque hay cuatro columnas, y una resolución absurda.
        let texto = r#"Proyecto(version: 4, composicion: (ancho: 5, alto: 1080,
            columnas: [(nombre: "1"), (nombre: "2"), (nombre: "3")],
            capas: [(nombre: "Capa 1", celdas: [None])]), salidas: [])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        assert_eq!(p.composicion.ancho, RESOLUCION_MINIMA);
        assert_eq!(p.composicion.capas[0].celdas.len(), 3);
    }

    #[test]
    fn normalizar_deja_la_entrada_ordenada_y_dentro_de_0_a_1() {
        // Esquinas invertidas y fuera de rango.
        let r = normalizar_entrada([1.4, 0.9, -0.2, 0.1]);
        assert_eq!(r, [0.0, 0.1, 1.0, 0.9]);
    }

    #[test]
    fn normalizar_garantiza_un_tamano_minimo() {
        let r = normalizar_entrada([0.5, 0.2, 0.5, 0.8]);
        assert!((r[2] - r[0] - ENTRADA_TAMANO_MINIMO).abs() < 1e-6);
        // Pegado al borde derecho: no puede salirse de 1.
        let borde = normalizar_entrada([1.0, 0.0, 1.0, 1.0]);
        assert!((borde[2] - 1.0).abs() < 1e-6);
        assert!((borde[2] - borde[0] - ENTRADA_TAMANO_MINIMO).abs() < 1e-6);
    }

    #[test]
    fn una_entrada_valida_no_se_modifica() {
        let r = [0.1, 0.2, 0.6, 0.9];
        assert_eq!(normalizar_entrada(r), r);
    }

    #[test]
    fn rechaza_texto_invalido() {
        assert!(matches!(
            Proyecto::desde_texto("esto no es un proyecto"),
            Err(ErrorProyecto::Formato(_))
        ));
    }

    #[test]
    fn archivo_inexistente_da_error_de_archivo() {
        assert!(matches!(
            Proyecto::cargar("/ruta/que/no/existe.ron"),
            Err(ErrorProyecto::Archivo(_))
        ));
    }

    #[test]
    fn rectangulo_centrado_es_simetrico() {
        let s = ConfigSuperficie::rectangulo_centrado(200.0, 100.0, 0.5);
        let tol = 1e-6;
        assert!((s.esquinas[0][0] + 50.0).abs() < tol); // sup-izq x
        assert!((s.esquinas[0][1] - 25.0).abs() < tol); // sup-izq y
        assert!((s.esquinas[2][0] - 50.0).abs() < tol); // inf-der x
        assert!((s.esquinas[2][1] + 25.0).abs() < tol); // inf-der y
    }

    #[test]
    fn copiar_una_superficie_a_otra_salida_la_reescala() {
        let mut p = Proyecto::default();
        p.salidas
            .push(ConfigSalida::nueva("A".into(), 1920.0, 1080.0));
        p.salidas
            .push(ConfigSalida::nueva("B".into(), 3840.0, 2160.0));
        let original = p.salidas[0].superficies[0].esquinas;
        let nuevo = p.copiar_superficie("A", 0, "B", [2.0, 2.0]).unwrap();
        assert_eq!(nuevo, 1); // B ya tenía una
        let copia = &p.salidas[1].superficies[nuevo];
        for (c, o) in copia.esquinas.iter().zip(original) {
            assert_eq!(*c, [o[0] * 2.0, o[1] * 2.0]);
        }
        assert!(copia.nombre.contains("(A)"));
        // La original no se toca.
        assert_eq!(p.salidas[0].superficies[0].esquinas, original);
        assert_eq!(p.salidas[0].superficies.len(), 1);
    }

    #[test]
    fn copiar_reescala_tambien_la_malla() {
        let mut p = Proyecto::default();
        p.salidas
            .push(ConfigSalida::nueva("A".into(), 1000.0, 1000.0));
        p.salidas
            .push(ConfigSalida::nueva("B".into(), 2000.0, 2000.0));
        let mut malla = Malla::nueva(3, 3);
        malla.poner(1, 1, [10.0, -4.0]);
        p.salidas[0].superficies[0].malla = Some(malla);
        let nuevo = p.copiar_superficie("A", 0, "B", [2.0, 3.0]).unwrap();
        let m = p.salidas[1].superficies[nuevo].malla.as_ref().unwrap();
        assert_eq!(m.desplazamiento(1, 1), [20.0, -12.0]);
    }

    #[test]
    fn copiar_con_salida_o_superficie_inexistente_no_hace_nada() {
        let mut p = Proyecto::default();
        p.salidas
            .push(ConfigSalida::nueva("A".into(), 1920.0, 1080.0));
        assert_eq!(p.copiar_superficie("A", 5, "A", [1.0, 1.0]), None);
        assert_eq!(p.copiar_superficie("Z", 0, "A", [1.0, 1.0]), None);
        assert_eq!(p.copiar_superficie("A", 0, "Z", [1.0, 1.0]), None);
        assert_eq!(p.salidas[0].superficies.len(), 1);
    }

    #[test]
    fn las_mascaras_invalidas_se_descartan_al_leer() {
        let mut s = ConfigSalida::nueva("A".into(), 1920.0, 1080.0);
        s.mascaras = vec![
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], // válida
            vec![[0.0, 0.0], [1.0, 0.0]],             // pocos puntos
            vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]], // lados cruzados
            vec![[f32::NAN, 0.0], [1.0, 0.0], [1.0, 1.0]], // número inválido
        ];
        s.normalizar_mascaras();
        assert_eq!(s.mascaras.len(), 1);
    }

    #[test]
    fn agregar_una_mascara_la_deja_centrada_y_valida() {
        let mut s = ConfigSalida::nueva("A".into(), 1920.0, 1080.0);
        let i = s.agregar_mascara(1920.0, 1080.0, 0.5);
        assert_eq!(i, 0);
        assert_eq!(s.mascaras[0][0], [-480.0, 270.0]);
        assert!(es_simple(&s.mascaras[0]));
    }

    #[test]
    fn un_proyecto_viejo_sin_mascaras_se_lee_sin_ellas() {
        let texto = r#"Proyecto(version: 12, composicion: (ancho: 1920, alto: 1080, columnas: [(nombre: "1")], capas: []),
            salidas: [(monitor: "DP-1", superficies: [])])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        assert!(p.salidas[0].mascaras.is_empty());
    }

    #[test]
    fn un_hueco_nuevo_esta_en_el_centro_y_es_valido() {
        let mut s = ConfigSuperficie::rectangulo_centrado(1920.0, 1080.0, 0.5);
        let i = s.agregar_hueco();
        assert_eq!(i, 0);
        assert!(es_simple(&s.huecos[0]));
        let centro = centroide(&s.huecos[0]);
        assert!((centro[0] - 0.5).abs() < 1e-5 && (centro[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn normalizar_descarta_los_huecos_invalidos_y_limita_los_demas() {
        let mut s = ConfigSuperficie::rectangulo_centrado(100.0, 100.0, 1.0);
        s.huecos = vec![
            vec![[-0.5, 0.2], [0.5, 0.2], [0.5, 0.8]], // se limita a 0..1
            vec![[0.0, 0.0], [1.0, 1.0]],              // pocos puntos
            vec![[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]], // lados cruzados
        ];
        s.normalizar();
        assert_eq!(s.huecos.len(), 1);
        assert_eq!(s.huecos[0][0], [0.0, 0.2]);
    }

    #[test]
    fn un_proyecto_viejo_sin_huecos_se_lee_sin_ellos() {
        let texto = r#"Proyecto(version: 13, composicion: (ancho: 1920, alto: 1080, columnas: [(nombre: "1")], capas: []),
            salidas: [(monitor: "DP-1", superficies: [(esquinas: ((0.0,0.0),(1.0,0.0),(1.0,1.0),(0.0,1.0)))])])"#;
        let p = Proyecto::desde_texto(texto).unwrap();
        assert!(p.salidas[0].superficies[0].huecos.is_empty());
    }

    #[test]
    fn un_origen_con_la_capa_sin_nombre_dice_capa_n() {
        let mut comp = Composicion::default();
        comp.capas[0].nombre = "  ".into();
        assert_eq!(OrigenSuperficie::Capa(0).nombre(&comp), "Capa 1");
    }

    #[test]
    fn las_superficies_siempre_tienen_nombre_propio_y_distinto() {
        let mut s = ConfigSalida::nueva("DP-1".into(), 1920.0, 1080.0);
        assert_eq!(s.superficies[0].nombre, "Superficie 1");
        s.agregar_superficie(1920.0, 1080.0);
        s.agregar_superficie(1920.0, 1080.0);
        assert_eq!(
            nombres(&s),
            ["Superficie 1", "Superficie 2", "Superficie 3"]
        );
        // Se borra la del medio y se agrega otra: usa el hueco, sin repetir ninguno.
        s.borrar_superficie(1);
        s.agregar_superficie(1920.0, 1080.0);
        assert_eq!(
            nombres(&s),
            ["Superficie 1", "Superficie 3", "Superficie 2"]
        );
    }

    #[test]
    fn reordenar_no_cambia_los_nombres_de_las_superficies() {
        let mut s = ConfigSalida::nueva("DP-1".into(), 1920.0, 1080.0);
        s.agregar_superficie(1920.0, 1080.0);
        s.agregar_superficie(1920.0, 1080.0);
        let antes: std::collections::HashSet<String> =
            s.superficies.iter().map(|x| x.nombre.clone()).collect();
        s.mover_superficie(0, 1);
        s.mover_superficie(2, -1);
        // Los mismos nombres, cada uno una sola vez.
        let despues: std::collections::HashSet<String> =
            s.superficies.iter().map(|x| x.nombre.clone()).collect();
        assert_eq!(antes, despues);
        assert_eq!(s.superficies.len(), 3);
    }

    #[test]
    fn un_proyecto_viejo_sin_nombres_los_recibe_al_cargarlo() {
        let mut salida = ConfigSalida::nueva("DP-1".into(), 100.0, 100.0);
        salida.agregar_superficie(100.0, 100.0);
        // Como los de antes: sin nombre propio.
        salida.superficies[0].nombre = String::new();
        salida.superficies[1].nombre = "Fachada".into();
        let mut p = Proyecto::default();
        p.salidas.push(salida);
        let leido = Proyecto::desde_texto(&p.a_texto().unwrap()).unwrap();
        assert_eq!(
            nombres(&leido.salidas[0]),
            ["Superficie 1", "Fachada"],
            "el vacío recibe el primer nombre libre"
        );
    }
}
