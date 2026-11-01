//! La composición: la grilla de capas × columnas de clips, y la lógica de disparo.
//! Rust puro: no depende de Bevy ni de GStreamer.
//!
//! Vocabulario (ver `docs/04_plan_composicion_y_superficies.md`):
//! - **Clip**: una fuente de imagen (hoy, un archivo de video).
//! - **Capa**: una fila de la grilla. Muestra **un clip a la vez**.
//! - **Columna**: una "escena": disparar una columna dispara el clip de esa columna en cada capa.
//!
//! Lo que se **guarda** es qué hay en cada celda (`Composicion`). Qué clip está sonando en
//! cada capa es estado de ejecución (`EstadoReproduccion`) y no va en el archivo.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modulacion::Modulacion;

pub const ANCHO_POR_DEFECTO: u32 = 1920;
pub const ALTO_POR_DEFECTO: u32 = 1080;
/// Límites de la resolución de la composición. Una textura de 8192×8192 ya ocupa 256 MB.
pub const RESOLUCION_MINIMA: u32 = 64;
pub const RESOLUCION_MAXIMA: u32 = 8192;

/// Deja una dimensión de la composición dentro de los límites permitidos.
pub fn limitar_resolucion(valor: u32) -> u32 {
    valor.clamp(RESOLUCION_MINIMA, RESOLUCION_MAXIMA)
}

/// De dónde sale la imagen de un clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fuente {
    /// Un archivo de video. La ruta es relativa a la carpeta del proyecto si el archivo
    /// está dentro de ella; si no, absoluta (ver `ruta_para_guardar`).
    Video { ruta: String },
    /// Una imagen fija (PNG, JPG o WebP). La ruta se guarda igual que la de un video. Se
    /// decodifica una vez y se queda quieta: sin duración, bucle, velocidad ni tramo.
    Imagen { ruta: String },
    /// Una cámara o un dispositivo de captura (V4L2), por ejemplo `/dev/video0`. Una cámara
    /// virtual (como la de OBS) sirve para traer la salida de otro programa.
    Camara { dispositivo: String },
    /// Una transmisión de red: `rtsp://…`, `srt://…`, `http://…`.
    Url { url: String },
    /// La captura de una pantalla o ventana, por el portal del escritorio. El `token` es lo
    /// que el portal da para volver a abrir la misma fuente sin preguntar (`None` la primera
    /// vez: el escritorio muestra su selector). `audio` dice de dónde sale el sonido: el
    /// portal solo entrega imagen, así que el sonido se toma aparte.
    Captura {
        token: Option<String>,
        #[serde(default)]
        audio: AudioCaptura,
        /// No dibujar el cursor del mouse dentro de la imagen. Con el cursor adentro, una
        /// ventana que no se repinta casi no manda cuadros y el puntero se ve a saltos; en un
        /// mapping casi nunca se quiere en la proyección. Los proyectos viejos no lo traen y
        /// siguen mostrándolo.
        #[serde(default)]
        ocultar_cursor: bool,
    },
}

/// De dónde sale el sonido de una captura de pantalla o ventana.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCaptura {
    /// Sin sonido.
    #[default]
    Ninguno,
    /// Todo lo que suena en el escritorio. Incluye el sonido del propio programa: si el clip
    /// no está silenciado se puede armar un eco.
    Escritorio,
    /// Lo que suena una aplicación, elegida por su nombre (`application.name` de PipeWire).
    /// No incluye el sonido del programa.
    Aplicacion(String),
}

impl AudioCaptura {
    /// Cómo se muestra en la interfaz.
    pub fn nombre(&self) -> String {
        match self {
            AudioCaptura::Ninguno => "Sin audio".into(),
            AudioCaptura::Escritorio => "Todo el escritorio".into(),
            AudioCaptura::Aplicacion(nombre) => nombre.clone(),
        }
    }
}

impl Fuente {
    /// La ruta del archivo, si es un video o una imagen de archivo.
    pub fn ruta_de_archivo(&self) -> Option<&str> {
        match self {
            Fuente::Video { ruta } | Fuente::Imagen { ruta } => Some(ruta),
            _ => None,
        }
    }

    /// ¿Es una imagen en vivo (cámara, transmisión, captura: sin duración, sin principio ni
    /// fin)? Una imagen fija de archivo NO lo es: ver `es_imagen`.
    pub fn es_en_vivo(&self) -> bool {
        matches!(
            self,
            Fuente::Camara { .. } | Fuente::Url { .. } | Fuente::Captura { .. }
        )
    }

    /// ¿Es una imagen fija (un archivo PNG, JPG o WebP)?
    pub fn es_imagen(&self) -> bool {
        matches!(self, Fuente::Imagen { .. })
    }

    /// Una descripción corta para mostrar: «Cámara /dev/video0», la URL, o la ruta.
    pub fn descripcion(&self) -> String {
        match self {
            Fuente::Video { ruta } | Fuente::Imagen { ruta } => ruta.clone(),
            Fuente::Camara { dispositivo } => format!("Cámara {dispositivo}"),
            Fuente::Url { url } => url.clone(),
            Fuente::Captura { .. } => "Captura de pantalla o ventana".into(),
        }
    }
}

/// Las extensiones de imagen fija que se aceptan (en minúsculas): las que GStreamer abre en
/// esta máquina. BMP, TIFF y TGA no se pudieron abrir, así que no se ofrecen.
pub const EXTENSIONES_DE_IMAGEN: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// ¿La ruta es de una imagen fija, por su extensión (sin distinguir mayúsculas)?
pub fn es_ruta_de_imagen(ruta: impl AsRef<Path>) -> bool {
    ruta.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES_DE_IMAGEN.contains(&e.to_ascii_lowercase().as_str()))
}

/// Límites de los ajustes de reproducción de un clip.
pub const VELOCIDAD_MINIMA: f32 = 0.1;
pub const VELOCIDAD_MAXIMA: f32 = 4.0;
pub const VOLUMEN_MAXIMO: f32 = 2.0;
/// El fundido más largo que se puede pedir, en segundos.
pub const TRANSICION_MAXIMA: f32 = 5.0;

/// Las opacidades con que se dibujan los dos clips de una capa durante un fundido:
/// `(saliente, entrante)`. `opacidad` es la de la capa y `avance` el de `avance_de_transicion`.
///
/// En modo **Normal** el saliente se dibuja primero y el entrante encima, y las cuentas se
/// eligen para que el resultado sea exactamente `fondo·(1−op) + op·(avance·nuevo +
/// (1−avance)·viejo)`. Con una capa opaca eso da «el viejo entero, y encima el nuevo que
/// sube» (no hay bajón de luz a mitad del fundido); con una capa semitransparente, bajar
/// uno y subir el otro a ojo dejaría un salto al terminar, cuando se quita el viejo.
/// En los demás modos (suma, multiplicar, pantalla) cada clip aporta su parte proporcional.
pub fn opacidades_de_transicion(opacidad: f32, avance: f32, modo: ModoMezcla) -> (f32, f32) {
    let entrante = opacidad * avance;
    let saliente = if modo == ModoMezcla::Normal {
        let resto = 1.0 - entrante;
        if resto <= 1e-6 {
            0.0
        } else {
            opacidad * (1.0 - avance) / resto
        }
    } else {
        opacidad * (1.0 - avance)
    };
    (saliente, entrante)
}

/// Cuánto del clip nuevo se ve (de 0 a 1) cuando pasaron `transcurrido` segundos de un
/// fundido que dura `duracion`. Sube en línea recta, y con duración 0 (o negativa) es un
/// corte: ya se ve entero. Es lógica pura: la ejecución solo le pasa el tiempo.
pub fn avance_de_transicion(transcurrido: f32, duracion: f32) -> f32 {
    if duracion <= 0.0 || !duracion.is_finite() {
        return 1.0;
    }
    (transcurrido / duracion).clamp(0.0, 1.0)
}
/// Lo mínimo que tiene que separar la entrada de la salida (fracción de la duración).
pub const SEPARACION_MINIMA: f32 = 0.01;

/// Cómo se reproduce un clip: bucle, velocidad, tramo (entrada y salida) y audio.
/// `#[serde(default)]`: un proyecto viejo, sin estos ajustes, se lee con los de siempre.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AjustesClip {
    /// Al llegar al final vuelve a empezar. Si no, queda quieto en el último cuadro.
    pub bucle: bool,
    /// 1 es la velocidad normal.
    pub velocidad: f32,
    /// Dónde empieza y dónde termina el tramo que se reproduce, como fracción (0 a 1) de la
    /// duración del video.
    pub inicio: f32,
    pub fin: f32,
    /// 1 es el volumen original.
    pub volumen: f32,
    pub silenciado: bool,
}

impl Default for AjustesClip {
    fn default() -> Self {
        AjustesClip {
            bucle: true,
            velocidad: 1.0,
            inicio: 0.0,
            fin: 1.0,
            volumen: 1.0,
            silenciado: false,
        }
    }
}

impl AjustesClip {
    /// Deja los ajustes dentro de sus límites (un archivo editado a mano puede traer
    /// cualquier cosa): lo que no sea un número vuelve a su valor normal, y el tramo queda
    /// ordenado y con un largo mínimo.
    pub fn normalizar(&mut self) {
        let normal = AjustesClip::default();
        let limitar = |v: f32, min: f32, max: f32, neutro: f32| {
            if v.is_finite() {
                v.clamp(min, max)
            } else {
                neutro
            }
        };
        self.velocidad = limitar(
            self.velocidad,
            VELOCIDAD_MINIMA,
            VELOCIDAD_MAXIMA,
            normal.velocidad,
        );
        self.volumen = limitar(self.volumen, 0.0, VOLUMEN_MAXIMO, normal.volumen);
        let mut inicio = limitar(self.inicio, 0.0, 1.0, 0.0);
        let mut fin = limitar(self.fin, 0.0, 1.0, 1.0);
        if inicio > fin {
            std::mem::swap(&mut inicio, &mut fin);
        }
        if fin - inicio < SEPARACION_MINIMA {
            // Se agranda hacia el lado que tenga lugar.
            if inicio + SEPARACION_MINIMA <= 1.0 {
                fin = inicio + SEPARACION_MINIMA;
            } else {
                inicio = fin - SEPARACION_MINIMA;
            }
        }
        self.inicio = inicio;
        self.fin = fin;
    }

    /// ¿El tramo es el video entero?
    pub fn tramo_completo(&self) -> bool {
        self.inicio <= 0.0 && self.fin >= 1.0
    }
}

/// Un clip: lo que hay en una celda de la grilla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigClip {
    pub nombre: String,
    pub fuente: Fuente,
    /// Cómo se reproduce (bucle, velocidad, tramo, audio).
    #[serde(default)]
    pub ajustes: AjustesClip,
}

impl ConfigClip {
    /// Un clip que toma la imagen de una cámara o dispositivo de captura (V4L2).
    pub fn de_camara(dispositivo: String, nombre: String) -> Self {
        ConfigClip {
            nombre,
            fuente: Fuente::Camara { dispositivo },
            ajustes: AjustesClip::default(),
        }
    }

    /// Un clip que captura una pantalla o ventana (la primera vez, el escritorio pregunta cuál).
    pub fn de_captura(nombre: String) -> Self {
        ConfigClip {
            nombre,
            fuente: Fuente::Captura {
                token: None,
                audio: AudioCaptura::Ninguno,
                // Una captura nueva arranca sin cursor (ver `Fuente::Captura`).
                ocultar_cursor: true,
            },
            ajustes: AjustesClip::default(),
        }
    }

    /// Un clip que toma la imagen de una transmisión de red. El nombre es la parte de la URL
    /// que identifica el servidor (sin el protocolo).
    pub fn de_url(url: String) -> Self {
        let nombre = url
            .split_once("://")
            .map_or(url.as_str(), |(_, resto)| resto)
            .split(['/', '?'])
            .next()
            .unwrap_or(&url)
            .to_string();
        ConfigClip {
            nombre,
            fuente: Fuente::Url { url },
            ajustes: AjustesClip::default(),
        }
    }

    /// Un clip de imagen fija con el nombre del archivo (sin extensión).
    pub fn de_imagen(ruta_guardada: String) -> Self {
        let mut clip = ConfigClip::de_video(ruta_guardada);
        if let Fuente::Video { ruta } = clip.fuente {
            clip.fuente = Fuente::Imagen { ruta };
        }
        clip
    }

    /// Un clip de archivo: imagen si la extensión es de imagen, y video en cualquier otro
    /// caso.
    pub fn de_archivo(ruta_guardada: String) -> Self {
        if es_ruta_de_imagen(&ruta_guardada) {
            ConfigClip::de_imagen(ruta_guardada)
        } else {
            ConfigClip::de_video(ruta_guardada)
        }
    }

    /// Un clip de video con el nombre del archivo (sin extensión).
    pub fn de_video(ruta_guardada: String) -> Self {
        let nombre = Path::new(&ruta_guardada)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ruta_guardada.clone());
        ConfigClip {
            nombre,
            fuente: Fuente::Video {
                ruta: ruta_guardada,
            },
            ajustes: AjustesClip::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Columna {
    pub nombre: String,
}

/// Cómo se combina una capa con lo que hay **debajo** de ella en la composición.
///
/// Los cuatro modos se pueden lograr con la función de mezcla fija de la GPU (sin leer
/// lo ya dibujado), que es rápida y no gasta memoria. `S` es el color de la capa y `D` el
/// de lo que hay debajo; en todos, la opacidad de la capa se pliega en el color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ModoMezcla {
    /// La capa tapa lo de abajo según su opacidad: `D·(1−a) + S·a`.
    #[default]
    Normal,
    /// Suma la luz (aclara, como dos proyectores superpuestos): `D + S·a`.
    Suma,
    /// Oscurece: lo negro de una capa tapa a la otra: `D·(1 − a + a·S)`.
    Multiplicar,
    /// Aclara sin quemar los blancos: `D + S·a − D·S·a`.
    Pantalla,
}

impl ModoMezcla {
    /// Todos los modos, en el orden en que se ofrecen en la interfaz.
    pub const TODOS: [ModoMezcla; 4] = [
        ModoMezcla::Normal,
        ModoMezcla::Suma,
        ModoMezcla::Multiplicar,
        ModoMezcla::Pantalla,
    ];

    pub fn nombre(&self) -> &'static str {
        match self {
            ModoMezcla::Normal => "Normal",
            ModoMezcla::Suma => "Suma",
            ModoMezcla::Multiplicar => "Multiplicar",
            ModoMezcla::Pantalla => "Pantalla",
        }
    }

    /// El número con el que el shader reconoce el modo.
    pub fn numero(&self) -> u32 {
        match self {
            ModoMezcla::Normal => 0,
            ModoMezcla::Suma => 1,
            ModoMezcla::Multiplicar => 2,
            ModoMezcla::Pantalla => 3,
        }
    }
}

/// Cómo se acomoda el video de una capa dentro de la composición cuando no tienen la
/// misma proporción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Ajuste {
    /// Se ve entero, con bandas (transparentes) en los lados que sobran.
    #[default]
    Ajustar,
    /// Llena toda la composición; lo que sobra se recorta.
    Llenar,
    /// Se estira a la proporción de la composición (deforma la imagen).
    Estirar,
}

impl Ajuste {
    pub const TODOS: [Ajuste; 3] = [Ajuste::Ajustar, Ajuste::Llenar, Ajuste::Estirar];

    pub fn nombre(&self) -> &'static str {
        match self {
            Ajuste::Ajustar => "Ajustar (con bandas)",
            Ajuste::Llenar => "Llenar (recortando)",
            Ajuste::Estirar => "Estirar",
        }
    }
}

/// El tamaño en píxeles con el que se dibuja un video dentro de la composición, según el
/// ajuste. `composicion` y `video` son `(ancho, alto)`. El video se dibuja centrado, y con
/// `Llenar` el resultado es más grande que la composición (el borde sobrante se recorta).
pub fn tamano_de_ajuste(ajuste: Ajuste, composicion: (f32, f32), video: (f32, f32)) -> (f32, f32) {
    let ((cw, ch), (vw, vh)) = (composicion, video);
    if vw <= 0.0 || vh <= 0.0 {
        return composicion;
    }
    match ajuste {
        Ajuste::Estirar => composicion,
        Ajuste::Ajustar => {
            let escala = (cw / vw).min(ch / vh);
            (vw * escala, vh * escala)
        }
        Ajuste::Llenar => {
            let escala = (cw / vw).max(ch / vh);
            (vw * escala, vh * escala)
        }
    }
}

fn opacidad_completa() -> f32 {
    1.0
}

/// Una fila de la grilla. `celdas` tiene **una entrada por columna**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capa {
    pub nombre: String,
    pub celdas: Vec<Option<ConfigClip>>,
    /// Si se dibuja. Una capa oculta sigue existiendo y su clip sigue sonando.
    #[serde(default = "crate::verdadero")]
    pub visible: bool,
    /// De 0 (invisible) a 1 (opaca).
    #[serde(default = "opacidad_completa")]
    pub opacidad: f32,
    #[serde(default)]
    pub modo: ModoMezcla,
    #[serde(default)]
    pub ajuste: Ajuste,
    /// Si la opacidad se mueve con una señal (el sonido, por ejemplo). `opacidad` es la base.
    #[serde(default)]
    pub modulacion_de_opacidad: Option<Modulacion>,
}

impl Capa {
    /// Una capa vacía con una celda por columna, visible, opaca y en modo normal.
    pub fn nueva(nombre: String, columnas: usize) -> Self {
        Capa {
            nombre,
            celdas: vec![None; columnas],
            visible: true,
            opacidad: 1.0,
            modo: ModoMezcla::Normal,
            ajuste: Ajuste::Ajustar,
            modulacion_de_opacidad: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composicion {
    /// Resolución de la textura donde se mezcla todo, elegida por proyecto.
    pub ancho: u32,
    pub alto: u32,
    pub columnas: Vec<Columna>,
    pub capas: Vec<Capa>,
    /// Segundos que dura el fundido al cambiar de clip en una capa. `0` es un corte seco
    /// (como siempre). Va de 0 a `TRANSICION_MAXIMA`.
    #[serde(default)]
    pub transicion: f32,
}

impl Default for Composicion {
    /// Una capa con cuatro columnas vacías, a 1920×1080.
    fn default() -> Self {
        let mut c = Composicion {
            ancho: ANCHO_POR_DEFECTO,
            alto: ALTO_POR_DEFECTO,
            columnas: Vec::new(),
            capas: vec![Capa::nueva("Capa 1".into(), 0)],
            transicion: 0.0,
        };
        for _ in 0..4 {
            c.agregar_columna();
        }
        c
    }
}

impl Composicion {
    /// Las fuentes de sonido que hay que escuchar ahora: las de las modulaciones de las capas,
    /// sin repetir y sin «Ninguna». Así se abre una captura por cada fuente distinta y ninguna
    /// de más.
    pub fn fuentes_de_audio(&self) -> Vec<crate::modulacion::FuenteDeAudio> {
        let mut fuentes: Vec<crate::modulacion::FuenteDeAudio> = Vec::new();
        for m in self
            .capas
            .iter()
            .filter_map(|c| c.modulacion_de_opacidad.as_ref())
        {
            if m.fuente != crate::modulacion::FuenteDeAudio::Ninguna && !fuentes.contains(&m.fuente)
            {
                fuentes.push(m.fuente.clone());
            }
        }
        fuentes
    }

    /// Agrega una columna al final (y una celda vacía en cada capa).
    pub fn agregar_columna(&mut self) {
        self.columnas.push(Columna {
            nombre: (self.columnas.len() + 1).to_string(),
        });
        for capa in &mut self.capas {
            capa.celdas.push(None);
        }
    }

    /// Quita una columna y las celdas que tenía en cada capa. No hace nada si no existe.
    pub fn quitar_columna(&mut self, columna: usize) {
        if columna >= self.columnas.len() {
            return;
        }
        self.columnas.remove(columna);
        for capa in &mut self.capas {
            if columna < capa.celdas.len() {
                capa.celdas.remove(columna);
            }
        }
    }

    /// Agrega una capa vacía al final.
    pub fn agregar_capa(&mut self) {
        self.capas.push(Capa::nueva(
            format!("Capa {}", self.capas.len() + 1),
            self.columnas.len(),
        ));
    }

    /// Reescribe las rutas de los videos para un proyecto que se mueve de `desde` a `hacia`
    /// (carpetas): cada ruta se resuelve con la carpeta vieja y se vuelve a guardar según
    /// la nueva, así los videos se siguen encontrando (relativos si quedan dentro de la
    /// nueva carpeta, absolutos si no).
    pub fn cambiar_carpeta(&mut self, desde: &Path, hacia: &Path) {
        for capa in &mut self.capas {
            for clip in capa.celdas.iter_mut().flatten() {
                // Solo los archivos tienen una ruta que reescribir.
                if let Fuente::Video { ruta } | Fuente::Imagen { ruta } = &mut clip.fuente {
                    *ruta = ruta_para_guardar(hacia, &resolver_ruta(desde, ruta));
                }
            }
        }
    }

    /// Pone (o reemplaza) el clip de una celda. No hace nada si la celda no existe.
    pub fn poner_clip(&mut self, capa: usize, columna: usize, clip: ConfigClip) {
        if let Some(celda) = self
            .capas
            .get_mut(capa)
            .and_then(|c| c.celdas.get_mut(columna))
        {
            *celda = Some(clip);
        }
    }

    /// Vacía una celda.
    pub fn quitar_clip(&mut self, capa: usize, columna: usize) {
        if let Some(celda) = self
            .capas
            .get_mut(capa)
            .and_then(|c| c.celdas.get_mut(columna))
        {
            *celda = None;
        }
    }

    /// Mueve el clip de la celda `desde` a la celda `hasta` (cada una es `(capa, columna)`).
    /// Si la de destino ya tiene un clip, los dos **se intercambian**. Devuelve `false` (y no
    /// cambia nada) si la de origen está vacía, si alguna celda no existe o si son la misma.
    pub fn mover_clip(&mut self, desde: (usize, usize), hasta: (usize, usize)) -> bool {
        let existe = |c: &Composicion, (capa, columna): (usize, usize)| {
            c.capas.get(capa).is_some_and(|l| columna < l.celdas.len())
        };
        if desde == hasta
            || !existe(self, desde)
            || !existe(self, hasta)
            || self.clip(desde.0, desde.1).is_none()
        {
            return false;
        }
        let origen = self.capas[desde.0].celdas[desde.1].take();
        let destino = self.capas[hasta.0].celdas[hasta.1].take();
        self.capas[hasta.0].celdas[hasta.1] = origen;
        self.capas[desde.0].celdas[desde.1] = destino;
        true
    }

    /// El clip de una celda, si existe y no está vacía.
    pub fn clip(&self, capa: usize, columna: usize) -> Option<&ConfigClip> {
        self.capas.get(capa)?.celdas.get(columna)?.as_ref()
    }

    /// Repara una composición leída de un archivo: la resolución dentro de los límites y
    /// una celda por columna en cada capa (un archivo editado a mano puede no cumplirlo).
    pub fn normalizar(&mut self) {
        self.ancho = limitar_resolucion(self.ancho);
        self.alto = limitar_resolucion(self.alto);
        // Un número inválido (NaN, de un archivo editado a mano) vale como «sin fundido».
        self.transicion = if self.transicion.is_finite() {
            self.transicion.clamp(0.0, TRANSICION_MAXIMA)
        } else {
            0.0
        };
        let n = self.columnas.len();
        for capa in &mut self.capas {
            capa.celdas.resize(n, None);
            // Una opacidad fuera de 0..1 (o NaN, de un archivo editado a mano) se limita.
            capa.opacidad = if capa.opacidad.is_nan() {
                1.0
            } else {
                capa.opacidad.clamp(0.0, 1.0)
            };
            if let Some(m) = &mut capa.modulacion_de_opacidad {
                m.normalizar();
            }
            for clip in capa.celdas.iter_mut().flatten() {
                clip.ajustes.normalizar();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Disparo de clips
// ---------------------------------------------------------------------------------------

/// Dónde queda el elemento que estaba en `i` cuando, en una lista, se mueve el elemento que
/// estaba en `desde` a la posición `hasta` (como un `remove` seguido de un `insert`).
///
/// Es la única cuenta que hace falta para reordenar capas: la usan el proyecto (para
/// corregir qué capa toma cada superficie), el estado de reproducción y las entidades de
/// Bevy, así todos coinciden.
pub fn nueva_posicion(i: usize, desde: usize, hasta: usize) -> usize {
    if i == desde {
        hasta
    } else if desde < hasta && i > desde && i <= hasta {
        i - 1 // el elemento se movió hacia atrás en la lista: los del medio suben un lugar
    } else if hasta < desde && i >= hasta && i < desde {
        i + 1 // se movió hacia adelante: los del medio bajan un lugar
    } else {
        i
    }
}

/// Una orden sobre la reproducción. La interfaz, el teclado y (más adelante) MIDI y OSC
/// envían las mismas órdenes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdenComposicion {
    /// Reproducir el clip de una celda en su capa.
    Disparar {
        capa: usize,
        columna: usize,
    },
    /// Dispara el clip de una captura de pantalla **y vuelve a mostrar el selector** del
    /// escritorio para elegir otra ventana o pantalla. La elección anterior no se pierde hasta
    /// que se elige otra: si se cancela, el clip sigue como estaba.
    ReelegirCaptura {
        capa: usize,
        columna: usize,
    },
    /// Disparar, en cada capa, el clip de esa columna.
    DispararColumna(usize),
    /// Detener la capa.
    Detener {
        capa: usize,
    },
    DetenerTodo,
    /// Pausar la capa, o seguir si estaba en pausa (si no suena nada, no hace nada).
    AlternarPausa {
        capa: usize,
    },
    /// Saltar a un punto del clip de la capa: `milesimas` de 0 a 1000 sobre su duración.
    Buscar {
        capa: usize,
        milesimas: u32,
    },
}

/// Lo que tiene que hacer el reproductor para reflejar un cambio de estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CambioCapa {
    Reproducir {
        capa: usize,
        columna: usize,
    },
    Detener {
        capa: usize,
    },
    /// Pausar (`true`) o seguir (`false`).
    Pausar {
        capa: usize,
        en_pausa: bool,
    },
    Buscar {
        capa: usize,
        milesimas: u32,
    },
}

/// Qué clip suena en cada capa (`Some(columna)`) o si está detenida (`None`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EstadoReproduccion {
    pub activo: Vec<Option<usize>>,
    /// Qué capas están en pausa (una capa detenida nunca lo está).
    pub pausada: Vec<bool>,
}

impl EstadoReproduccion {
    /// Aplica una orden y devuelve los cambios que el reproductor debe hacer. Es una
    /// función de datos puros: no abre videos, solo decide qué debería pasar.
    pub fn aplicar(&mut self, orden: OrdenComposicion, comp: &Composicion) -> Vec<CambioCapa> {
        self.activo.resize(comp.capas.len(), None);
        self.pausada.resize(comp.capas.len(), false);
        let mut cambios = Vec::new();
        match orden {
            // Para el estado de reproducción, elegir otra captura es disparar el clip.
            OrdenComposicion::Disparar { capa, columna }
            | OrdenComposicion::ReelegirCaptura { capa, columna } => {
                self.disparar(capa, columna, comp, &mut cambios);
            }
            OrdenComposicion::DispararColumna(columna) => {
                for capa in 0..comp.capas.len() {
                    self.disparar(capa, columna, comp, &mut cambios);
                }
            }
            OrdenComposicion::Detener { capa } => self.detener(capa, &mut cambios),
            OrdenComposicion::DetenerTodo => {
                for capa in 0..self.activo.len() {
                    self.detener(capa, &mut cambios);
                }
            }
            OrdenComposicion::AlternarPausa { capa } => {
                if self.activo.get(capa).copied().flatten().is_some() {
                    let en_pausa = !self.pausada[capa];
                    self.pausada[capa] = en_pausa;
                    cambios.push(CambioCapa::Pausar { capa, en_pausa });
                }
            }
            OrdenComposicion::Buscar { capa, milesimas } => {
                if self.activo.get(capa).copied().flatten().is_some() {
                    cambios.push(CambioCapa::Buscar { capa, milesimas });
                }
            }
        }
        cambios
    }

    /// Reconcilia el estado con la composición después de editarla: si el clip que sonaba
    /// se borró (o ya no existe), la capa se detiene.
    pub fn ajustar_a(&mut self, comp: &Composicion) -> Vec<CambioCapa> {
        self.activo.resize(comp.capas.len(), None);
        self.pausada.resize(comp.capas.len(), false);
        let mut cambios = Vec::new();
        for capa in 0..self.activo.len() {
            if let Some(columna) = self.activo[capa]
                && comp.clip(capa, columna).is_none()
            {
                self.detener(capa, &mut cambios);
            }
        }
        cambios
    }

    /// Avisa que se quitó una columna: la capa que la tenía activa se detiene (su clip ya no
    /// existe) y las columnas siguientes corren un lugar, así cada capa sigue apuntando al
    /// mismo clip que antes.
    pub fn columna_quitada(&mut self, columna: usize) {
        for (capa, activo) in self.activo.iter_mut().enumerate() {
            match *activo {
                Some(c) if c == columna => {
                    *activo = None;
                    if let Some(p) = self.pausada.get_mut(capa) {
                        *p = false;
                    }
                }
                Some(c) if c > columna => *activo = Some(c - 1),
                _ => {}
            }
        }
    }

    /// Avisa que se intercambiaron las celdas `desde` y `hasta` (ver `Composicion::mover_clip`).
    ///
    /// Dentro de una misma capa, si sonaba uno de los dos clips sigue sonando: solo cambia
    /// la columna a la que apunta. Si los clips cambian de capa, el que sonaba ya no está
    /// donde estaba: se devuelven las capas que hay que **detener** (con
    /// `OrdenComposicion::Detener`, antes de que el estado se actualice).
    pub fn clip_movido(&mut self, desde: (usize, usize), hasta: (usize, usize)) -> Vec<usize> {
        if desde.0 == hasta.0 {
            if let Some(activo) = self.activo.get_mut(desde.0) {
                if *activo == Some(desde.1) {
                    *activo = Some(hasta.1);
                } else if *activo == Some(hasta.1) {
                    *activo = Some(desde.1);
                }
            }
            return Vec::new();
        }
        [desde, hasta]
            .into_iter()
            .filter(|(capa, columna)| self.activo.get(*capa).copied().flatten() == Some(*columna))
            .map(|(capa, _)| capa)
            .collect()
    }

    /// Avisa que se movió la capa `desde` a la posición `hasta`: el estado de cada capa sigue
    /// a su capa.
    pub fn capa_movida(&mut self, desde: usize, hasta: usize) {
        if desde < self.activo.len() && hasta < self.activo.len() {
            let activo = self.activo.remove(desde);
            self.activo.insert(hasta, activo);
            if desde < self.pausada.len() && hasta < self.pausada.len() {
                let pausada = self.pausada.remove(desde);
                self.pausada.insert(hasta, pausada);
            }
        }
    }

    /// Avisa que se quitó la capa `capa`: su entrada del estado desaparece y las capas
    /// siguientes corren un lugar, así cada una sigue con su propio clip.
    pub fn capa_quitada(&mut self, capa: usize) {
        if capa < self.pausada.len() {
            self.pausada.remove(capa);
        }
        if capa < self.activo.len() {
            self.activo.remove(capa);
        }
    }

    fn disparar(
        &mut self,
        capa: usize,
        columna: usize,
        comp: &Composicion,
        cambios: &mut Vec<CambioCapa>,
    ) {
        // Una celda vacía o inexistente no hace nada: el clip que sonaba sigue sonando.
        if comp.clip(capa, columna).is_none() {
            return;
        }
        self.activo[capa] = Some(columna);
        self.pausada[capa] = false;
        cambios.push(CambioCapa::Reproducir { capa, columna });
    }

    fn detener(&mut self, capa: usize, cambios: &mut Vec<CambioCapa>) {
        if let Some(activo) = self.activo.get_mut(capa)
            && activo.take().is_some()
        {
            self.pausada[capa] = false;
            cambios.push(CambioCapa::Detener { capa });
        }
    }
}

/// Las órdenes con las que arranca un proyecto recién cargado: detener todo y disparar
/// el primer clip de cada capa, para que haya algo en pantalla sin tener que hacer clic.
pub fn ordenes_iniciales(comp: &Composicion) -> Vec<OrdenComposicion> {
    let mut ordenes = vec![OrdenComposicion::DetenerTodo];
    for (capa, c) in comp.capas.iter().enumerate() {
        if let Some(columna) = c.celdas.iter().position(|celda| celda.is_some()) {
            ordenes.push(OrdenComposicion::Disparar { capa, columna });
        }
    }
    ordenes
}

// ---------------------------------------------------------------------------------------
// Rutas de archivos
// ---------------------------------------------------------------------------------------

/// Cómo guardar la ruta de un archivo elegido: **relativa a la carpeta del proyecto** si
/// está dentro de ella (así el proyecto se puede mover o copiar con sus videos), y
/// absoluta si no.
pub fn ruta_para_guardar(carpeta_proyecto: &Path, elegida: &Path) -> String {
    match elegida.strip_prefix(carpeta_proyecto) {
        Ok(relativa) => relativa.to_string_lossy().into_owned(),
        Err(_) => elegida.to_string_lossy().into_owned(),
    }
}

/// Lo inverso: la ruta real de un archivo guardado en el proyecto.
pub fn resolver_ruta(carpeta_proyecto: &Path, guardada: &str) -> PathBuf {
    let ruta = Path::new(guardada);
    if ruta.is_absolute() {
        ruta.to_path_buf()
    } else {
        carpeta_proyecto.join(ruta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con_clips() -> Composicion {
        // 2 capas × 4 columnas, con clips en (0,0), (0,2), (1,2).
        let mut c = Composicion::default();
        c.agregar_capa();
        c.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        c.poner_clip(0, 2, ConfigClip::de_video("b.mp4".into()));
        c.poner_clip(1, 2, ConfigClip::de_video("c.mp4".into()));
        c
    }

    #[test]
    fn la_composicion_por_defecto_tiene_una_capa_y_cuatro_columnas() {
        let c = Composicion::default();
        assert_eq!((c.ancho, c.alto), (1920, 1080));
        assert_eq!(c.columnas.len(), 4);
        assert_eq!(c.capas.len(), 1);
        assert_eq!(c.capas[0].celdas.len(), 4);
        assert!(c.capas[0].celdas.iter().all(|celda| celda.is_none()));
    }

    #[test]
    fn agregar_y_quitar_columnas_mantiene_una_celda_por_columna() {
        let mut c = con_clips();
        c.agregar_columna();
        assert_eq!(c.columnas.len(), 5);
        assert!(c.capas.iter().all(|capa| capa.celdas.len() == 5));
        c.quitar_columna(0);
        assert_eq!(c.columnas.len(), 4);
        assert!(c.capas.iter().all(|capa| capa.celdas.len() == 4));
        // El clip de la columna 2 pasó a ser el de la 1.
        assert_eq!(c.clip(0, 1).unwrap().nombre, "b");
    }

    #[test]
    fn quitar_una_columna_inexistente_no_hace_nada() {
        let mut c = con_clips();
        let antes = c.clone();
        c.quitar_columna(99);
        assert_eq!(c, antes);
    }

    #[test]
    fn agregar_capa_crea_celdas_vacias_para_todas_las_columnas() {
        let mut c = Composicion::default();
        c.agregar_capa();
        assert_eq!(c.capas.len(), 2);
        assert_eq!(c.capas[1].nombre, "Capa 2");
        assert_eq!(c.capas[1].celdas.len(), 4);
    }

    #[test]
    fn una_capa_nueva_es_visible_opaca_y_normal() {
        let c = Composicion::default();
        let capa = &c.capas[0];
        assert!(capa.visible);
        assert_eq!(capa.opacidad, 1.0);
        assert_eq!(capa.modo, ModoMezcla::Normal);
        assert_eq!(capa.ajuste, Ajuste::Ajustar);
    }

    #[test]
    fn normalizar_limita_la_opacidad() {
        let mut c = Composicion::default();
        c.agregar_capa();
        c.agregar_capa();
        c.capas[0].opacidad = 3.0;
        c.capas[1].opacidad = -1.0;
        c.capas[2].opacidad = f32::NAN;
        c.normalizar();
        assert_eq!(c.capas[0].opacidad, 1.0);
        assert_eq!(c.capas[1].opacidad, 0.0);
        assert_eq!(c.capas[2].opacidad, 1.0);
    }

    #[test]
    fn los_modos_tienen_numeros_distintos_para_el_shader() {
        let mut numeros: Vec<u32> = ModoMezcla::TODOS.iter().map(|m| m.numero()).collect();
        numeros.sort();
        numeros.dedup();
        assert_eq!(numeros, vec![0, 1, 2, 3]);
    }

    #[test]
    fn estirar_ocupa_toda_la_composicion() {
        assert_eq!(
            tamano_de_ajuste(Ajuste::Estirar, (1920.0, 1080.0), (640.0, 480.0)),
            (1920.0, 1080.0)
        );
    }

    #[test]
    fn ajustar_un_video_mas_cuadrado_deja_bandas_a_los_lados() {
        // Un video 4:3 en una composición 16:9: entra a lo alto y sobra ancho.
        let (w, h) = tamano_de_ajuste(Ajuste::Ajustar, (1920.0, 1080.0), (640.0, 480.0));
        assert!((h - 1080.0).abs() < 1e-3);
        assert!((w - 1440.0).abs() < 1e-3);
    }

    #[test]
    fn ajustar_un_video_mas_ancho_deja_bandas_arriba_y_abajo() {
        // Un video 21:9 en una composición 16:9: entra a lo ancho y sobra alto.
        let (w, h) = tamano_de_ajuste(Ajuste::Ajustar, (1920.0, 1080.0), (2560.0, 1080.0));
        assert!((w - 1920.0).abs() < 1e-3);
        assert!(h < 1080.0);
    }

    #[test]
    fn llenar_cubre_toda_la_composicion_y_conserva_la_proporcion() {
        let (w, h) = tamano_de_ajuste(Ajuste::Llenar, (1920.0, 1080.0), (640.0, 480.0));
        assert!((w - 1920.0).abs() < 1e-3); // cubre el ancho
        assert!(h > 1080.0); // y se pasa de alto: se recorta
        assert!((w / h - 640.0 / 480.0).abs() < 1e-4);
    }

    #[test]
    fn un_video_de_la_misma_proporcion_se_ve_igual_en_los_tres_ajustes() {
        for ajuste in Ajuste::TODOS {
            let (w, h) = tamano_de_ajuste(ajuste, (1920.0, 1080.0), (1280.0, 720.0));
            assert!(
                (w - 1920.0).abs() < 1e-2 && (h - 1080.0).abs() < 1e-2,
                "{ajuste:?}"
            );
        }
    }

    #[test]
    fn un_video_de_tamano_cero_usa_la_composicion() {
        assert_eq!(
            tamano_de_ajuste(Ajuste::Ajustar, (1920.0, 1080.0), (0.0, 0.0)),
            (1920.0, 1080.0)
        );
    }

    #[test]
    fn el_nombre_del_clip_es_el_del_archivo_sin_extension() {
        assert_eq!(
            ConfigClip::de_video("videos/mi video.mp4".into()).nombre,
            "mi video"
        );
    }

    #[test]
    fn normalizar_repara_celdas_y_resolucion() {
        let mut c = Composicion {
            ancho: 10,
            alto: 100_000,
            ..Default::default()
        };
        c.capas[0].celdas.truncate(1); // faltan celdas
        c.normalizar();
        assert_eq!(c.ancho, RESOLUCION_MINIMA);
        assert_eq!(c.alto, RESOLUCION_MAXIMA);
        assert_eq!(c.capas[0].celdas.len(), 4);
    }

    #[test]
    fn disparar_una_celda_con_clip_la_activa() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        let cambios = e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 2,
            },
            &c,
        );
        assert_eq!(
            cambios,
            vec![CambioCapa::Reproducir {
                capa: 0,
                columna: 2
            }]
        );
        assert_eq!(e.activo, vec![Some(2), None]);
    }

    #[test]
    fn disparar_una_celda_vacia_no_cambia_lo_que_suena() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &c,
        );
        let cambios = e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 1,
            },
            &c,
        );
        assert!(cambios.is_empty());
        assert_eq!(e.activo[0], Some(0));
    }

    #[test]
    fn disparar_una_celda_inexistente_no_hace_nada() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        assert!(
            e.aplicar(
                OrdenComposicion::Disparar {
                    capa: 9,
                    columna: 0
                },
                &c
            )
            .is_empty()
        );
        assert!(
            e.aplicar(
                OrdenComposicion::Disparar {
                    capa: 0,
                    columna: 9
                },
                &c
            )
            .is_empty()
        );
    }

    #[test]
    fn disparar_una_columna_dispara_cada_capa_que_tenga_clip() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        let cambios = e.aplicar(OrdenComposicion::DispararColumna(2), &c);
        assert_eq!(
            cambios,
            vec![
                CambioCapa::Reproducir {
                    capa: 0,
                    columna: 2
                },
                CambioCapa::Reproducir {
                    capa: 1,
                    columna: 2
                },
            ]
        );
        // La columna 0 solo tiene clip en la capa 0: la capa 1 sigue como estaba.
        let cambios = e.aplicar(OrdenComposicion::DispararColumna(0), &c);
        assert_eq!(
            cambios,
            vec![CambioCapa::Reproducir {
                capa: 0,
                columna: 0
            }]
        );
        assert_eq!(e.activo, vec![Some(0), Some(2)]);
    }

    #[test]
    fn detener_avisa_solo_si_algo_sonaba() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        assert!(
            e.aplicar(OrdenComposicion::Detener { capa: 0 }, &c)
                .is_empty()
        );
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &c,
        );
        assert_eq!(
            e.aplicar(OrdenComposicion::Detener { capa: 0 }, &c),
            vec![CambioCapa::Detener { capa: 0 }]
        );
        assert_eq!(e.activo[0], None);
    }

    #[test]
    fn detener_todo_detiene_cada_capa_activa() {
        let c = con_clips();
        let mut e = EstadoReproduccion::default();
        e.aplicar(OrdenComposicion::DispararColumna(2), &c);
        let cambios = e.aplicar(OrdenComposicion::DetenerTodo, &c);
        assert_eq!(cambios.len(), 2);
        assert_eq!(e.activo, vec![None, None]);
    }

    #[test]
    fn las_ordenes_iniciales_detienen_todo_y_disparan_el_primer_clip_de_cada_capa() {
        let c = con_clips();
        assert_eq!(
            ordenes_iniciales(&c),
            vec![
                OrdenComposicion::DetenerTodo,
                OrdenComposicion::Disparar {
                    capa: 0,
                    columna: 0
                },
                OrdenComposicion::Disparar {
                    capa: 1,
                    columna: 2
                },
            ]
        );
    }

    #[test]
    fn una_composicion_vacia_solo_detiene() {
        assert_eq!(
            ordenes_iniciales(&Composicion::default()),
            vec![OrdenComposicion::DetenerTodo]
        );
    }

    #[test]
    fn nueva_posicion_coincide_con_remove_mas_insert() {
        // Se compara con lo que hace de verdad una lista, para todas las combinaciones.
        for largo in 1..=6 {
            for desde in 0..largo {
                for hasta in 0..largo {
                    let mut lista: Vec<usize> = (0..largo).collect();
                    let movido = lista.remove(desde);
                    lista.insert(hasta, movido);
                    // `lista[nueva]` tiene que ser el elemento que estaba en `i`.
                    for i in 0..largo {
                        let nueva = nueva_posicion(i, desde, hasta);
                        assert_eq!(
                            lista[nueva], i,
                            "largo {largo}, mover {desde} a {hasta}: el {i} debería quedar en {nueva}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mover_una_capa_mueve_su_estado_de_reproduccion() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(0), None, Some(2), Some(1)],
            ..Default::default()
        };
        e.capa_movida(3, 0);
        assert_eq!(e.activo, vec![Some(1), Some(0), None, Some(2)]);
        e.capa_movida(0, 2);
        assert_eq!(e.activo, vec![Some(0), None, Some(1), Some(2)]);
        e.capa_movida(9, 0); // no existe: no hace nada
        assert_eq!(e.activo.len(), 4);
    }

    #[test]
    fn quitar_una_capa_corre_el_estado_de_las_siguientes() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(0), Some(2), None, Some(1)],
            ..Default::default()
        };
        e.capa_quitada(1);
        assert_eq!(e.activo, vec![Some(0), None, Some(1)]);
        e.capa_quitada(9); // no existe: no hace nada
        assert_eq!(e.activo.len(), 3);
    }

    #[test]
    fn quitar_una_columna_corre_los_indices_de_las_siguientes() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(0), Some(2), Some(3), None],
            ..Default::default()
        };
        e.columna_quitada(2);
        // La capa de la columna quitada se detiene; la de una posterior corre un lugar;
        // la de una anterior y la detenida no cambian.
        assert_eq!(e.activo, vec![Some(0), None, Some(2), None]);
    }

    #[test]
    fn ajustar_detiene_la_capa_si_se_borro_su_clip() {
        let mut c = con_clips();
        let mut e = EstadoReproduccion::default();
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &c,
        );
        assert!(e.ajustar_a(&c).is_empty()); // todo en orden
        c.quitar_clip(0, 0);
        assert_eq!(e.ajustar_a(&c), vec![CambioCapa::Detener { capa: 0 }]);
        assert_eq!(e.activo[0], None);
    }

    #[test]
    fn las_rutas_dentro_del_proyecto_se_guardan_relativas() {
        let base = Path::new("/home/marco/proyecto");
        let guardada = ruta_para_guardar(base, Path::new("/home/marco/proyecto/videos/a.mp4"));
        assert_eq!(guardada, "videos/a.mp4");
        assert_eq!(
            resolver_ruta(base, &guardada),
            PathBuf::from("/home/marco/proyecto/videos/a.mp4")
        );
    }

    #[test]
    fn las_rutas_fuera_del_proyecto_se_guardan_absolutas() {
        let base = Path::new("/home/marco/proyecto");
        let guardada = ruta_para_guardar(base, Path::new("/mnt/discos/b.mp4"));
        assert_eq!(guardada, "/mnt/discos/b.mp4");
        assert_eq!(
            resolver_ruta(base, &guardada),
            PathBuf::from("/mnt/discos/b.mp4")
        );
    }

    #[test]
    fn cambiar_de_carpeta_conserva_donde_estan_los_videos() {
        let mut c = Composicion::default();
        c.poner_clip(0, 0, ConfigClip::de_video("videos/a.mp4".into()));
        c.poner_clip(0, 1, ConfigClip::de_video("/mnt/discos/b.mp4".into()));
        c.cambiar_carpeta(Path::new("/home/m/uno"), Path::new("/home/m/uno/sub"));
        // El primero ahora queda fuera de la carpeta nueva: pasa a absoluto.
        assert_eq!(
            c.clip(0, 0).unwrap().fuente,
            Fuente::Video {
                ruta: "/home/m/uno/videos/a.mp4".into()
            }
        );
        // El absoluto sigue igual.
        assert_eq!(
            c.clip(0, 1).unwrap().fuente,
            Fuente::Video {
                ruta: "/mnt/discos/b.mp4".into()
            }
        );
        // Y al volver a la carpeta original, el primero vuelve a ser relativo.
        c.cambiar_carpeta(Path::new("/home/m/uno/sub"), Path::new("/home/m/uno"));
        assert_eq!(
            c.clip(0, 0).unwrap().fuente,
            Fuente::Video {
                ruta: "videos/a.mp4".into()
            }
        );
    }

    #[test]
    fn mover_un_clip_a_una_celda_vacia_lo_traslada() {
        let mut c = Composicion::default();
        c.agregar_capa();
        c.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        assert!(c.mover_clip((0, 0), (1, 2)));
        assert!(c.clip(0, 0).is_none());
        assert_eq!(c.clip(1, 2).unwrap().nombre, "a");
    }

    #[test]
    fn mover_un_clip_a_una_celda_ocupada_los_intercambia() {
        let mut c = Composicion::default();
        c.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        c.poner_clip(0, 3, ConfigClip::de_video("b.mp4".into()));
        assert!(c.mover_clip((0, 0), (0, 3)));
        assert_eq!(c.clip(0, 0).unwrap().nombre, "b");
        assert_eq!(c.clip(0, 3).unwrap().nombre, "a");
    }

    #[test]
    fn mover_un_clip_rechaza_lo_que_no_tiene_sentido() {
        let mut c = Composicion::default();
        c.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        let antes = c.clone();
        assert!(!c.mover_clip((0, 0), (0, 0))); // la misma celda
        assert!(!c.mover_clip((0, 1), (0, 2))); // origen vacío
        assert!(!c.mover_clip((0, 0), (5, 0))); // capa inexistente
        assert!(!c.mover_clip((0, 0), (0, 9))); // columna inexistente
        assert_eq!(c, antes);
    }

    #[test]
    fn dentro_de_una_capa_el_clip_que_suena_sigue_sonando() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(0), None],
            ..Default::default()
        };
        assert!(e.clip_movido((0, 0), (0, 2)).is_empty());
        assert_eq!(e.activo, vec![Some(2), None]);
        // Si sonaba el de destino (intercambio), pasa a la columna de origen.
        assert!(e.clip_movido((0, 1), (0, 2)).is_empty());
        assert_eq!(e.activo, vec![Some(1), None]);
        // Si sonaba otro, no cambia.
        assert!(e.clip_movido((0, 0), (0, 3)).is_empty());
        assert_eq!(e.activo, vec![Some(1), None]);
    }

    #[test]
    fn al_cambiar_de_capa_se_detienen_las_capas_que_perderian_su_clip() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(1), Some(3), None],
            ..Default::default()
        };
        // El clip (0,1) suena; se manda a la capa 2: la capa 0 se detiene.
        assert_eq!(e.clip_movido((0, 1), (2, 0)), vec![0]);
        // Ambos suenan y se intercambian entre capas: las dos se detienen.
        assert_eq!(e.clip_movido((0, 1), (1, 3)), vec![0, 1]);
        // Ninguno suena: no hay nada que detener.
        assert!(e.clip_movido((0, 0), (1, 0)).is_empty());
    }

    #[test]
    fn los_ajustes_por_defecto_son_los_de_siempre() {
        let a = AjustesClip::default();
        assert!(a.bucle && !a.silenciado);
        assert_eq!(
            (a.velocidad, a.inicio, a.fin, a.volumen),
            (1.0, 0.0, 1.0, 1.0)
        );
        assert!(a.tramo_completo());
    }

    #[test]
    fn normalizar_los_ajustes_limita_y_ordena_el_tramo() {
        let mut a = AjustesClip {
            velocidad: 99.0,
            volumen: f32::NAN,
            inicio: 0.8,
            fin: 0.2,
            ..Default::default()
        };
        a.normalizar();
        assert_eq!(a.velocidad, VELOCIDAD_MAXIMA);
        assert_eq!(a.volumen, 1.0);
        assert_eq!((a.inicio, a.fin), (0.2, 0.8)); // se ordenó
        // Un tramo casi nulo se agranda.
        let mut b = AjustesClip {
            inicio: 0.5,
            fin: 0.5,
            ..Default::default()
        };
        b.normalizar();
        assert!(b.fin - b.inicio >= SEPARACION_MINIMA - 1e-6);
        // Al final del video, se agranda hacia atrás.
        let mut c = AjustesClip {
            inicio: 1.0,
            fin: 1.0,
            ..Default::default()
        };
        c.normalizar();
        assert_eq!(c.fin, 1.0);
        assert!(c.inicio <= 1.0 - SEPARACION_MINIMA + 1e-6);
    }

    #[test]
    fn un_clip_viejo_sin_ajustes_se_lee_con_los_de_siempre() {
        let clip: ConfigClip =
            ron::from_str(r#"(nombre: "a", fuente: Video(ruta: "a.mp4"))"#).unwrap();
        assert_eq!(clip.ajustes, AjustesClip::default());
        // Y uno con ajustes parciales completa el resto.
        let clip: ConfigClip = ron::from_str(
            r#"(nombre: "a", fuente: Video(ruta: "a.mp4"), ajustes: (velocidad: 2.0))"#,
        )
        .unwrap();
        assert_eq!(clip.ajustes.velocidad, 2.0);
        assert!(clip.ajustes.bucle);
    }

    #[test]
    fn la_pausa_solo_vale_con_un_clip_sonando() {
        let mut comp = Composicion::default();
        comp.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        let mut e = EstadoReproduccion::default();
        // Sin nada sonando, pausar no hace nada.
        assert!(
            e.aplicar(OrdenComposicion::AlternarPausa { capa: 0 }, &comp)
                .is_empty()
        );
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &comp,
        );
        assert_eq!(
            e.aplicar(OrdenComposicion::AlternarPausa { capa: 0 }, &comp),
            vec![CambioCapa::Pausar {
                capa: 0,
                en_pausa: true
            }]
        );
        assert!(e.pausada[0]);
        assert_eq!(
            e.aplicar(OrdenComposicion::AlternarPausa { capa: 0 }, &comp),
            vec![CambioCapa::Pausar {
                capa: 0,
                en_pausa: false
            }]
        );
    }

    #[test]
    fn disparar_o_detener_saca_la_pausa() {
        let mut comp = Composicion::default();
        comp.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        comp.poner_clip(0, 1, ConfigClip::de_video("b.mp4".into()));
        let mut e = EstadoReproduccion::default();
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &comp,
        );
        e.aplicar(OrdenComposicion::AlternarPausa { capa: 0 }, &comp);
        // Un clip nuevo suena de inmediato.
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 1,
            },
            &comp,
        );
        assert!(!e.pausada[0]);
        e.aplicar(OrdenComposicion::AlternarPausa { capa: 0 }, &comp);
        e.aplicar(OrdenComposicion::Detener { capa: 0 }, &comp);
        assert!(!e.pausada[0]);
    }

    #[test]
    fn buscar_solo_vale_con_un_clip_sonando() {
        let mut comp = Composicion::default();
        comp.poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        let mut e = EstadoReproduccion::default();
        let orden = OrdenComposicion::Buscar {
            capa: 0,
            milesimas: 500,
        };
        assert!(e.aplicar(orden, &comp).is_empty());
        e.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 0,
            },
            &comp,
        );
        assert_eq!(
            e.aplicar(orden, &comp),
            vec![CambioCapa::Buscar {
                capa: 0,
                milesimas: 500
            }]
        );
    }

    #[test]
    fn la_pausa_sigue_a_su_capa_al_mover_o_quitar() {
        let mut e = EstadoReproduccion {
            activo: vec![Some(0), Some(0), Some(0)],
            pausada: vec![true, false, false],
        };
        e.capa_movida(0, 2);
        assert_eq!(e.pausada, vec![false, false, true]);
        e.capa_quitada(2);
        assert_eq!(e.pausada, vec![false, false]);
        e.columna_quitada(0);
        assert_eq!(e.activo, vec![None, None]);
        assert_eq!(e.pausada, vec![false, false]);
    }

    #[test]
    fn las_fuentes_en_vivo_se_reconocen_y_se_describen() {
        let archivo = Fuente::Video {
            ruta: "a.mp4".into(),
        };
        let camara = Fuente::Camara {
            dispositivo: "/dev/video0".into(),
        };
        let red = Fuente::Url {
            url: "rtsp://192.168.1.5/stream".into(),
        };
        assert!(!archivo.es_en_vivo() && camara.es_en_vivo() && red.es_en_vivo());
        assert_eq!(archivo.ruta_de_archivo(), Some("a.mp4"));
        assert_eq!(camara.ruta_de_archivo(), None);
        assert_eq!(camara.descripcion(), "Cámara /dev/video0");
        assert_eq!(red.descripcion(), "rtsp://192.168.1.5/stream");
    }

    #[test]
    fn el_nombre_de_un_clip_de_red_es_el_servidor() {
        assert_eq!(
            ConfigClip::de_url("rtsp://192.168.1.5:8554/camara?x=1".into()).nombre,
            "192.168.1.5:8554"
        );
        assert_eq!(
            ConfigClip::de_url("http://ejemplo.com".into()).nombre,
            "ejemplo.com"
        );
    }

    #[test]
    fn las_fuentes_en_vivo_se_guardan_y_se_leen() {
        let clip = ConfigClip::de_camara("/dev/video2".into(), "Webcam".into());
        let texto = ron::to_string(&clip).unwrap();
        let leido: ConfigClip = ron::from_str(&texto).unwrap();
        assert_eq!(leido, clip);
    }

    #[test]
    fn cambiar_de_carpeta_no_toca_las_fuentes_en_vivo() {
        let mut c = Composicion::default();
        c.poner_clip(
            0,
            0,
            ConfigClip::de_camara("/dev/video0".into(), "Cam".into()),
        );
        c.poner_clip(0, 1, ConfigClip::de_url("rtsp://x/y".into()));
        let antes = c.clone();
        c.cambiar_carpeta(Path::new("/a"), Path::new("/b"));
        assert_eq!(c, antes);
    }

    #[test]
    fn el_audio_de_una_captura_se_guarda_y_los_proyectos_viejos_no_lo_traen() {
        let mut c = Composicion::default();
        c.poner_clip(
            0,
            0,
            ConfigClip {
                nombre: "Cap".into(),
                fuente: Fuente::Captura {
                    token: Some("t".into()),
                    audio: AudioCaptura::Aplicacion("Firefox".into()),
                    ocultar_cursor: true,
                },
                ajustes: AjustesClip::default(),
            },
        );
        let texto = ron::to_string(&c).unwrap();
        let leida: Composicion = ron::from_str(&texto).unwrap();
        assert_eq!(leida, c);

        // Un proyecto de antes: la captura no tiene `audio` ni `ocultar_cursor`: queda sin
        // sonido y con el cursor visible, como siempre.
        let viejo: Fuente = ron::from_str(r#"Captura(token: Some("t"))"#).unwrap();
        assert_eq!(
            viejo,
            Fuente::Captura {
                token: Some("t".into()),
                audio: AudioCaptura::Ninguno,
                ocultar_cursor: false,
            }
        );
    }

    #[test]
    fn el_fundido_sube_de_cero_a_uno_y_sin_duracion_es_un_corte() {
        assert_eq!(avance_de_transicion(0.0, 2.0), 0.0);
        assert!((avance_de_transicion(0.5, 2.0) - 0.25).abs() < 1e-6);
        assert!((avance_de_transicion(1.0, 2.0) - 0.5).abs() < 1e-6);
        // Se queda en 1 aunque pase de largo (y no baja de 0 con un tiempo negativo).
        assert_eq!(avance_de_transicion(5.0, 2.0), 1.0);
        assert_eq!(avance_de_transicion(-1.0, 2.0), 0.0);
        // Sin duración, o con una inválida: ya se ve el clip nuevo entero.
        assert_eq!(avance_de_transicion(0.0, 0.0), 1.0);
        assert_eq!(avance_de_transicion(0.3, -1.0), 1.0);
        assert_eq!(avance_de_transicion(0.3, f32::NAN), 1.0);
    }

    #[test]
    fn la_duracion_del_fundido_se_limita_y_los_proyectos_viejos_no_la_traen() {
        let mut c = Composicion::default();
        assert_eq!(c.transicion, 0.0);
        c.transicion = 99.0;
        c.normalizar();
        assert_eq!(c.transicion, TRANSICION_MAXIMA);
        c.transicion = f32::NAN;
        c.normalizar();
        assert_eq!(c.transicion, 0.0);
        c.transicion = -3.0;
        c.normalizar();
        assert_eq!(c.transicion, 0.0);

        // Un proyecto de antes no tiene el campo: queda en corte.
        let viejo = r#"(ancho: 1920, alto: 1080, columnas: [], capas: [])"#;
        let leida: Composicion = ron::from_str(viejo).unwrap();
        assert_eq!(leida.transicion, 0.0);
    }

    #[test]
    fn el_fundido_normal_da_la_mezcla_exacta_con_cualquier_opacidad() {
        // Se dibuja el saliente y encima el entrante, con mezcla normal, y se compara con
        // `fondo·(1−op) + op·(a·nuevo + (1−a)·viejo)`.
        let (fondo, viejo, nuevo) = (0.2_f32, 0.9_f32, 0.4_f32);
        for opacidad in [1.0, 0.8, 0.5, 0.1] {
            for paso in 0..=10 {
                let a = paso as f32 / 10.0;
                let (s, e) = opacidades_de_transicion(opacidad, a, ModoMezcla::Normal);
                let tras_saliente = fondo * (1.0 - s) + viejo * s;
                let resultado = tras_saliente * (1.0 - e) + nuevo * e;
                let ideal = fondo * (1.0 - opacidad) + opacidad * (a * nuevo + (1.0 - a) * viejo);
                assert!(
                    (resultado - ideal).abs() < 1e-5,
                    "op {opacidad} avance {a}: {resultado} y no {ideal}"
                );
            }
        }
    }

    #[test]
    fn con_la_capa_opaca_el_saliente_queda_entero_y_al_final_desaparece() {
        let (s, e) = opacidades_de_transicion(1.0, 0.5, ModoMezcla::Normal);
        assert!((s - 1.0).abs() < 1e-6 && (e - 0.5).abs() < 1e-6);
        // Al final solo queda el nuevo, con la opacidad de la capa.
        assert_eq!(
            opacidades_de_transicion(0.7, 1.0, ModoMezcla::Normal),
            (0.0, 0.7)
        );
        // Al principio solo se ve el viejo, con la opacidad de la capa.
        let (s, e) = opacidades_de_transicion(0.7, 0.0, ModoMezcla::Normal);
        assert!((s - 0.7).abs() < 1e-6 && e == 0.0);
    }

    #[test]
    fn en_los_otros_modos_cada_clip_aporta_su_parte() {
        for modo in [
            ModoMezcla::Suma,
            ModoMezcla::Multiplicar,
            ModoMezcla::Pantalla,
        ] {
            let (s, e) = opacidades_de_transicion(0.8, 0.25, modo);
            assert!((s - 0.6).abs() < 1e-6 && (e - 0.2).abs() < 1e-6, "{modo:?}");
        }
    }

    #[test]
    fn elegir_otra_captura_dispara_el_clip_igual_que_disparar() {
        let c = con_clips();
        let mut a = EstadoReproduccion::default();
        let mut b = EstadoReproduccion::default();
        let por_disparo = a.aplicar(
            OrdenComposicion::Disparar {
                capa: 0,
                columna: 2,
            },
            &c,
        );
        let por_reeleccion = b.aplicar(
            OrdenComposicion::ReelegirCaptura {
                capa: 0,
                columna: 2,
            },
            &c,
        );
        assert_eq!(por_disparo, por_reeleccion);
        assert_eq!(a, b);
    }

    #[test]
    fn un_archivo_de_imagen_se_reconoce_por_la_extension() {
        for ruta in ["a.png", "fotos/B.JPG", "c.jpeg", "x/y/d.WebP"] {
            assert!(es_ruta_de_imagen(ruta), "{ruta}");
            assert!(
                ConfigClip::de_archivo(ruta.into()).fuente.es_imagen(),
                "{ruta}"
            );
        }
        for ruta in ["a.mp4", "b.mov", "sin_extension", "c.bmp", "d.tiff"] {
            assert!(!es_ruta_de_imagen(ruta), "{ruta}");
            assert!(
                !ConfigClip::de_archivo(ruta.into()).fuente.es_imagen(),
                "{ruta}"
            );
        }
    }

    #[test]
    fn una_imagen_tiene_archivo_pero_no_es_una_fuente_en_vivo() {
        let clip = ConfigClip::de_imagen("fotos/logo.png".into());
        assert_eq!(clip.nombre, "logo");
        assert_eq!(clip.fuente.ruta_de_archivo(), Some("fotos/logo.png"));
        assert_eq!(clip.fuente.descripcion(), "fotos/logo.png");
        assert!(clip.fuente.es_imagen());
        // No tiene duración, pero tampoco es «en vivo»: es un archivo.
        assert!(!clip.fuente.es_en_vivo());
        assert!(
            ConfigClip::de_camara("/dev/video0".into(), "Cam".into())
                .fuente
                .es_en_vivo()
        );
        assert!(!ConfigClip::de_video("a.mp4".into()).fuente.es_imagen());
    }

    #[test]
    fn al_cambiar_de_carpeta_las_imagenes_tambien_se_reescriben() {
        let mut c = Composicion::default();
        c.poner_clip(0, 0, ConfigClip::de_imagen("fotos/a.png".into()));
        c.cambiar_carpeta(Path::new("/home/m/uno"), Path::new("/home/m/uno/sub"));
        assert_eq!(
            c.clip(0, 0).unwrap().fuente,
            Fuente::Imagen {
                ruta: "/home/m/uno/fotos/a.png".into()
            }
        );
    }

    #[test]
    fn una_imagen_se_guarda_y_se_lee() {
        let mut c = Composicion::default();
        c.poner_clip(0, 1, ConfigClip::de_imagen("fotos/a.webp".into()));
        let leida: Composicion = ron::from_str(&ron::to_string(&c).unwrap()).unwrap();
        assert_eq!(leida, c);
    }

    #[test]
    fn la_modulacion_de_la_opacidad_se_guarda_y_los_proyectos_viejos_no_la_traen() {
        let mut c = Composicion::default();
        assert!(c.capas[0].modulacion_de_opacidad.is_none());
        c.capas[0].modulacion_de_opacidad = Some(Modulacion {
            senal: crate::modulacion::OrigenDeSenal::Graves,
            cantidad: 0.7,
            ..Modulacion::default()
        });
        let leida: Composicion = ron::from_str(&ron::to_string(&c).unwrap()).unwrap();
        assert_eq!(leida, c);
        // Una capa de un proyecto de antes no tiene el campo.
        let viejo: Capa = ron::from_str(r#"(nombre: "Fondo", celdas: [])"#).unwrap();
        assert!(viejo.modulacion_de_opacidad.is_none());
    }

    #[test]
    fn las_fuentes_de_audio_en_uso_no_se_repiten() {
        use crate::modulacion::FuenteDeAudio as F;
        let mut c = Composicion::default();
        c.agregar_capa();
        c.agregar_capa();
        assert!(c.fuentes_de_audio().is_empty());
        let con = |f: F| {
            Some(Modulacion {
                fuente: f,
                ..Modulacion::default()
            })
        };
        c.capas[0].modulacion_de_opacidad = con(F::Microfono);
        c.capas[1].modulacion_de_opacidad = con(F::Microfono);
        c.capas[2].modulacion_de_opacidad = con(F::Ninguna);
        assert_eq!(c.fuentes_de_audio(), vec![F::Microfono]);
        c.capas[1].modulacion_de_opacidad = con(F::Capa(0));
        assert_eq!(c.fuentes_de_audio(), vec![F::Microfono, F::Capa(0)]);
    }

    #[test]
    fn al_cargar_se_repara_una_modulacion_con_numeros_invalidos() {
        let mut c = Composicion::default();
        c.capas[0].modulacion_de_opacidad = Some(Modulacion {
            senal: crate::modulacion::OrigenDeSenal::Nivel,
            cantidad: f32::NAN,
            ..Modulacion::default()
        });
        c.normalizar();
        assert_eq!(
            c.capas[0].modulacion_de_opacidad.as_ref().unwrap().cantidad,
            0.0
        );
    }
}
