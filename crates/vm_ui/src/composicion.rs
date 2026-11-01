//! Pestaña "Composición": resolución, grilla de capas × clips y vista previa.
//!
//! - Hacer clic en un clip lo **dispara** en su capa; el clip que suena se resalta.
//! - Una celda vacía muestra un **+** para elegir un archivo con el selector del sistema.
//! - Clic derecho en un clip o en una columna abre un menú (cambiar archivo, quitar...).
//!
//! La interfaz no abre videos ni toca la reproducción: edita el proyecto y envía
//! órdenes de disparo; `vm_composicion` las ejecuta.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::estilo::{amarillo, azul, con_icono, icono, naranja, rojo, texto_negrita, verde};
use crate::explorador::{self, EntradaArchivo};
use crate::inspector::{
    ALTO_FILA, boton, boton_cuadrado, boton_elegible, botones_iguales, campo_de_texto,
    contenido_en_rect, deslizador, deslizador_con, desplegable, fila, seccion,
};
use crate::parametro::{ContextoDeAudio, fila_de_parametro};
use crate::reparto;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use vm_audio::{AplicacionesQueSuenan, SenalesDeAudio};
use vm_composicion::{
    ErroresDeCapas, EstadoMiniatura, Miniaturas, OrdenClip, ProgresoDeCapas, TexturaComposicion,
    TexturasDeCapas,
};
use vm_control::{
    AprendizajeMidi, ConfigMidi, ConfigOsc, DestinoOsc, EstadoMidi, EstadoOsc, PuertosMidi,
};
use vm_core::{
    Ajuste, AjustesClip, ConfigClip, EstadoReproduccion, Fuente, ModoMezcla, OrdenComposicion,
    RESOLUCION_MAXIMA, RESOLUCION_MINIMA, TRANSICION_MAXIMA, VELOCIDAD_MAXIMA, VELOCIDAD_MINIMA,
    VOLUMEN_MAXIMO, limitar_resolucion, nueva_posicion, resolver_ruta, ruta_para_guardar,
};
use vm_proyecto::{ProyectoActual, RutaProyecto};
use vm_video::decodificador::{DispositivoDeVideo, listar_dispositivos};

/// Dónde guarda egui, entre un frame y otro, los rectángulos de las filas de capas y cuál se arrastra.
const MEMORIA_FILAS: &str = "filas_de_capas";
const MEMORIA_ARRASTRADA: &str = "capa_arrastrada";
/// Clave en la memoria de egui del clip `(capa, columna)` que se está arrastrando.
const MEMORIA_CLIP_ARRASTRADO: &str = "clip_arrastrado";

/// Tamaño de cada celda de la grilla, en píxeles.
const TAM_CELDA: [f32; 2] = [120.0, 68.0];

/// Margen a la izquierda del nombre y los botones de cada capa, dentro de su área.
const PADDING_DE_CAPA: f32 = 8.0;

/// Ancho de la columna con el nombre de cada capa (y del botón «+ Capa» que está sobre ella).
const ANCHO_NOMBRE_DE_CAPA: f32 = 86.0;

/// Resoluciones frecuentes para elegir con un clic.
const PRESETS: [(&str, u32, u32); 5] = [
    ("HD 1280×720", 1280, 720),
    ("Full HD 1920×1080", 1920, 1080),
    ("4K 3840×2160", 3840, 2160),
    ("2 proyectores 3840×1080", 3840, 1080),
    ("Vertical 1080×1920", 1080, 1920),
];

/// Clave en la memoria de egui del archivo que se está arrastrando desde el explorador.
const MEMORIA_ARCHIVO_ARRASTRADO: &str = "archivo_arrastrado";

// ---------------------------------------------------------------------------------------
// Selector de archivos
// ---------------------------------------------------------------------------------------

/// Un selector de archivos abierto, esperando que el usuario elija.
struct DialogoPendiente {
    capa: usize,
    columna: usize,
    /// Por acá llega el archivo elegido (`None` si se canceló). Un `Receiver` no se puede
    /// compartir entre hilos, así que va dentro de un `Mutex`, que Bevy exige en un recurso.
    respuesta: Mutex<Receiver<Option<PathBuf>>>,
}

/// El selector de archivos que se está mostrando, si hay uno.
///
/// Abrir un selector **bloquea** hasta que el usuario elige. Si se hiciera en el hilo
/// principal, toda la aplicación (incluida la salida a los proyectores) se congelaría
/// mientras el diálogo está abierto. Por eso corre en un hilo aparte y el resultado se
/// recoge, frame a frame, con `recibir_archivo_elegido`.
#[derive(Resource, Default)]
pub(crate) struct DialogoArchivo {
    pendiente: Option<DialogoPendiente>,
}

impl DialogoArchivo {
    /// Abre el selector de archivos para la celda `(capa, columna)`. Si ya hay uno abierto,
    /// no hace nada.
    fn abrir(&mut self, capa: usize, columna: usize, carpeta: PathBuf) {
        if self.pendiente.is_some() {
            return;
        }
        let (emisor, receptor) = mpsc::channel();
        std::thread::spawn(move || {
            let elegido = rfd::FileDialog::new()
                .set_title("Elegir un video o una imagen")
                .set_directory(carpeta)
                .add_filter("Video e imagen", &explorador::extensiones_de_medios())
                .pick_file();
            let _ = emisor.send(elegido);
        });
        self.pendiente = Some(DialogoPendiente {
            capa,
            columna,
            respuesta: Mutex::new(receptor),
        });
    }
}

/// Recoge el archivo elegido en el selector y lo pone en la celda.
pub(crate) fn recibir_archivo_elegido(
    mut dialogo: ResMut<DialogoArchivo>,
    mut proyecto: ResMut<ProyectoActual>,
    ruta: Res<RutaProyecto>,
    reproduccion: Res<vm_composicion::Reproduccion>,
    mut ordenes: MessageWriter<OrdenClip>,
) {
    // Se mira primero con acceso de solo lectura (`as_ref`): pedir acceso de escritura
    // marcaría el recurso como modificado en cada frame.
    let Some(pendiente) = dialogo.pendiente.as_ref() else {
        return;
    };
    let (capa, columna) = (pendiente.capa, pendiente.columna);
    let respuesta = match pendiente.respuesta.lock() {
        Ok(receptor) => receptor.try_recv(),
        Err(_) => Err(TryRecvError::Disconnected),
    };

    let elegido = match respuesta {
        Err(TryRecvError::Empty) => return, // el usuario todavía no eligió
        Err(TryRecvError::Disconnected) => {
            dialogo.pendiente = None;
            warn!("el selector de archivos se cerró sin dar respuesta");
            return;
        }
        Ok(elegido) => elegido,
    };
    dialogo.pendiente = None;

    // `None` es que el usuario canceló.
    let Some(archivo) = elegido else { return };

    let guardada = ruta_para_guardar(&ruta.carpeta(), &archivo);
    proyecto
        .0
        .composicion
        .poner_clip(capa, columna, ConfigClip::de_archivo(guardada));

    // Si se reemplazó el clip que estaba sonando, se vuelve a disparar para que suene el nuevo.
    if reproduccion.0.activo.get(capa).copied().flatten() == Some(columna) {
        ordenes.write(OrdenClip(OrdenComposicion::Disparar { capa, columna }));
    }
}

/// ¿Existe el archivo? Se consulta al disco como máximo cada 2 segundos por archivo (la grilla
/// se dibuja en cada frame y no tiene sentido preguntarle al disco 60 veces por segundo).
fn existe_cacheado(ctx: &egui::Context, ruta: &Path) -> bool {
    let id = egui::Id::new(("existe_el_archivo", ruta));
    let ahora = ctx.input(|i| i.time);
    if let Some((cuando, existe)) = ctx.data(|d| d.get_temp::<(f64, bool)>(id))
        && ahora - cuando < 2.0
    {
        return existe;
    }
    let existe = ruta.exists();
    ctx.data_mut(|d| d.insert_temp(id, (ahora, existe)));
    existe
}

/// Los clips `(capa, columna)` cuyo archivo no está.
fn clips_sin_archivo(
    ctx: &egui::Context,
    comp: &vm_core::Composicion,
    carpeta: &Path,
) -> Vec<(usize, usize)> {
    let mut faltan = Vec::new();
    for (capa, l) in comp.capas.iter().enumerate() {
        for (columna, celda) in l.celdas.iter().enumerate() {
            // Solo los archivos pueden faltar: una cámara o una URL no tienen ruta.
            if let Some(clip) = celda
                && let Some(ruta) = clip.fuente.ruta_de_archivo()
                && !existe_cacheado(ctx, &resolver_ruta(carpeta, ruta))
            {
                faltan.push((capa, columna));
            }
        }
    }
    faltan
}

/// El selector de **carpeta** abierto para reubicar archivos, si hay uno (corre en un hilo
/// aparte por la misma razón que `DialogoArchivo`).
#[derive(Resource, Default)]
pub(crate) struct DialogoCarpeta {
    respuesta: Option<Mutex<Receiver<Option<PathBuf>>>>,
}

impl DialogoCarpeta {
    fn abrir(&mut self, inicial: PathBuf) {
        if self.respuesta.is_some() {
            return;
        }
        let (emisor, receptor) = mpsc::channel();
        std::thread::spawn(move || {
            let elegida = rfd::FileDialog::new()
                .set_title("Carpeta donde están los videos")
                .set_directory(inicial)
                .pick_folder();
            let _ = emisor.send(elegida);
        });
        self.respuesta = Some(Mutex::new(receptor));
    }
}

/// Con la carpeta elegida, busca por nombre los videos que faltan y arregla sus rutas.
pub(crate) fn recibir_carpeta_de_reubicacion(
    mut dialogo: ResMut<DialogoCarpeta>,
    mut proyecto: ResMut<ProyectoActual>,
    ruta: Res<RutaProyecto>,
    mut estado: ResMut<vm_proyecto::EstadoProyecto>,
) {
    let Some(receptor) = dialogo.respuesta.as_ref() else {
        return;
    };
    let respuesta = match receptor.lock() {
        Ok(r) => r.try_recv(),
        Err(_) => Err(TryRecvError::Disconnected),
    };
    let elegida = match respuesta {
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => None,
        Ok(elegida) => elegida,
    };
    dialogo.respuesta = None;
    let Some(carpeta_nueva) = elegida else { return };

    let carpeta = ruta.carpeta();
    let (mut faltaban, mut encontrados) = (0, 0);
    for capa in &mut proyecto.0.composicion.capas {
        for clip in capa.celdas.iter_mut().flatten() {
            let (Fuente::Video { ruta: guardada } | Fuente::Imagen { ruta: guardada }) =
                &mut clip.fuente
            else {
                continue;
            };
            let actual = resolver_ruta(&carpeta, guardada);
            if actual.exists() {
                continue;
            }
            faltaban += 1;
            if let Some(nombre) = actual.file_name()
                && let Some(hallado) = explorador::buscar_por_nombre(&carpeta_nueva, nombre, 6)
            {
                *guardada = ruta_para_guardar(&carpeta, &hallado);
                encontrados += 1;
            }
        }
    }
    estado.mensaje = format!("Reubicados {encontrados} de {faltaban} archivos que faltaban");
}

/// Las cámaras y dispositivos de captura que se encontraron, y el diálogo para escribir la
/// dirección de una transmisión de red. Buscar dispositivos puede tardar un instante, así que
/// se hace en un hilo aparte y el resultado se recoge frame a frame.
#[derive(Resource, Default)]
pub(crate) struct Dispositivos {
    lista: Vec<DispositivoDeVideo>,
    buscando: Option<Mutex<Receiver<Vec<DispositivoDeVideo>>>>,
    /// La celda `(capa, columna)` para la que se está pidiendo una URL, si hay un diálogo abierto.
    url_para: Option<(usize, usize)>,
    texto_url: String,
}

impl Dispositivos {
    /// Vuelve a buscar las cámaras (si ya hay una búsqueda en curso, no hace nada).
    fn actualizar(&mut self) {
        if self.buscando.is_some() {
            return;
        }
        let (emisor, receptor) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = emisor.send(listar_dispositivos());
        });
        self.buscando = Some(Mutex::new(receptor));
    }

    /// Recoge el resultado de la búsqueda, si ya terminó.
    fn recibir(&mut self) {
        let Some(receptor) = self.buscando.as_ref() else {
            return;
        };
        let respuesta = match receptor.lock() {
            Ok(r) => r.try_recv(),
            Err(_) => Err(TryRecvError::Disconnected),
        };
        match respuesta {
            Ok(lista) => {
                self.lista = lista;
                self.buscando = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.buscando = None,
        }
    }
}

/// El menú que se abre al hacer clic en una celda vacía: de dónde sale la imagen.
fn menu_de_fuentes(
    ui: &mut egui::Ui,
    dispositivos: &mut Dispositivos,
    capa: usize,
    columna: usize,
    acciones: &mut Vec<Accion>,
) {
    use egui_phosphor::regular as icono_de;
    ui.set_min_width(260.0);
    if ui
        .button(con_icono(
            ui,
            icono_de::FILM_STRIP,
            azul(),
            "Archivo de video…",
        ))
        .clicked()
    {
        acciones.push(Accion::ElegirArchivo { capa, columna });
        ui.close();
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(icono(icono_de::VIDEO_CAMERA, rojo()));
        ui.label(texto_negrita("Cámaras y capturas"));
        if dispositivos.buscando.is_some() {
            ui.weak("buscando…");
        }
    });
    if dispositivos.lista.is_empty() && dispositivos.buscando.is_none() {
        ui.weak("No se encontró ninguna cámara.");
    }
    for d in dispositivos.lista.clone() {
        let texto = format!("{}  ({})", d.nombre, d.dispositivo);
        if ui
            .button(con_icono(ui, icono_de::CAMERA, rojo(), &texto))
            .clicked()
        {
            acciones.push(Accion::PonerFuente {
                capa,
                columna,
                clip: ConfigClip::de_camara(d.dispositivo, d.nombre),
            });
            ui.close();
        }
    }
    ui.weak(
        "Para traer la salida de otro programa, usá una cámara virtual (OBS: «Iniciar cámara virtual»).",
    );
    ui.separator();
    if ui
        .button(con_icono(
            ui,
            icono_de::MONITOR_ARROW_UP,
            azul(),
            "Captura de pantalla o ventana…",
        ))
        .on_hover_text(
            "El escritorio te pregunta qué compartir (pantalla o ventana) y la próxima vez se \
             acuerda de la elección",
        )
        .clicked()
    {
        acciones.push(Accion::PonerFuente {
            capa,
            columna,
            clip: ConfigClip::de_captura("Captura".into()),
        });
        ui.close();
    }
    if ui
        .button(con_icono(
            ui,
            icono_de::BROADCAST,
            naranja(),
            "Transmisión de red…",
        ))
        .on_hover_text("rtsp://, srt:// o http://")
        .clicked()
    {
        dispositivos.url_para = Some((capa, columna));
        dispositivos.texto_url.clear();
        ui.close();
    }
}

/// El botón de un clip que no es un archivo (cámara o transmisión): un ícono y el aviso «en
/// vivo», con el nombre debajo, del mismo tamaño que cualquier otra celda.
fn boton_en_vivo(ui: &mut egui::Ui, clip: &ConfigClip, sonando: bool) -> egui::Response {
    use egui_phosphor::regular as icono_de;
    let glifo = match clip.fuente {
        Fuente::Camara { .. } => icono_de::CAMERA,
        Fuente::Captura { .. } => icono_de::APP_WINDOW,
        _ => icono_de::BROADCAST,
    };
    dibujar_celda(
        ui,
        ContenidoDeCelda::EnVivo {
            glifo,
            nombre: &clip.nombre,
        },
        sonando,
    )
}

/// Alto de la franja con el nombre, debajo de la miniatura. Todas las celdas la reservan, tengan
/// clip o no: así las filas miden lo mismo y nada «escalona» la grilla.
const ALTO_NOMBRE: f32 = 22.0;

/// Lo que se dibuja dentro de una celda de la grilla.
enum ContenidoDeCelda<'a> {
    /// Sin clip: un «+».
    Vacia,
    /// La miniatura del clip (se acomoda sin deformarse) y su nombre.
    Miniatura {
        textura: egui::TextureId,
        ancho: u32,
        alto: u32,
        nombre: &'a str,
    },
    /// Una cámara, una captura o una transmisión.
    EnVivo { glifo: &'a str, nombre: &'a str },
    /// Todavía no hay miniatura (o no se pudo sacar): un texto corto y el nombre.
    Texto { texto: &'a str, nombre: &'a str },
}

/// Dibuja una celda de la grilla. **Todas ocupan exactamente lo mismo** (`TAM_CELDA` más la
/// franja del nombre): se reserva un rectángulo fijo y se pinta adentro, en vez de dejar que
/// cada botón decida su tamaño según lo que tenga.
fn dibujar_celda(ui: &mut egui::Ui, contenido: ContenidoDeCelda, sonando: bool) -> egui::Response {
    let tamano = egui::vec2(TAM_CELDA[0], TAM_CELDA[1] + ALTO_NOMBRE);
    let (rect, respuesta) = ui.allocate_exact_size(tamano, egui::Sense::click_and_drag());
    let area = egui::Rect::from_min_size(rect.min, egui::vec2(TAM_CELDA[0], TAM_CELDA[1]));
    let visuales = *ui.style().interact(&respuesta);
    let relleno = if sonando {
        verde().gamma_multiply(0.45)
    } else {
        visuales.weak_bg_fill
    };
    let pintor = ui.painter().clone();
    pintor.rect(
        area,
        visuales.corner_radius,
        relleno,
        visuales.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let color_texto = visuales.fg_stroke.color;
    let nombre = match contenido {
        ContenidoDeCelda::Vacia => {
            pintor.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(18.0),
                color_texto,
            );
            None
        }
        ContenidoDeCelda::Miniatura {
            textura,
            ancho,
            alto,
            nombre,
        } => {
            // La miniatura entra en la celda conservando su proporción (también una vertical).
            let escala = (area.width() / ancho as f32).min(area.height() / alto as f32);
            let imagen = egui::Rect::from_center_size(
                area.center(),
                egui::vec2(ancho as f32 * escala, alto as f32 * escala),
            );
            pintor.image(
                textura,
                imagen,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            Some(nombre)
        }
        ContenidoDeCelda::EnVivo { glifo, nombre } => {
            pintor.text(
                area.center() - egui::vec2(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                glifo,
                egui::FontId::proportional(22.0),
                rojo(),
            );
            pintor.text(
                area.center() + egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "EN VIVO",
                egui::FontId::proportional(13.0),
                rojo(),
            );
            Some(nombre)
        }
        ContenidoDeCelda::Texto { texto, nombre } => {
            pintor.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                texto,
                egui::FontId::proportional(12.0),
                color_texto,
            );
            Some(nombre)
        }
    };
    if sonando {
        // El clip que suena se marca además con un borde, porque el relleno verde apenas se ve
        // detrás de la imagen.
        pintor.rect_stroke(
            area,
            4.0,
            egui::Stroke::new(3.0, verde()),
            egui::StrokeKind::Outside,
        );
    }
    if let Some(nombre) = nombre {
        // Una sola línea: un nombre largo se recorta con «…» en vez de ensanchar la celda.
        let mut trabajo = egui::text::LayoutJob::simple_singleline(
            nombre.to_string(),
            egui::FontId::proportional(11.0),
            crate::tema::tema().texto,
        );
        trabajo.wrap = egui::text::TextWrapping {
            max_width: TAM_CELDA[0],
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = pintor.layout_job(trabajo);
        pintor.galley(
            egui::pos2(rect.left(), area.bottom() + 5.0),
            galley,
            crate::tema::tema().texto,
        );
    }
    respuesta
}

/// La capa elegida en la grilla: la que muestra sus propiedades el panel de abajo.
#[derive(Resource, Default)]
pub(crate) struct CapaElegida(pub(crate) usize);

/// El clip elegido en la grilla, `(capa, columna)`: el que muestra la zona de Clip.
#[derive(Resource, Default)]
pub(crate) struct ClipElegido(pub(crate) Option<(usize, usize)>);

/// El explorador de archivos: qué carpeta se mira y qué hay en ella.
#[derive(Resource, Default)]
pub(crate) struct Explorador {
    carpeta: Option<PathBuf>,
    entradas: Vec<EntradaArchivo>,
    error: Option<String>,
    filtro: String,
}

impl Explorador {
    /// Va a una carpeta. Si no se puede leer, se queda donde estaba y guarda el motivo.
    fn ir_a(&mut self, carpeta: PathBuf) {
        match explorador::listar(&carpeta) {
            Ok(entradas) => {
                self.entradas = entradas;
                self.error = None;
                self.carpeta = Some(carpeta);
            }
            Err(e) => self.error = Some(format!("No se pudo abrir {}: {e}", carpeta.display())),
        }
    }

    /// Vuelve a leer la carpeta actual (por si se agregaron archivos).
    fn recargar(&mut self) {
        if let Some(carpeta) = self.carpeta.clone() {
            self.ir_a(carpeta);
        }
    }
}

// ---------------------------------------------------------------------------------------
// La pestaña
// ---------------------------------------------------------------------------------------

/// Lo que necesita la pestaña, agrupado para no pasar una decena de parámetros.
///
/// `proyecto` es el `ResMut` completo: convertirlo en `&mut ProyectoActual` lo marcaría
/// como modificado al instante; así solo se marca cuando de verdad se escribe en él.
pub(crate) struct ContextoComposicion<'a, 'w, 'x> {
    /// La capa elegida, cuyas propiedades se muestran en el panel fijo.
    pub(crate) capa_elegida: &'a mut usize,
    /// El clip elegido (la zona de Clip muestra sus propiedades).
    pub(crate) clip_elegido: &'a mut Option<(usize, usize)>,
    pub(crate) explorador: &'a mut Explorador,
    pub(crate) dispositivos: &'a mut Dispositivos,
    /// Dónde va el video de cada capa.
    pub(crate) progreso: &'a ProgresoDeCapas,
    /// El servidor OSC (control externo): si está prendido y en qué puerto, y qué le pasa.
    pub(crate) config_osc: &'a mut ResMut<'x, ConfigOsc>,
    pub(crate) estado_osc: &'a EstadoOsc,
    /// A dónde se manda el estado por OSC (el celular que controla).
    pub(crate) destino_osc: &'a mut ResMut<'x, DestinoOsc>,
    /// El controlador MIDI: cuál es, qué le pasa, qué puertos hay y si se está aprendiendo.
    pub(crate) config_midi: &'a mut ResMut<'x, ConfigMidi>,
    pub(crate) estado_midi: &'a EstadoMidi,
    pub(crate) puertos_midi: &'a mut ResMut<'x, PuertosMidi>,
    pub(crate) aprendizaje_midi: &'a mut ResMut<'x, AprendizajeMidi>,
    pub(crate) senales_audio: &'a SenalesDeAudio,
    pub(crate) aplicaciones_audio: &'a mut AplicacionesQueSuenan,
    /// Qué clip falló en cada capa (se marca en la grilla).
    pub(crate) errores: &'a ErroresDeCapas,
    pub(crate) dialogo_carpeta: &'a mut DialogoCarpeta,
    /// La textura de cada capa (para saber el tamaño del video que suena).
    pub(crate) capas_texturas: &'a TexturasDeCapas,
    /// Las capas que se quitaron en este frame; quien llama avisa de cada una.
    pub(crate) capas_quitadas: &'a mut Vec<usize>,
    /// Las capas que se movieron en este frame: `(desde, hasta)`.
    pub(crate) capas_movidas: &'a mut Vec<(usize, usize)>,
    pub(crate) texturas: &'a mut EguiUserTextures,
    pub(crate) textura: Option<&'a TexturaComposicion>,
    pub(crate) proyecto: &'a mut ResMut<'w, ProyectoActual>,
    pub(crate) ruta: &'a RutaProyecto,
    pub(crate) miniaturas: &'a Miniaturas,
    pub(crate) reproduccion: &'a mut EstadoReproduccion,
    pub(crate) mensaje: &'a str,
    pub(crate) dialogo: &'a mut DialogoArchivo,
    /// Las órdenes de disparo que se pidieron en este frame; quien llama las envía.
    pub(crate) ordenes: &'a mut Vec<OrdenComposicion>,
}

/// Cambios a la composición que se piden dentro del dibujo de la grilla y se aplican
/// después (no se puede modificar la composición mientras se la está recorriendo).
enum Accion {
    ElegirArchivo {
        capa: usize,
        columna: usize,
    },
    QuitarClip {
        capa: usize,
        columna: usize,
    },
    AgregarColumna,
    QuitarColumna(usize),
    AgregarCapa,
    /// Quitar la capa con ese índice (cualquiera, no solo la última).
    QuitarCapa(usize),
    /// Mover una capa a otra posición (se arrastró su fila).
    MoverCapa {
        desde: usize,
        hasta: usize,
    },
    /// Poner un clip ya armado (una cámara, una transmisión) en una celda.
    PonerFuente {
        capa: usize,
        columna: usize,
        clip: ConfigClip,
    },
    /// Poner un video (arrastrado desde el explorador) en una celda.
    PonerClip {
        capa: usize,
        columna: usize,
        archivo: PathBuf,
    },
    /// Mover un clip a otra celda `(capa, columna)`; si está ocupada, se intercambian.
    MoverClip {
        desde: (usize, usize),
        hasta: (usize, usize),
    },
    /// Volver a mostrar el selector del escritorio para elegir otra ventana o pantalla.
    ReelegirCaptura {
        capa: usize,
        columna: usize,
    },
}

/// Qué parte del alto disponible ocupa la zona de abajo al arrancar (después se arrastra su
/// borde de arriba).
const FRACCION_ALTO_ZONA_INFERIOR: f32 = 0.60;
/// Alto mínimo de la grilla de clips (lo que no puede quitarle la zona de abajo al agrandarse).
const ALTO_MINIMO_DE_LA_GRILLA: f32 = 190.0;
/// Alto mínimo de la zona de abajo.
const ALTO_MINIMO_DE_LA_ZONA_INFERIOR: f32 = 240.0;

/// Cómo se reparte el ancho al arrancar entre las cuatro columnas de la zona de abajo
/// (previsualizaciones, propiedades, clip y archivos) y lo mínimo que mide cada una. Las
/// previsualizaciones son lo más grande.
const FRACCIONES_INICIALES: [f32; reparto::COLUMNAS] = [0.277, 0.278, 0.256, 0.189];
const MINIMOS_DE_COLUMNA: [f32; reparto::COLUMNAS] = [200.0, 240.0, 250.0, 190.0];

/// Grosor de la franja donde se agarra un divisor con el mouse.
const GROSOR_DEL_DIVISOR: f32 = 8.0;
/// Margen interior de las columnas de la zona de abajo del lado de un divisor (del lado de la
/// ventana no llevan: ya lo da el panel central, y así se alinean con las pestañas y la grilla).
pub(crate) const MARGEN_DE_COLUMNA: f32 = 10.0;
/// Margen de arriba y de abajo de cada columna.
const MARGEN_VERTICAL_DE_COLUMNA: f32 = 8.0;

/// Un divisor ARRASTRABLE: una franja de `GROSOR_DEL_DIVISOR` donde el cursor cambia y que se
/// pinta con una línea, resaltada al pasar el mouse. Devuelve la respuesta (con
/// `drag_delta()` para saber cuánto se movió).
fn divisor(
    ui: &mut egui::Ui,
    franja: egui::Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    vertical: bool,
) -> egui::Response {
    let respuesta = ui.interact(franja, ui.id().with(("divisor", id)), egui::Sense::drag());
    if respuesta.hovered() || respuesta.dragged() {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        });
    }
    let color = if respuesta.hovered() || respuesta.dragged() {
        crate::tema::tema().primario
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    let trazo = egui::Stroke::new(if respuesta.dragged() { 2.0 } else { 1.0 }, color);
    let (a, b) = if vertical {
        (franja.center_top(), franja.center_bottom())
    } else {
        (franja.left_center(), franja.right_center())
    };
    ui.painter().line_segment([a, b], trazo);
    respuesta
}

/// La zona de abajo: las cuatro columnas con divisores arrastrables entre ellas y el borde de
/// arriba también arrastrable. Lo que el usuario eligió se guarda como FRACCIONES (en la memoria
/// de egui) y el tamaño de cada cuadro se calcula desde cero con el espacio que hay ahora
/// (`reparto`): al redimensionar la ventana todo se acomoda en el mismo cuadro, sin depender de
/// lo que se dibujó antes.
fn zona_inferior(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let total = ui.available_height();
    let id_alto = egui::Id::new("fraccion_alto_zona_inferior");
    let id_columnas = egui::Id::new("fracciones_columnas_zona_inferior");
    let fraccion_alto = ui
        .data(|d| d.get_temp::<f32>(id_alto))
        .unwrap_or(FRACCION_ALTO_ZONA_INFERIOR);
    let alto = reparto::alto_de_la_zona(
        total,
        fraccion_alto,
        ALTO_MINIMO_DE_LA_ZONA_INFERIOR,
        ALTO_MINIMO_DE_LA_GRILLA,
    );

    egui::Panel::bottom("zona_inferior_composicion")
        .resizable(false)
        .exact_size(alto)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let area = ui.available_rect_before_wrap();

            // El borde de arriba: arrastrándolo cambia el alto de la zona.
            let franja =
                egui::Rect::from_min_size(area.min, egui::vec2(area.width(), GROSOR_DEL_DIVISOR));
            let arrastre = divisor(ui, franja, "alto", false);
            if arrastre.dragged() && total > 0.0 {
                let nuevo = (alto - arrastre.drag_delta().y) / total;
                ui.data_mut(|d| d.insert_temp(id_alto, nuevo));
            }

            // Las cuatro columnas, con el ancho que sale del reparto.
            let cuerpo =
                egui::Rect::from_min_max(area.min + egui::vec2(0.0, GROSOR_DEL_DIVISOR), area.max);
            let mut fracciones = ui
                .data(|d| d.get_temp::<[f32; reparto::COLUMNAS]>(id_columnas))
                .unwrap_or(FRACCIONES_INICIALES);
            let anchos = reparto::repartir(cuerpo.width(), fracciones, MINIMOS_DE_COLUMNA);
            let mut x = cuerpo.left();
            let mut limites = [cuerpo.left(); reparto::COLUMNAS + 1];
            for i in 0..reparto::COLUMNAS {
                x += anchos[i];
                limites[i + 1] = x;
            }

            // Los divisores entre columnas.
            for i in 0..reparto::COLUMNAS - 1 {
                let franja = egui::Rect::from_min_max(
                    egui::pos2(limites[i + 1] - GROSOR_DEL_DIVISOR / 2.0, cuerpo.top()),
                    egui::pos2(limites[i + 1] + GROSOR_DEL_DIVISOR / 2.0, cuerpo.bottom()),
                );
                let r = divisor(ui, franja, ("columna", i), true);
                if r.dragged() {
                    fracciones = reparto::mover_divisor(
                        fracciones,
                        i,
                        r.drag_delta().x,
                        cuerpo.width(),
                        MINIMOS_DE_COLUMNA,
                    );
                    ui.data_mut(|d| d.insert_temp(id_columnas, fracciones));
                }
            }

            // El contenido de cada columna, en su rectángulo y recortado a él: nunca la agranda.
            let rect_de = |i: usize| {
                let izquierda = if i == 0 { 0.0 } else { MARGEN_DE_COLUMNA };
                let derecha = if i == reparto::COLUMNAS - 1 {
                    0.0
                } else {
                    MARGEN_DE_COLUMNA
                };
                egui::Rect::from_min_max(
                    egui::pos2(
                        limites[i] + izquierda,
                        cuerpo.top() + MARGEN_VERTICAL_DE_COLUMNA,
                    ),
                    egui::pos2(
                        limites[i + 1] - derecha,
                        cuerpo.bottom() - MARGEN_VERTICAL_DE_COLUMNA,
                    ),
                )
            };
            contenido_en_rect(ui, rect_de(0), |ui| previsualizaciones(ui, ctx));
            contenido_en_rect(ui, rect_de(1), |ui| propiedades(ui, ctx));
            contenido_en_rect(ui, rect_de(2), |ui| zona_de_clip(ui, ctx));
            contenido_en_rect(ui, rect_de(3), |ui| panel_explorador(ui, ctx));
            ui.allocate_rect(area, egui::Sense::hover());
        });
}

/// La pantalla de composición, con la disposición de Resolume: la grilla de clips arriba y,
/// abajo, las previsualizaciones, las propiedades (de la capa o de la composición), las del clip
/// elegido y el explorador de archivos, con divisores arrastrables.
pub(crate) fn pestana_composicion(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    // La zona de abajo se dibuja antes que el resto para que egui le reserve su lugar.
    zona_inferior(ui, ctx);

    egui::ScrollArea::both()
        .id_salt("grilla_de_clips")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let comp = ctx.proyecto.0.composicion.clone();
            let faltan = clips_sin_archivo(ui.ctx(), &comp, &ctx.ruta.carpeta());
            if !faltan.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(icono(egui_phosphor::regular::WARNING, rojo()));
                    ui.label(format!(
                        "{} clip(s) sin su archivo (se movió o se borró).",
                        faltan.len()
                    ));
                    if ui
                        .button(con_icono(
                            ui,
                            egui_phosphor::regular::MAGNIFYING_GLASS,
                            azul(),
                            "Reubicar…",
                        ))
                        .on_hover_text(
                            "Elegí la carpeta donde están los videos: se buscan por nombre en ella \
                             y en sus subcarpetas.",
                        )
                        .clicked()
                    {
                        ctx.dialogo_carpeta.abrir(ctx.ruta.carpeta());
                    }
                });
            }
            grilla(ui, ctx);
            if !ctx.mensaje.is_empty() {
                ui.colored_label(rojo(), ctx.mensaje);
            }
        });
}

/// Las propiedades de la capa elegida o las de la composición, con dos pestañas. Tiene
/// scroll: si hay más opciones que alto, nada se corre ni queda tapado.
fn propiedades(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    use egui_phosphor::regular as icono_de;
    let id = egui::Id::new("pestana_de_propiedades");
    // false = capa, true = composición.
    let mut de_composicion = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    ui.horizontal(|ui| {
        let capa = con_icono(ui, icono_de::STACK, naranja(), "Capa");
        if crate::inspector::elegible(ui, !de_composicion, capa).clicked() {
            de_composicion = false;
        }
        let composicion = con_icono(ui, icono_de::SLIDERS, azul(), "Composición");
        if crate::inspector::elegible(ui, de_composicion, composicion).clicked() {
            de_composicion = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, de_composicion));
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("propiedades_con_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if de_composicion {
                selector_de_resolucion(ui, ctx);
                ui.add_space(6.0);
                selector_de_transicion(ui, ctx);
            } else {
                propiedades_de_capa(ui, ctx);
            }
        });
}

/// La zona del clip elegido: su nombre, de dónde viene, si suena y qué se puede hacer con él.
fn zona_de_clip(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    use egui_phosphor::regular as icono_de;
    ui.horizontal(|ui| {
        ui.label(icono(icono_de::FILM_STRIP, azul()));
        ui.label(texto_negrita("Clip"));
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("zona_del_clip")
        .auto_shrink([false, false])
        .show(ui, |ui| contenido_del_clip(ui, ctx));
}

/// Lo que muestra la zona del clip elegido (dentro de un área con scroll: es más largo que
/// el espacio de abajo).
fn contenido_del_clip(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    use egui_phosphor::regular as icono_de;

    let Some((capa, columna)) = *ctx.clip_elegido else {
        ui.weak("Elegí un clip de la grilla para ver sus propiedades.");
        return;
    };
    let comp = &ctx.proyecto.0.composicion;
    let Some(clip) = comp.clip(capa, columna).cloned() else {
        ui.weak("La celda elegida está vacía.");
        return;
    };
    let nombre_de_capa = comp
        .capas
        .get(capa)
        .map(|c| c.nombre.clone())
        .unwrap_or_default();
    let nombre_de_columna = comp
        .columnas
        .get(columna)
        .map(|c| c.nombre.clone())
        .unwrap_or_default();
    // De dónde sale la imagen: un archivo (con su ruta) o una fuente en vivo.
    let descripcion = clip.fuente.descripcion();
    let en_vivo = clip.fuente.es_en_vivo();
    let archivo = clip
        .fuente
        .ruta_de_archivo()
        .map(|r| resolver_ruta(&ctx.ruta.carpeta(), r));
    let sonando = ctx.reproduccion.activo.get(capa).copied().flatten() == Some(columna);

    // Lo más usado va primero: reproducción, pausa, velocidad, tramo y audio.
    controles_de_reproduccion(ui, ctx, capa, columna, sonando);
    ui.separator();

    let mut nombre = clip.nombre.clone();
    let mut cambio_nombre = false;
    let mut ocultar_cursor = matches!(
        clip.fuente,
        Fuente::Captura {
            ocultar_cursor: true,
            ..
        }
    );
    let mut cambio_cursor = false;
    fila(ui, "Nombre", |ui| {
        cambio_nombre = campo_de_texto(ui, &mut nombre).changed();
    });
    fila(ui, "Lugar", |ui| {
        ui.add(
            egui::Label::new(format!("{nombre_de_capa} · columna {nombre_de_columna}")).truncate(),
        );
    });
    fila(ui, "Estado", |ui| {
        if sonando {
            ui.label(icono(icono_de::PLAY, verde()))
                .on_hover_text("Este clip está sonando");
        } else {
            ui.weak("detenido");
        }
    });
    if sonando && let Some(t) = ctx.capas_texturas.0.get(capa) {
        fila(ui, "Video", |ui| {
            ui.label(format!("{}×{}", t.ancho, t.alto));
        });
    }
    fila(ui, if en_vivo { "Fuente" } else { "Archivo" }, |ui| {
        ui.add(egui::Label::new(descripcion.as_str()).truncate())
            .on_hover_text(
                archivo
                    .as_ref()
                    .map_or(descripcion.clone(), |a| a.display().to_string()),
            );
    });
    if archivo.as_ref().is_some_and(|a| !a.exists()) {
        fila(ui, "", |ui| {
            ui.colored_label(rojo(), "El archivo no está");
        });
    }
    // Solo las capturas de pantalla tienen cursor.
    if let Fuente::Captura { .. } = &clip.fuente {
        fila(ui, "Cursor", |ui| {
            cambio_cursor = ui
                .checkbox(&mut ocultar_cursor, "Ocultar")
                .on_hover_text(
                    "No dibujar el puntero del mouse en la imagen. Con el puntero adentro, \
                     una ventana que no se repinta casi no manda cuadros y se ve a saltos.",
                )
                .changed();
        });
    }
    ui.add_space(6.0);

    let mut quitar = false;
    botones_iguales(ui, 2, |ui, i| {
        if i == 0 {
            if boton(ui, con_icono(ui, icono_de::PLAY, verde(), "Disparar")).clicked() {
                ctx.ordenes
                    .push(OrdenComposicion::Disparar { capa, columna });
            }
        } else if ui
            .add_enabled_ui(sonando, |ui| {
                boton(ui, con_icono(ui, icono_de::STOP, rojo(), "Parar"))
            })
            .inner
            .on_hover_text("Detiene la capa de este clip")
            .clicked()
        {
            ctx.ordenes.push(OrdenComposicion::Detener { capa });
        }
    });
    botones_iguales(ui, 2, |ui, i| {
        if i == 0 {
            if boton(
                ui,
                con_icono(ui, icono_de::FOLDER_OPEN, naranja(), "Cambiar…"),
            )
            .on_hover_text("Elegir otro archivo para este clip")
            .clicked()
            {
                ctx.dialogo.abrir(capa, columna, ctx.ruta.carpeta());
            }
        } else if boton(ui, con_icono(ui, icono_de::TRASH, rojo(), "Quitar")).clicked() {
            quitar = true;
        }
    });

    // Solo se escribe en el proyecto si algo cambió.
    if cambio_nombre
        && let Some(Some(clip)) = ctx
            .proyecto
            .0
            .composicion
            .capas
            .get_mut(capa)
            .and_then(|c| c.celdas.get_mut(columna))
    {
        clip.nombre = nombre;
    }
    if cambio_cursor
        && let Some(Some(clip)) = ctx
            .proyecto
            .0
            .composicion
            .capas
            .get_mut(capa)
            .and_then(|c| c.celdas.get_mut(columna))
        && let Fuente::Captura {
            ocultar_cursor: actual,
            ..
        } = &mut clip.fuente
    {
        *actual = ocultar_cursor;
        // Si suena, se vuelve a abrir (con el token guardado, sin preguntar) para que el
        // cambio se vea enseguida.
        if sonando {
            ctx.ordenes
                .push(OrdenComposicion::Disparar { capa, columna });
        }
    }
    if quitar {
        ctx.proyecto.0.composicion.quitar_clip(capa, columna);
        *ctx.clip_elegido = None;
    }
}

/// «m:ss» a partir de segundos.
fn minutos_y_segundos(segundos: f64) -> String {
    let total = segundos.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// La barra de progreso del clip: se ve el tramo elegido, dónde va el video y, con un clic o
/// arrastrando, se salta a otro punto.
fn barra_de_progreso(
    ui: &mut egui::Ui,
    ordenes: &mut Vec<OrdenComposicion>,
    capa: usize,
    ajustes: &AjustesClip,
    progreso: Option<(f64, f64)>,
    sonando: bool,
) {
    let (rect, respuesta) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 18.0),
        egui::Sense::click_and_drag(),
    );
    let pintor = ui.painter();
    pintor.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    // El tramo que se reproduce.
    let x = |fraccion: f32| rect.left() + rect.width() * fraccion;
    let tramo = egui::Rect::from_min_max(
        egui::pos2(x(ajustes.inicio), rect.top()),
        egui::pos2(x(ajustes.fin), rect.bottom()),
    );
    pintor.rect_filled(
        tramo,
        4.0,
        crate::tema::tema().primario.gamma_multiply(0.28),
    );
    // Dónde va el video.
    if let Some((posicion, duracion)) = progreso
        && duracion > 0.0
    {
        let marca = x((posicion / duracion).clamp(0.0, 1.0) as f32);
        pintor.line_segment(
            [
                egui::pos2(marca, rect.top()),
                egui::pos2(marca, rect.bottom()),
            ],
            egui::Stroke::new(2.5, crate::tema::tema().primario),
        );
    }
    if sonando && (respuesta.hovered() || respuesta.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // Un clic salta enseguida; al arrastrar, se salta como mucho cada décima de segundo
    // (cada salto detiene y rearma el video).
    if sonando && let Some(pos) = respuesta.interact_pointer_pos() {
        let id = respuesta.id.with("ultimo_salto");
        let ahora = ui.input(|i| i.time);
        let ultimo = ui
            .data(|d| d.get_temp::<f64>(id))
            .unwrap_or(f64::NEG_INFINITY);
        if respuesta.clicked() || (respuesta.dragged() && ahora - ultimo > 0.1) {
            let fraccion = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            ordenes.push(OrdenComposicion::Buscar {
                capa,
                milesimas: (fraccion * 1000.0).round() as u32,
            });
            ui.data_mut(|d| d.insert_temp(id, ahora));
        }
    }
    let texto = match progreso {
        Some((p, d)) => format!("{} / {}", minutos_y_segundos(p), minutos_y_segundos(d)),
        None => "—".to_string(),
    };
    ui.weak(texto);
}

/// Cómo se reproduce el clip elegido: progreso, pausa, bucle, velocidad, tramo y audio.
fn controles_de_reproduccion(
    ui: &mut egui::Ui,
    ctx: &mut ContextoComposicion,
    capa: usize,
    columna: usize,
    sonando: bool,
) {
    use egui_phosphor::regular as icono_de;
    let Some(clip) = ctx.proyecto.0.composicion.clip(capa, columna) else {
        return;
    };
    // Una imagen fija no tiene nada que reproducir: ni progreso, ni bucle, ni velocidad, ni
    // audio.
    if clip.fuente.es_imagen() {
        ui.label(texto_negrita("Imagen fija"));
        return;
    }
    let original = clip.ajustes;
    let en_vivo = clip.fuente.es_en_vivo();
    let mut a = original;
    let en_pausa = ctx.reproduccion.pausada.get(capa).copied().unwrap_or(false);
    let progreso = ctx.progreso.0.get(capa).copied().flatten();

    ui.label(texto_negrita(if en_vivo {
        "En vivo"
    } else {
        "Reproducción"
    }));
    // Una imagen en vivo no tiene duración: sin barra de progreso, bucle, velocidad ni tramo.
    if !en_vivo {
        barra_de_progreso(ui, ctx.ordenes, capa, &a, progreso, sonando);
    }
    ui.add_space(2.0);

    // Pausa, bucle y silencio: botones del mismo ancho que reparten todo el ancho.
    let cantidad_de_botones = if en_vivo { 2 } else { 3 };
    botones_iguales(ui, cantidad_de_botones, |ui, i| {
        // En vivo no hay bucle: los dos botones que quedan son pausa y silencio.
        let cual = if en_vivo && i == 1 { 2 } else { i };
        match cual {
            0 => {
                let (glifo, texto, color) = if en_pausa {
                    (icono_de::PLAY, "Seguir", verde())
                } else {
                    (icono_de::PAUSE, "Pausa", naranja())
                };
                if ui
                    .add_enabled_ui(sonando, |ui| boton(ui, con_icono(ui, glifo, color, texto)))
                    .inner
                    .on_hover_text("Pausar o seguir esta capa")
                    .clicked()
                {
                    ctx.ordenes.push(OrdenComposicion::AlternarPausa { capa });
                }
            }
            1 => {
                if boton_elegible(ui, a.bucle, con_icono(ui, icono_de::REPEAT, azul(), "Bucle"))
                    .on_hover_text(
                        "Al llegar al final, vuelve a empezar. Apagado, queda quieto en el último cuadro.",
                    )
                    .clicked()
                {
                    a.bucle = !a.bucle;
                }
            }
            _ => {
                let (glifo, color) = if a.silenciado {
                    (icono_de::SPEAKER_SLASH, rojo())
                } else {
                    (icono_de::SPEAKER_HIGH, verde())
                };
                if boton_elegible(ui, a.silenciado, con_icono(ui, glifo, color, "Silencio"))
                    .on_hover_text("Silenciar el audio de este clip")
                    .clicked()
                {
                    a.silenciado = !a.silenciado;
                }
            }
        }
    });

    if !en_vivo {
        fila(ui, "Velocidad", |ui| {
            let reservado = 40.0 + ui.spacing().item_spacing.x;
            deslizador_con(
                ui,
                &mut a.velocidad,
                VELOCIDAD_MINIMA..=VELOCIDAD_MAXIMA,
                reservado,
                |s| s.logarithmic(true).suffix(" ×").fixed_decimals(2),
            );
            if ui
                .add_sized([40.0, ALTO_FILA], egui::Button::new("1×"))
                .on_hover_text("Velocidad normal")
                .clicked()
            {
                a.velocidad = 1.0;
            }
        });

        // El tramo se edita en porcentaje de la duración del video.
        let (mut inicio, mut fin) = (a.inicio * 100.0, a.fin * 100.0);
        fila(ui, "Entrada", |ui| {
            if deslizador(ui, &mut inicio, 0.0..=100.0, " %", 1, 0.0)
                .on_hover_text("Dónde empieza el tramo que se reproduce")
                .changed()
            {
                a.inicio = inicio / 100.0;
            }
        });
        fila(ui, "Salida", |ui| {
            if deslizador(ui, &mut fin, 0.0..=100.0, " %", 1, 0.0)
                .on_hover_text("Dónde termina el tramo que se reproduce")
                .changed()
            {
                a.fin = fin / 100.0;
            }
        });
    }
    fila(ui, "Volumen", |ui| {
        let mut volumen = a.volumen * 100.0;
        if deslizador(ui, &mut volumen, 0.0..=VOLUMEN_MAXIMO * 100.0, " %", 0, 0.0).changed() {
            a.volumen = volumen / 100.0;
        }
    });

    // Solo se escribe en el proyecto si algo cambió.
    a.normalizar();
    if a != original
        && let Some(Some(clip)) = ctx
            .proyecto
            .0
            .composicion
            .capas
            .get_mut(capa)
            .and_then(|c| c.celdas.get_mut(columna))
    {
        clip.ajustes = a;
    }
}

/// El explorador de archivos: carpetas y videos, con búsqueda. Un video se **arrastra** a
/// una celda de la grilla (en Wayland no se pueden arrastrar archivos desde afuera del
/// programa, por eso se hace desde acá); con doble clic va a la primera celda libre de
/// la capa elegida.
fn panel_explorador(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    use egui_phosphor::regular as icono_de;
    if ctx.explorador.carpeta.is_none() {
        let inicial = ctx.ruta.carpeta();
        ctx.explorador.ir_a(inicial);
    }
    ui.horizontal(|ui| {
        ui.label(icono(icono_de::FOLDER_SIMPLE, amarillo()));
        ui.label(texto_negrita("Archivos"));
    });
    ui.add_space(2.0);

    let mut ir_a: Option<PathBuf> = None;
    let mut poner: Option<PathBuf> = None;
    ui.horizontal(|ui| {
        let superior = ctx
            .explorador
            .carpeta
            .as_ref()
            .and_then(|c| c.parent())
            .map(PathBuf::from);
        if ui
            .add_enabled_ui(superior.is_some(), |ui| {
                boton_cuadrado(ui, icono(icono_de::ARROW_UP, azul()))
            })
            .inner
            .on_hover_text("Carpeta superior")
            .clicked()
        {
            ir_a = superior;
        }
        if boton_cuadrado(ui, icono(icono_de::HOUSE, naranja()))
            .on_hover_text("Carpeta del proyecto")
            .clicked()
        {
            ir_a = Some(ctx.ruta.carpeta());
        }
        if boton_cuadrado(ui, icono(icono_de::ARROWS_CLOCKWISE, verde()))
            .on_hover_text("Volver a leer la carpeta")
            .clicked()
        {
            ctx.explorador.recargar();
        }
        // El buscador ocupa lo que queda de la fila (con un ancho «infinito» se salía de la
        // columna y quedaba recortado).
        ui.add_sized(
            [ui.available_width(), ALTO_FILA],
            egui::TextEdit::singleline(&mut ctx.explorador.filtro).hint_text("Buscar…"),
        );
    });
    if let Some(carpeta) = &ctx.explorador.carpeta {
        ui.add(
            egui::Label::new(egui::RichText::new(carpeta.display().to_string()).weak()).truncate(),
        )
        .on_hover_text(carpeta.display().to_string());
    }
    if let Some(error) = &ctx.explorador.error {
        ui.colored_label(rojo(), error);
    }

    let id_archivo = egui::Id::new(MEMORIA_ARCHIVO_ARRASTRADO);
    egui::ScrollArea::vertical()
        .id_salt("lista_de_archivos")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entrada in explorador::filtrar(&ctx.explorador.entradas, &ctx.explorador.filtro) {
                let (glifo, color) = if entrada.es_carpeta {
                    (icono_de::FOLDER_SIMPLE, amarillo())
                } else if vm_core::es_ruta_de_imagen(&entrada.ruta) {
                    (icono_de::IMAGE, verde())
                } else {
                    (icono_de::FILM_STRIP, azul())
                };
                let texto = con_icono(ui, glifo, color, &entrada.nombre);
                let fila = ui.add(
                    egui::Label::new(texto)
                        .truncate()
                        .sense(egui::Sense::click_and_drag()),
                );
                // El resaltado aparece y se va con un fundido corto.
                let brillo = ui.ctx().animate_bool_with_time(
                    fila.id.with("resaltado"),
                    fila.hovered(),
                    0.12,
                );
                if brillo > 0.01 {
                    ui.painter().rect_filled(
                        fila.rect.expand2(egui::vec2(3.0, 1.0)),
                        3.0,
                        crate::tema::tema().primario.gamma_multiply(0.2 * brillo),
                    );
                }
                if fila.hovered() {
                    ui.ctx().set_cursor_icon(if entrada.es_carpeta {
                        egui::CursorIcon::PointingHand
                    } else {
                        egui::CursorIcon::Grab
                    });
                }
                if entrada.es_carpeta {
                    if fila.clicked() {
                        ir_a = Some(entrada.ruta.clone());
                    }
                } else {
                    fila.clone().on_hover_text(format!(
                        "{}\n{}\nArrastralo a una celda de la grilla",
                        entrada.ruta.display(),
                        explorador::tamano_legible(entrada.bytes)
                    ));
                    if fila.drag_started() {
                        let ruta = entrada.ruta.clone();
                        ui.data_mut(|d| d.insert_temp(id_archivo, ruta));
                    }
                    if fila.double_clicked() {
                        poner = Some(entrada.ruta.clone());
                    }
                }
            }
        });

    if let Some(carpeta) = ir_a {
        ctx.explorador.filtro.clear();
        ctx.explorador.ir_a(carpeta);
    }
    // Doble clic en un video: va a la primera celda libre de la capa elegida.
    if let Some(archivo) = poner {
        let capa = *ctx.capa_elegida;
        let libre = ctx
            .proyecto
            .0
            .composicion
            .capas
            .get(capa)
            .and_then(|c| c.celdas.iter().position(Option::is_none));
        if let Some(columna) = libre {
            let guardada = ruta_para_guardar(&ctx.ruta.carpeta(), &archivo);
            ctx.proyecto
                .0
                .composicion
                .poner_clip(capa, columna, ConfigClip::de_archivo(guardada));
            *ctx.clip_elegido = Some((capa, columna));
        }
    }
}

/// Resolución de la composición: campos numéricos (se escribe el valor) y atajos.
fn selector_de_resolucion(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let (mut ancho, mut alto) = {
        let comp = &ctx.proyecto.0.composicion;
        (comp.ancho, comp.alto)
    };
    let mut cambio = false;

    seccion(ui, "Resolución");
    fila(ui, "Tamaño", |ui| {
        // Ancho y alto del mismo tamaño, repartidos en todo el ancho.
        ui.columns(2, |c| {
            cambio |= c[0]
                .add_sized(
                    [c[0].available_width(), ALTO_FILA],
                    campo_de_resolucion(&mut ancho).prefix("ancho: "),
                )
                .on_hover_text("Hacé clic y escribí el ancho")
                .changed();
            cambio |= c[1]
                .add_sized(
                    [c[1].available_width(), ALTO_FILA],
                    campo_de_resolucion(&mut alto).prefix("alto: "),
                )
                .on_hover_text("Hacé clic y escribí el alto")
                .changed();
        });
    });
    // Los atajos, uno debajo del otro y de ancho completo: los nombres son largos.
    for (nombre, a, h) in PRESETS {
        if boton(ui, nombre).clicked() {
            (ancho, alto) = (a, h);
            cambio = true;
        }
    }

    if cambio {
        let comp = &mut ctx.proyecto.0.composicion;
        comp.ancho = limitar_resolucion(ancho);
        comp.alto = limitar_resolucion(alto);
    }
}

/// Cuánto dura el fundido al cambiar de clip en una capa (0 = corte seco).
fn selector_de_transicion(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let mut segundos = ctx.proyecto.0.composicion.transicion;
    seccion(ui, "Transición");
    let cambio = fila(ui, "Fundido", |ui| {
        deslizador_con(ui, &mut segundos, 0.0..=TRANSICION_MAXIMA, 0.0, |s| {
            s.step_by(0.1).suffix(" s")
        })
        .on_hover_text(
            "Segundos que tarda un clip nuevo en aparecer sobre el anterior en la misma capa. \
             0 es un corte seco. Mientras dura, se reproducen los dos clips.",
        )
        .changed()
    });
    // Solo se escribe en el proyecto si algo cambió.
    if cambio {
        ctx.proyecto.0.composicion.transicion = segundos.clamp(0.0, TRANSICION_MAXIMA);
    }
}

/// Un campo numérico para una dimensión de la resolución.
///
/// `speed(0.0)`: el valor no cambia al arrastrar, solo al hacer clic y escribir. Cada
/// cambio crea una textura nueva, y arrastrar la crearía en cada frame.
///
/// Es una función y no un *closure* porque el `DragValue` que devuelve toma prestado
/// `valor`, y un closure no puede expresar esa relación entre su argumento y su resultado
/// (`'_` en el tipo de retorno sí).
fn campo_de_resolucion(valor: &mut u32) -> egui::DragValue<'_> {
    egui::DragValue::new(valor)
        .range(RESOLUCION_MINIMA..=RESOLUCION_MAXIMA)
        .speed(0.0)
}

/// El panel fijo con las propiedades de la capa elegida: nombre, visibilidad, opacidad,
/// modo de mezcla y ajuste. Está siempre en el mismo lugar, sea cual sea la cantidad de capas.
///
/// Las capas se dibujan de abajo hacia arriba, y la **primera de la lista queda arriba de
/// todas**. Los modos combinan una capa con lo que hay debajo de ella.
fn propiedades_de_capa(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let cantidad = ctx.proyecto.0.composicion.capas.len();
    if cantidad == 0 {
        return;
    }
    // Si la capa elegida ya no existe (se quitó), se elige la última.
    if *ctx.capa_elegida >= cantidad {
        *ctx.capa_elegida = cantidad - 1;
    }
    let i = *ctx.capa_elegida;

    // Una copia de lo que se muestra; los cambios se escriben recién cuando el usuario toca algo.
    let capa = ctx.proyecto.0.composicion.capas[i].clone();

    seccion(ui, &format!("Capa {}", i + 1));

    let mut nombre = capa.nombre.clone();
    let mut visible = capa.visible;
    let mut porcentaje = capa.opacidad * 100.0;
    let mut modulacion = capa.modulacion_de_opacidad.clone();
    let nombres_de_capas: Vec<String> = ctx
        .proyecto
        .0
        .composicion
        .capas
        .iter()
        .map(|c| c.nombre.clone())
        .collect();
    let mut audio = ContextoDeAudio {
        aplicaciones: &mut *ctx.aplicaciones_audio,
        senales: ctx.senales_audio,
        nombres_de_capas: &nombres_de_capas,
    };
    let mut modo = capa.modo;
    let mut ajuste = capa.ajuste;
    let mut cambio_nombre = false;
    let mut cambio = false;

    fila(ui, "Nombre", |ui| {
        cambio_nombre = campo_de_texto(ui, &mut nombre).changed();
    });
    fila(ui, "Visible", |ui| {
        cambio |= ui.checkbox(&mut visible, "").changed();
    });
    // La opacidad se muestra en porcentaje y se guarda de 0 a 1. El botón del sonido
    // despliega la modulación de este parámetro.
    cambio |= fila_de_parametro(
        ui,
        egui::Id::new(("opacidad_de_capa", i)),
        "Opacidad",
        &mut porcentaje,
        &mut modulacion,
        &mut audio,
    );
    fila(ui, "Mezcla", |ui| {
        desplegable(ui, "modo_de_capa", modo.nombre(), |ui| {
            for opcion in ModoMezcla::TODOS {
                cambio |=
                    crate::inspector::opcion(ui, &mut modo, opcion, opcion.nombre()).changed();
            }
        })
        .on_hover_text("Cómo se combina esta capa con lo que hay debajo");
    });
    fila(ui, "Ajuste", |ui| {
        desplegable(ui, "ajuste_de_capa", ajuste.nombre(), |ui| {
            for opcion in Ajuste::TODOS {
                cambio |=
                    crate::inspector::opcion(ui, &mut ajuste, opcion, opcion.nombre()).changed();
            }
        })
        .on_hover_text("Cómo se acomoda el video si no tiene la proporción de la composición");
    });
    ui.add_space(4.0);

    // Solo se escribe en el proyecto si algo cambió.
    if (cambio || cambio_nombre)
        && let Some(capa) = ctx.proyecto.0.composicion.capas.get_mut(i)
    {
        capa.nombre = nombre;
        capa.visible = visible;
        capa.opacidad = (porcentaje / 100.0).clamp(0.0, 1.0);
        capa.modulacion_de_opacidad = modulacion;
        capa.modo = modo;
        capa.ajuste = ajuste;
    }
}

/// Agranda un rectángulo (que puede no existir todavía) para que incluya a `otro`.
fn unir(actual: Option<egui::Rect>, otro: egui::Rect) -> Option<egui::Rect> {
    Some(actual.map_or(otro, |a| a.union(otro)))
}

/// El botón de un clip: su miniatura (con el nombre debajo) cuando ya está lista; mientras
/// tanto, o si no se pudo sacar, un texto corto. Siempre del mismo tamaño.
fn boton_de_clip(
    ui: &mut egui::Ui,
    texturas: &mut EguiUserTextures,
    miniaturas: &Miniaturas,
    archivo: &std::path::Path,
    nombre: &str,
    sonando: bool,
) -> egui::Response {
    match miniaturas.de(archivo) {
        Some(EstadoMiniatura::Lista {
            imagen,
            ancho,
            alto,
        }) => {
            let textura = texturas.add_image(EguiTextureHandle::Weak(imagen.id()));
            dibujar_celda(
                ui,
                ContenidoDeCelda::Miniatura {
                    textura,
                    ancho: *ancho,
                    alto: *alto,
                    nombre,
                },
                sonando,
            )
        }
        Some(EstadoMiniatura::Fallo(motivo)) => dibujar_celda(
            ui,
            ContenidoDeCelda::Texto {
                texto: "sin miniatura",
                nombre,
            },
            sonando,
        )
        .on_hover_text(format!("No se pudo sacar la miniatura: {motivo}")),
        // Todavía se está sacando (o aún no se pidió).
        Some(EstadoMiniatura::Pendiente) | None => dibujar_celda(
            ui,
            ContenidoDeCelda::Texto {
                texto: "…", nombre
            },
            sonando,
        ),
    }
}

/// La grilla de capas × columnas.
fn grilla(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    // Una copia de la composición para recorrerla mientras se junta lo que el usuario pide
    // (son unos pocos textos: copiarla en cada frame no cuesta nada).
    let comp = ctx.proyecto.0.composicion.clone();
    let carpeta = ctx.ruta.carpeta();
    let mut acciones: Vec<Accion> = Vec::new();
    // Si terminó la búsqueda de cámaras, se recoge el resultado.
    ctx.dispositivos.recibir();

    // Lo que egui recuerda del frame anterior: el rectángulo de cada fila de capa (para el
    // área sensible de fondo) y qué capa se está arrastrando.
    let id_filas = egui::Id::new(MEMORIA_FILAS);
    let id_arrastrada = egui::Id::new(MEMORIA_ARRASTRADA);
    let filas_previas: Vec<egui::Rect> = ui.data(|d| d.get_temp(id_filas)).unwrap_or_default();
    let mut arrastrada: Option<usize> = ui.data(|d| d.get_temp::<usize>(id_arrastrada));
    let mut filas_actuales: Vec<egui::Rect> = Vec::new();
    // Lo mismo para el clip que se está arrastrando, y el lugar de cada celda en este frame.
    let id_clip = egui::Id::new(MEMORIA_CLIP_ARRASTRADO);
    let mut clip_arrastrado: Option<(usize, usize)> =
        ui.data(|d| d.get_temp::<(usize, usize)>(id_clip));
    let mut celdas_actuales: Vec<((usize, usize), egui::Rect)> = Vec::new();
    let derecha_de_la_grilla = ui.max_rect().right();

    egui::Grid::new("grilla_clips")
        // Espacio entre capas (y entre clips): las filas respiran y el resaltado de la capa
        // elegida no se pega a la de al lado.
        .spacing([10.0, 18.0])
        .show(ui, |ui| {
            // Encabezado: «+ Capa» arriba a la izquierda (así no se mueve cada vez que se agrega
            // una) y un botón por columna que dispara la columna entera.
            if ui
                .add_sized(
                    [ANCHO_NOMBRE_DE_CAPA, 24.0],
                    egui::Button::new(con_icono(ui, egui_phosphor::regular::PLUS, verde(), "Capa")),
                )
                .on_hover_text("Agregar una capa")
                .clicked()
            {
                acciones.push(Accion::AgregarCapa);
            }
            for (c, columna) in comp.columnas.iter().enumerate() {
                let boton = ui
                    .add_sized(
                        [TAM_CELDA[0], 24.0],
                        egui::Button::new(format!("Columna {}", columna.nombre)),
                    )
                    .on_hover_text("Disparar el clip de esta columna en cada capa");
                if boton.clicked() {
                    ctx.ordenes.push(OrdenComposicion::DispararColumna(c));
                }
                boton.context_menu(|ui| {
                    if ui.button("Quitar columna").clicked() {
                        acciones.push(Accion::QuitarColumna(c));
                        ui.close();
                    }
                });
            }
            if ui
                .button(con_icono(
                    ui,
                    egui_phosphor::regular::PLUS,
                    verde(),
                    "Columna",
                ))
                .clicked()
            {
                acciones.push(Accion::AgregarColumna);
            }
            ui.end_row();

            // Una fila por capa.
            for (capa_i, capa) in comp.capas.iter().enumerate() {
                // El fondo de la fila: un área sensible que la cubre entera. Un clic sobre
                // espacio vacío elige la capa y un arrastre la reordena. Se crea ANTES que
                // los botones, así que los botones tienen prioridad y el fondo solo recibe
                // lo que cae en espacio vacío. Usa el rectángulo del frame anterior,
                // porque el tamaño de la fila recién se conoce al terminar de dibujarla.
                // El alto sale del cuadro anterior (el tamaño de la fila recién se conoce al terminar de
                // dibujarla); el ancho llega siempre hasta el borde derecho de AHORA, así al
                // redimensionar la ventana la franja no va un cuadro atrasada.
                let rect_previo = filas_previas.get(capa_i).copied().map(|mut r| {
                    r.max.x = derecha_de_la_grilla.max(r.min.x + 1.0);
                    r
                });
                let fondo = rect_previo.map(|r| {
                    ui.interact(
                        r,
                        egui::Id::new(("fondo_de_fila_de_capa", capa_i)),
                        egui::Sense::click_and_drag(),
                    )
                });
                let elegida = *ctx.capa_elegida == capa_i;
                if let Some(r) = rect_previo {
                    let color = if elegida {
                        Some(crate::tema::acento().gamma_multiply(0.20))
                    } else if fondo.as_ref().is_some_and(|f| f.hovered()) {
                        Some(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14))
                    } else {
                        None
                    };
                    if let Some(color) = color {
                        ui.painter().rect_filled(r.expand(3.0), 6.0, color);
                    }
                }
                if let Some(f) = &fondo {
                    if f.clicked() {
                        *ctx.capa_elegida = capa_i;
                    }
                    if f.drag_started() {
                        arrastrada = Some(capa_i);
                        *ctx.capa_elegida = capa_i;
                    }
                    if f.hovered() || f.dragged() {
                        ui.ctx().set_cursor_icon(if f.dragged() {
                            egui::CursorIcon::Grabbing
                        } else {
                            egui::CursorIcon::Grab
                        });
                    }
                }

                // Lo que ocupa esta fila, para recordarlo hasta el próximo frame.
                let mut rect_fila: Option<egui::Rect> = None;

                let celda_de_nombre = ui.horizontal(|ui| {
                    // Un margen a la izquierda: el nombre y los botones no quedan pegados al borde
                    // del área de la capa.
                    ui.add_space(PADDING_DE_CAPA);
                    ui.vertical(|ui| {
                        // Ancho mínimo: si no, los nombres se parten en dos líneas.
                        ui.set_min_width(ANCHO_NOMBRE_DE_CAPA - PADDING_DE_CAPA);
                        ui.add_space(2.0);
                        // El nombre es un texto, no un botón: así un clic o un arrastre sobre él
                        // llegan al fondo de la fila (elegir y reordenar).
                        ui.label(egui::RichText::new(&capa.nombre).strong());
                        ui.add_space(2.0);
                        // Parar y borrar: dos botones cuadrados, solo con el ícono y del mismo tamaño.
                        ui.horizontal(|ui| {
                            if boton_cuadrado(ui, icono(egui_phosphor::regular::STOP, rojo()))
                                .on_hover_text("Parar esta capa")
                                .clicked()
                            {
                                ctx.ordenes.push(OrdenComposicion::Detener { capa: capa_i });
                            }
                            // Borrar pide una confirmación (un segundo clic): no se puede
                            // deshacer. Siempre queda una capa.
                            if comp.capas.len() > 1 {
                                let borrar = boton_cuadrado(
                                    ui,
                                    icono(egui_phosphor::regular::TRASH, rojo()),
                                )
                                .on_hover_text("Borrar la capa");
                                egui::Popup::menu(&borrar).show(|ui| {
                                    ui.label(format!("¿Borrar «{}»?", capa.nombre));
                                    if ui.button("Sí, borrar").clicked() {
                                        acciones.push(Accion::QuitarCapa(capa_i));
                                        ui.close();
                                    }
                                });
                            }
                        });
                    });
                });
                rect_fila = unir(rect_fila, celda_de_nombre.response.rect);

                for (col, celda) in capa.celdas.iter().enumerate() {
                    let sonando =
                        ctx.reproduccion.activo.get(capa_i).copied().flatten() == Some(col);
                    match celda {
                        Some(clip) => {
                            // Un archivo tiene miniatura; una cámara o una transmisión, un botón
                            // «en vivo».
                            let archivo = clip
                                .fuente
                                .ruta_de_archivo()
                                .map(|r| resolver_ruta(&carpeta, r));
                            let respuesta = match &archivo {
                                Some(archivo) => boton_de_clip(
                                    ui,
                                    ctx.texturas,
                                    ctx.miniaturas,
                                    archivo,
                                    &clip.nombre,
                                    sonando,
                                ),
                                None => boton_en_vivo(ui, clip, sonando),
                            }
                            .on_hover_text(clip.fuente.descripcion())
                            // Además de un clic (disparar), la celda siente el arrastre:
                            // sirve para moverla a otra columna o a otra capa. Como se crea
                            // después del fondo de la fila, tiene prioridad sobre él.
                            .interact(egui::Sense::click_and_drag());
                            rect_fila = unir(rect_fila, respuesta.rect);
                            celdas_actuales.push(((capa_i, col), respuesta.rect));
                            if archivo
                                .as_ref()
                                .is_some_and(|a| !existe_cacheado(ui.ctx(), a))
                            {
                                // El archivo no está: una advertencia sobre la celda.
                                ui.painter().text(
                                    respuesta.rect.right_top() + egui::vec2(-6.0, 4.0),
                                    egui::Align2::RIGHT_TOP,
                                    egui_phosphor::regular::WARNING,
                                    egui::FontId::proportional(18.0),
                                    rojo(),
                                );
                            }
                            if respuesta.drag_started() {
                                clip_arrastrado = Some((capa_i, col));
                            }
                            if respuesta.hovered() || respuesta.dragged() {
                                ui.ctx().set_cursor_icon(if respuesta.dragged() {
                                    egui::CursorIcon::Grabbing
                                } else {
                                    egui::CursorIcon::Grab
                                });
                            }
                            // El borde del clip elegido aparece con un fundido.
                            let elegido = ui.ctx().animate_bool_with_time(
                                respuesta.id.with("elegido"),
                                *ctx.clip_elegido == Some((capa_i, col)),
                                0.15,
                            );
                            if elegido > 0.01 {
                                ui.painter().rect_stroke(
                                    respuesta.rect.expand(3.0),
                                    4.0,
                                    egui::Stroke::new(
                                        2.5,
                                        crate::tema::acento().gamma_multiply(elegido),
                                    ),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if respuesta.clicked() || respuesta.drag_started() {
                                // Elegir el clip (y su capa) no depende de dispararlo.
                                *ctx.clip_elegido = Some((capa_i, col));
                                *ctx.capa_elegida = capa_i;
                            }
                            if respuesta.clicked() {
                                ctx.ordenes.push(OrdenComposicion::Disparar {
                                    capa: capa_i,
                                    columna: col,
                                });
                            }
                            // Una captura que falló: se marca en la celda, con el motivo.
                            if let Some((_, motivo)) = ctx
                                .errores
                                .0
                                .get(capa_i)
                                .and_then(|e| e.as_ref())
                                .filter(|(columna, _)| *columna == col)
                            {
                                let icono_rect = egui::Rect::from_center_size(
                                    respuesta.rect.right_top() + egui::vec2(-14.0, 14.0),
                                    egui::vec2(22.0, 22.0),
                                );
                                ui.painter().text(
                                    icono_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    egui_phosphor::regular::WARNING,
                                    egui::FontId::proportional(18.0),
                                    rojo(),
                                );
                                ui.interact(
                                    icono_rect,
                                    respuesta.id.with("fallo"),
                                    egui::Sense::hover(),
                                )
                                .on_hover_text(format!("No se pudo reproducir: {motivo}"));
                            }
                            respuesta.context_menu(|ui| {
                                if matches!(clip.fuente, Fuente::Captura { .. })
                                    && ui.button("Elegir otra ventana o pantalla…").clicked()
                                {
                                    acciones.push(Accion::ReelegirCaptura {
                                        capa: capa_i,
                                        columna: col,
                                    });
                                    ui.close();
                                }
                                if ui.button("Cambiar archivo…").clicked() {
                                    acciones.push(Accion::ElegirArchivo {
                                        capa: capa_i,
                                        columna: col,
                                    });
                                    ui.close();
                                }
                                if ui.button("Quitar clip").clicked() {
                                    acciones.push(Accion::QuitarClip {
                                        capa: capa_i,
                                        columna: col,
                                    });
                                    ui.close();
                                }
                            });
                        }
                        None => {
                            let vacia = dibujar_celda(ui, ContenidoDeCelda::Vacia, false)
                                .on_hover_text("Agregar un video, una cámara o una transmisión");
                            rect_fila = unir(rect_fila, vacia.rect);
                            celdas_actuales.push(((capa_i, col), vacia.rect));
                            if vacia.clicked() {
                                // Al abrir el menú se vuelven a buscar las cámaras.
                                ctx.dispositivos.actualizar();
                            }
                            egui::Popup::menu(&vacia).show(|ui| {
                                menu_de_fuentes(ui, ctx.dispositivos, capa_i, col, &mut acciones);
                            });
                        }
                    }
                }
                ui.end_row();

                // El área de la fila llega hasta el borde derecho: el espacio vacío a la
                // derecha de las celdas también es "de esa capa".
                if let Some(mut r) = rect_fila {
                    r.max.x = r.max.x.max(derecha_de_la_grilla);
                    filas_actuales.push(r);
                }
            }
        });

    // --- Arrastrar un clip a otra celda ---
    if let Some(origen) = clip_arrastrado {
        let puntero = ui.input(|i| i.pointer.interact_pos());
        let destino = puntero.and_then(|p| {
            celdas_actuales
                .iter()
                .find(|(_, r)| r.contains(p))
                .map(|(celda, _)| *celda)
        });
        let rect_de = |celda: (usize, usize)| {
            celdas_actuales
                .iter()
                .find(|(c, _)| *c == celda)
                .map(|(_, r)| *r)
        };
        let pintor = ui.painter();
        if let Some(r) = rect_de(origen) {
            pintor.rect_stroke(
                r.expand(2.0),
                4.0,
                egui::Stroke::new(2.0, verde()),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(d) = destino
            && d != origen
            && let Some(r) = rect_de(d)
        {
            pintor.rect_stroke(
                r.expand(2.0),
                4.0,
                egui::Stroke::new(3.0, verde()),
                egui::StrokeKind::Outside,
            );
        }
        // El nombre del clip acompaña al puntero.
        if let Some(p) = puntero
            && let Some(clip) = comp.clip(origen.0, origen.1)
        {
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("clip_en_vuelo"),
                ))
                .text(
                    p + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    &clip.nombre,
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(d) = destino
                && d != origen
            {
                acciones.push(Accion::MoverClip {
                    desde: origen,
                    hasta: d,
                });
            }
            clip_arrastrado = None;
        }
    }

    // --- El diálogo para escribir la dirección de una transmisión de red ---
    if let Some((capa, columna)) = ctx.dispositivos.url_para {
        let mut cerrar = false;
        egui::Window::new("Transmisión de red")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label("Dirección de la transmisión (rtsp://, srt://, http://…):");
                ui.add(
                    egui::TextEdit::singleline(&mut ctx.dispositivos.texto_url)
                        .desired_width(340.0)
                        .hint_text("rtsp://192.168.1.5:8554/camara"),
                );
                let valida = explorador::es_url_de_red(&ctx.dispositivos.texto_url);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(valida, egui::Button::new("Agregar"))
                        .on_disabled_hover_text("Tiene que empezar con rtsp://, srt://, http://…")
                        .clicked()
                    {
                        acciones.push(Accion::PonerFuente {
                            capa,
                            columna,
                            clip: ConfigClip::de_url(ctx.dispositivos.texto_url.trim().to_string()),
                        });
                        cerrar = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cerrar = true;
                    }
                });
            });
        if cerrar {
            ctx.dispositivos.url_para = None;
        }
    }

    // --- Soltar un video del explorador sobre una celda ---
    let id_archivo = egui::Id::new(MEMORIA_ARCHIVO_ARRASTRADO);
    if let Some(archivo) = ui.data(|d| d.get_temp::<PathBuf>(id_archivo)) {
        let puntero = ui.input(|i| i.pointer.interact_pos());
        let destino = puntero.and_then(|p| {
            celdas_actuales
                .iter()
                .find(|(_, r)| r.contains(p))
                .map(|(celda, _)| *celda)
        });
        if let Some(d) = destino
            && let Some((_, r)) = celdas_actuales.iter().find(|(c, _)| *c == d)
        {
            ui.painter().rect_stroke(
                r.expand(2.0),
                4.0,
                egui::Stroke::new(3.0, verde()),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(p) = puntero {
            let nombre = archivo
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("archivo_en_vuelo"),
                ))
                .text(
                    p + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    nombre,
                    egui::FontId::proportional(14.0),
                    crate::tema::tema().texto,
                );
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if ui.input(|i| i.pointer.any_released()) {
            if let Some((capa, columna)) = destino {
                acciones.push(Accion::PonerClip {
                    capa,
                    columna,
                    archivo,
                });
            }
            ui.data_mut(|d| d.remove_temp::<PathBuf>(id_archivo));
        }
    }

    // --- Arrastrar una fila para reordenar capas ---
    if let Some(desde) = arrastrada {
        let puntero = ui.input(|i| i.pointer.interact_pos());
        if let Some(p) = puntero
            && desde < filas_actuales.len()
        {
            // Dónde caería: entre qué filas está el puntero (0 = arriba de todas).
            let posicion = filas_actuales.iter().filter(|r| r.center().y < p.y).count();
            let (izquierda, derecha) = filas_actuales
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), r| {
                    (a.min(r.left()), b.max(r.right()))
                });
            let y = if posicion == 0 {
                filas_actuales[0].top() - 4.0
            } else {
                filas_actuales[posicion - 1].bottom() + 4.0
            };
            let pintor = ui.painter();
            // La fila que se arrastra queda resaltada, y una línea marca dónde va a caer.
            pintor.rect_stroke(
                filas_actuales[desde].expand(3.0),
                6.0,
                egui::Stroke::new(2.0, verde()),
                egui::StrokeKind::Outside,
            );
            pintor.line_segment(
                [egui::pos2(izquierda, y), egui::pos2(derecha, y)],
                egui::Stroke::new(3.0, verde()),
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

            if ui.input(|i| i.pointer.any_released()) {
                // Al sacar la fila de su lugar, las de abajo suben: si cae más abajo de
                // donde estaba, la posición final es una menos.
                let hasta = if posicion > desde {
                    posicion - 1
                } else {
                    posicion
                };
                if hasta != desde {
                    acciones.push(Accion::MoverCapa { desde, hasta });
                }
                arrastrada = None;
            }
        } else if ui.input(|i| i.pointer.any_released()) {
            arrastrada = None;
        }
    }
    // Se recuerda hasta el próximo frame.
    ui.data_mut(|d| {
        match clip_arrastrado {
            Some(c) => {
                d.insert_temp(id_clip, c);
            }
            None => {
                d.remove_temp::<(usize, usize)>(id_clip);
            }
        }
        d.insert_temp(id_filas, filas_actuales);
        match arrastrada {
            Some(i) => {
                d.insert_temp(id_arrastrada, i);
            }
            None => {
                d.remove_temp::<usize>(id_arrastrada);
            }
        }
    });

    // Recién ahora, con la grilla ya dibujada, se aplican los cambios pedidos.
    for accion in acciones {
        match accion {
            Accion::ElegirArchivo { capa, columna } => {
                ctx.dialogo.abrir(capa, columna, ctx.ruta.carpeta());
            }
            Accion::QuitarClip { capa, columna } => {
                ctx.proyecto.0.composicion.quitar_clip(capa, columna);
            }
            Accion::ReelegirCaptura { capa, columna } => {
                *ctx.clip_elegido = Some((capa, columna));
                *ctx.capa_elegida = capa;
                ctx.ordenes
                    .push(OrdenComposicion::ReelegirCaptura { capa, columna });
            }
            Accion::AgregarColumna => ctx.proyecto.0.composicion.agregar_columna(),
            Accion::AgregarCapa => {
                ctx.proyecto.0.composicion.agregar_capa();
                // La capa nueva queda elegida, para poder configurarla enseguida.
                *ctx.capa_elegida = ctx.proyecto.0.composicion.capas.len() - 1;
            }
            Accion::MoverCapa { desde, hasta } => {
                // `mover_capa` también corrige qué capa toma cada superficie.
                if ctx.proyecto.0.mover_capa(desde, hasta) {
                    ctx.reproduccion.capa_movida(desde, hasta);
                    ctx.capas_movidas.push((desde, hasta));
                    // La capa elegida sigue siendo la misma aunque cambie de lugar.
                    *ctx.capa_elegida = nueva_posicion(*ctx.capa_elegida, desde, hasta);
                }
            }
            Accion::PonerFuente {
                capa,
                columna,
                clip,
            } => {
                // Una captura nueva se dispara enseguida: así el escritorio muestra su
                // selector de pantalla o ventana.
                let es_captura = matches!(clip.fuente, Fuente::Captura { .. });
                ctx.proyecto.0.composicion.poner_clip(capa, columna, clip);
                if es_captura
                    || ctx.reproduccion.activo.get(capa).copied().flatten() == Some(columna)
                {
                    ctx.ordenes
                        .push(OrdenComposicion::Disparar { capa, columna });
                }
                *ctx.clip_elegido = Some((capa, columna));
            }
            Accion::PonerClip {
                capa,
                columna,
                archivo,
            } => {
                let guardada = ruta_para_guardar(&ctx.ruta.carpeta(), &archivo);
                ctx.proyecto.0.composicion.poner_clip(
                    capa,
                    columna,
                    ConfigClip::de_archivo(guardada),
                );
                // Si se reemplazó el clip que sonaba, se vuelve a disparar el nuevo.
                if ctx.reproduccion.activo.get(capa).copied().flatten() == Some(columna) {
                    ctx.ordenes
                        .push(OrdenComposicion::Disparar { capa, columna });
                }
                *ctx.clip_elegido = Some((capa, columna));
            }
            Accion::MoverClip { desde, hasta } => {
                if ctx.proyecto.0.composicion.mover_clip(desde, hasta) {
                    // Si el clip que suena cambió de capa, esa capa se detiene; dentro de la
                    // misma capa el que suena sigue sonando.
                    for capa in ctx.reproduccion.clip_movido(desde, hasta) {
                        ctx.ordenes.push(OrdenComposicion::Detener { capa });
                    }
                }
            }
            Accion::QuitarCapa(indice) => {
                // `quitar_capa` también corrige las superficies que tomaban capas.
                if ctx.proyecto.0.quitar_capa(indice) {
                    ctx.reproduccion.capa_quitada(indice);
                    ctx.capas_quitadas.push(indice);
                    // La elegida sigue siendo la misma capa aunque cambie su posición.
                    if *ctx.capa_elegida > indice {
                        *ctx.capa_elegida -= 1;
                    }
                }
            }
            Accion::QuitarColumna(columna) => {
                ctx.proyecto.0.composicion.quitar_columna(columna);
                // Las columnas siguientes corren un lugar: el estado de reproducción
                // tiene que enterarse para seguir apuntando al mismo clip.
                ctx.reproduccion.columna_quitada(columna);
            }
        }
    }
}

/// Alto que se reserva para el título de cada previsualización: ninguno, el título va ADENTRO de
/// la imagen, en una etiqueta arriba a la izquierda.
const ALTO_DE_TITULO_DE_PREVIA: f32 = 0.0;
/// Lo mínimo que se le deja a la previsualización del clip para que se vea algo.
const ALTO_MINIMO_DE_LA_PREVIA_DEL_CLIP: f32 = 36.0;

/// Una etiqueta con el título ADENTRO de la imagen, arriba a la izquierda, sobre un fondo oscuro
/// translúcido (se lee sobre cualquier imagen). El texto se recorta con «…» a lo que mide la
/// imagen: nunca se sale de ella.
fn etiqueta_sobre_la_imagen(ui: &egui::Ui, imagen: egui::Rect, titulo: &str) {
    const MARGEN: f32 = 6.0;
    const RELLENO: egui::Vec2 = egui::vec2(6.0, 3.0);
    let ancho_maximo = (imagen.width() - 2.0 * MARGEN - 2.0 * RELLENO.x).max(8.0);
    let mut trabajo = egui::text::LayoutJob::simple_singleline(
        titulo.to_string(),
        egui::FontId::proportional(12.5),
        egui::Color32::WHITE,
    );
    trabajo.wrap = egui::text::TextWrapping {
        max_width: ancho_maximo,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let pintor = ui.painter_at(imagen);
    let galley = pintor.layout_job(trabajo);
    let etiqueta = egui::Rect::from_min_size(
        imagen.min + egui::vec2(MARGEN, MARGEN),
        galley.size() + 2.0 * RELLENO,
    );
    pintor.rect_filled(etiqueta, 4.0, egui::Color32::from_black_alpha(165));
    pintor.galley(etiqueta.min + RELLENO, galley, egui::Color32::WHITE);
}

/// Las previsualizaciones, como en Resolume: la de la COMPOSICIÓN (lo que sale mezclado) ocupa
/// todo el ancho de su columna y, SIEMPRE debajo, la del CLIP elegido, que se achica para darle
/// el lugar necesario a la de arriba.
fn previsualizaciones(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let ancho = ui.available_width();
    let alto = ui.available_height();
    let separacion = ui.spacing().item_spacing.y;

    // La textura del clip elegido: la de su capa si está sonando; si no, su miniatura.
    let clip = ctx.clip_elegido.and_then(|(capa, columna)| {
        let comp = &ctx.proyecto.0.composicion;
        let config = comp.clip(capa, columna)?;
        let sonando = ctx.reproduccion.activo.get(capa).copied().flatten() == Some(columna);
        let en_vivo = if sonando {
            ctx.capas_texturas
                .0
                .get(capa)
                .map(|t| (t.imagen.id(), t.ancho, t.alto))
        } else {
            None
        };
        let miniatura = || {
            let archivo = config
                .fuente
                .ruta_de_archivo()
                .map(|r| resolver_ruta(&ctx.ruta.carpeta(), r))?;
            match ctx.miniaturas.de(&archivo) {
                Some(EstadoMiniatura::Lista {
                    imagen,
                    ancho,
                    alto,
                }) => Some((imagen.id(), *ancho, *alto)),
                _ => None,
            }
        };
        Some((config.nombre.clone(), sonando, en_vivo.or_else(miniatura)))
    });
    let composicion = ctx.textura.map(|t| {
        (
            t.imagen.id(),
            t.ancho,
            t.alto,
            format!("{}×{}", t.ancho, t.alto),
        )
    });

    let aspecto_de_la_composicion = composicion
        .as_ref()
        .map_or(16.0 / 9.0, |(_, a, h, _)| *a as f32 / (*h).max(1) as f32);
    let imagen_del_clip = clip.as_ref().and_then(|(_, _, i)| *i);
    let aspecto_del_clip =
        imagen_del_clip.map_or(16.0 / 9.0, |(_, a, h)| a as f32 / h.max(1) as f32);
    let (caja_de_la_composicion, caja_del_clip) = reparto::medidas_de_previas(
        (ancho, alto),
        aspecto_de_la_composicion,
        aspecto_del_clip,
        ALTO_DE_TITULO_DE_PREVIA,
        separacion,
        ALTO_MINIMO_DE_LA_PREVIA_DEL_CLIP,
    );

    let mut dibujar = |ui: &mut egui::Ui,
                       titulo: String,
                       imagen: Option<(bevy::asset::AssetId<Image>, u32, u32)>,
                       vacio: &str,
                       caja: (f32, f32)| {
        let caja = egui::vec2(caja.0, caja.1);
        ui.vertical(|ui| {
            ui.set_width(caja.x);
            let (rect, _) = ui.allocate_exact_size(caja, egui::Sense::hover());
            // La caja es negra, como un monitor apagado.
            ui.painter().rect_filled(rect, 3.0, egui::Color32::BLACK);
            match imagen {
                Some((id, _, _)) => {
                    let textura = ctx.texturas.add_image(EguiTextureHandle::Weak(id));
                    ui.painter().image(
                        textura,
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    // El texto se recorta a la caja: en una caja chica no se sale de ella.
                    ui.painter_at(rect).text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        vacio,
                        egui::FontId::proportional(13.0),
                        crate::tema::tema().texto,
                    );
                }
            }
            etiqueta_sobre_la_imagen(ui, rect, &titulo);
        });
    };

    let (titulo_comp, imagen_comp) = match composicion {
        Some((id, a, h, medida)) => (format!("Composición ({medida})"), Some((id, a, h))),
        None => ("Composición".to_string(), None),
    };
    let (titulo_clip, imagen_clip, vacio_clip) = match clip {
        Some((nombre, sonando, imagen)) => (
            format!("Clip: {nombre}"),
            imagen,
            if sonando {
                "Sin imagen todavía"
            } else {
                "Disparalo para verlo en vivo"
            },
        ),
        None => ("Clip".to_string(), None, "Elegí un clip de la grilla"),
    };

    dibujar(
        ui,
        titulo_comp,
        imagen_comp,
        "Sin composición",
        caja_de_la_composicion,
    );
    dibujar(ui, titulo_clip, imagen_clip, vacio_clip, caja_del_clip);
}

#[cfg(test)]
mod pruebas_de_celdas {
    use super::*;

    /// Lo que se vio en pantalla: las celdas con miniatura eran de otro tamaño que las vacías y
    /// las filas quedaban desparejas. Todas tienen que medir exactamente lo mismo, sea cual sea
    /// su contenido o lo largo que sea el nombre.
    #[test]
    fn todas_las_celdas_miden_lo_mismo_tengan_lo_que_tengan() {
        let ctx = egui::Context::default();
        let mut medidas: Vec<egui::Vec2> = Vec::new();
        for _ in 0..2 {
            // Dos cuadros: el primero deja listas las fuentes de texto.
            medidas.clear();
            let mut salida = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let textura = egui::TextureId::default();
                    let nombre_largo = "Un nombre larguísimo de clip que no entra en la celda.mp4";
                    let celdas = [
                        ContenidoDeCelda::Vacia,
                        ContenidoDeCelda::Miniatura {
                            textura,
                            ancho: 1920,
                            alto: 1080,
                            nombre: nombre_largo,
                        },
                        // Una miniatura vertical, que antes agrandaba la fila.
                        ContenidoDeCelda::Miniatura {
                            textura,
                            ancho: 1080,
                            alto: 1920,
                            nombre: "vertical",
                        },
                        ContenidoDeCelda::EnVivo {
                            glifo: "x",
                            nombre: "Captura",
                        },
                        ContenidoDeCelda::Texto {
                            texto: "…",
                            nombre: "pendiente",
                        },
                    ];
                    ui.horizontal(|ui| {
                        for (i, c) in celdas.into_iter().enumerate() {
                            // Una de las celdas suena: el borde de «sonando» no cambia el tamaño.
                            let r = dibujar_celda(ui, c, i == 3);
                            medidas.push(r.rect.size());
                        }
                    });
                },
            );
            // Sin GPU no hay a dónde subir las texturas que egui pide: se descartan a propósito.
            salida.textures_delta.clear();
        }
        assert_eq!(medidas.len(), 5);
        let esperado = egui::vec2(TAM_CELDA[0], TAM_CELDA[1] + ALTO_NOMBRE);
        for m in &medidas {
            assert_eq!(*m, esperado, "{medidas:?}");
        }
    }
}
