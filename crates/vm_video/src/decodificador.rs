//! Decodificación de video con GStreamer. Rust puro: no depende de Bevy.
//!
//! GStreamer arma una "tubería" (pipeline) de elementos: leer archivo →
//! decodificar → convertir a RGBA → `appsink`. El `appsink` es el final de la
//! tubería: en vez de mostrar el frame, nos lo entrega a nosotros.

use std::error::Error;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::audio::AudioDeCaptura;

/// Un cuadro de video ya decodificado, en RGBA (4 bytes por píxel).
pub struct Frame {
    pub ancho: u32,
    pub alto: u32,
    pub datos: Vec<u8>,
}

/// Cómo se reproduce un video: bucle, velocidad, tramo y audio. Es una copia de los ajustes
/// del clip (`vm_core::AjustesClip`) para que este módulo no dependa del modelo del proyecto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ajustes {
    pub bucle: bool,
    pub velocidad: f32,
    /// Inicio y fin del tramo, como fracción (0 a 1) de la duración.
    pub inicio: f32,
    pub fin: f32,
    pub volumen: f32,
    pub silenciado: bool,
}

impl Default for Ajustes {
    fn default() -> Self {
        Ajustes {
            bucle: true,
            velocidad: 1.0,
            inicio: 0.0,
            fin: 1.0,
            volumen: 1.0,
            silenciado: false,
        }
    }
}

impl Ajustes {
    /// ¿Hay que hacer un salto especial (distinta velocidad o un tramo) al empezar?
    fn pide_salto(&self) -> bool {
        self.velocidad != 1.0 || self.inicio > 0.0 || self.fin < 1.0
    }
}

/// Los instantes de un tramo, en nanosegundos, para un video de `duracion_ns`.
/// Devuelve `(inicio, fin)`; `fin` es `None` si el tramo llega hasta el final del video.
pub fn limites_del_tramo(a: &Ajustes, duracion_ns: u64) -> (u64, Option<u64>) {
    let en = |fraccion: f32| (duracion_ns as f64 * fraccion.clamp(0.0, 1.0) as f64) as u64;
    let fin = if a.fin >= 0.999 {
        None
    } else {
        Some(en(a.fin))
    };
    (en(a.inicio), fin)
}

/// De dónde sale la imagen de un video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origen {
    /// Un archivo.
    Archivo(std::path::PathBuf),
    /// Una cámara o dispositivo de captura V4L2 (`/dev/video0`).
    Camara(String),
    /// Una transmisión de red (`rtsp://`, `srt://`, `http://`...).
    Url(String),
    /// Un nodo de PipeWire (la captura de una pantalla o ventana, ya autorizada por el
    /// portal): el descriptor de archivo de la conexión y el número del nodo. Se usa con
    /// `Video::abrir_captura`, que además mantiene viva la sesión.
    PipeWire {
        fd: i32,
        nodo: u32,
        /// De dónde sale el sonido (el portal solo entrega imagen); `None` = sin sonido.
        audio: Option<AudioDeCaptura>,
    },
}

impl Origen {
    /// ¿Es una imagen en vivo (cámara, captura de pantalla, transmisión en directo)? Una
    /// imagen en vivo se muestra **apenas llega**: no tiene un ritmo propio que respetar, y
    /// esperar a la hora de cada cuadro congelaría la imagen si las marcas de tiempo de la
    /// fuente no coinciden con el reloj de la tubería (como pasa con las capturas de pantalla).
    pub fn es_en_vivo(&self) -> bool {
        match self {
            Origen::Archivo(_) => false,
            Origen::Camara(_) | Origen::PipeWire { .. } => true,
            Origen::Url(url) => {
                let protocolo = url.split("://").next().unwrap_or("").to_ascii_lowercase();
                ["rtsp", "rtsps", "srt", "udp", "rtmp"].contains(&protocolo.as_str())
            }
        }
    }
}

/// Una cámara o dispositivo de captura que el sistema reconoce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispositivoDeVideo {
    /// El nombre que le da el sistema («Integrated Camera»).
    pub nombre: String,
    /// El archivo del dispositivo (`/dev/video0`).
    pub dispositivo: String,
}

/// Lista las cámaras y dispositivos de captura de video (V4L2) conectados. Puede tardar un
/// instante: llamarla desde un hilo aparte.
pub fn listar_dispositivos() -> Vec<DispositivoDeVideo> {
    if gst::init().is_err() {
        return Vec::new();
    }
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    if monitor.start().is_err() {
        return Vec::new();
    }
    let mut dispositivos: Vec<DispositivoDeVideo> = monitor
        .devices()
        .iter()
        .filter_map(|d| {
            let propiedades = d.properties()?;
            // Solo los V4L2: tienen la ruta del archivo del dispositivo.
            let ruta = propiedades
                .get::<String>("api.v4l2.path")
                .or_else(|_| propiedades.get::<String>("device.path"))
                .ok()?;
            ruta.starts_with("/dev/video").then(|| DispositivoDeVideo {
                nombre: d.display_name().to_string(),
                dispositivo: ruta,
            })
        })
        .collect();
    monitor.stop();
    dispositivos.sort_by(|a, b| a.dispositivo.cmp(&b.dispositivo));
    dispositivos.dedup_by(|a, b| a.dispositivo == b.dispositivo);
    dispositivos
}

/// Un video en reproducción. Mientras exista este valor, el video sigue corriendo.
pub struct Video {
    pipeline: gst::Pipeline,
    /// Canal por donde llegan los frames. El `Mutex` hace que `Video` se pueda
    /// compartir entre hilos (un `Receiver` solo no lo permite), y Bevy lo exige.
    frames: Mutex<Receiver<Frame>>,
    /// Bandera para avisarle al hilo del loop que termine.
    terminar: Arc<AtomicBool>,
    hilo_bus: Option<JoinHandle<()>>,
    /// El último error que informó GStreamer **después** de arrancar (archivo dañado,
    /// formato que no se puede decodificar...). Lo escribe el hilo del bus.
    error: Arc<Mutex<Option<String>>>,
    /// Los ajustes vigentes; los lee el hilo del bus (para el bucle) y los cambia `ajustar`.
    ajustes: Arc<Mutex<Ajustes>>,
    /// El control de volumen de la rama de audio, si el video tiene audio.
    volumen: Arc<Mutex<Option<gst::Element>>>,
    /// Las muestras de audio (mono, 48 kHz) que salen de la derivación para analizar el sonido
    /// del clip (ver `tomar_muestras_de_audio`). Solo los videos de archivo con audio las llenan.
    muestras: Arc<Mutex<Vec<f32>>>,
    /// El video llegó al final y quedó quieto (sin bucle).
    terminado: Arc<AtomicBool>,
    /// Algo que tiene que seguir vivo mientras el video exista (la sesión de captura del
    /// portal). Va al final: se suelta después de apagar la tubería.
    _guardia: Mutex<Option<Box<dyn std::any::Any + Send>>>,
}

impl Video {
    /// Abre un archivo y empieza a reproducirlo.
    ///
    /// Devuelve `Result`: o `Ok(Video)` o `Err(...)` si algo falló (archivo
    /// inexistente, falta un plugin de GStreamer, etc.). El `?` de abajo
    /// significa "si esto es un error, salí de la función y devolvelo".
    pub fn abrir(ruta: impl AsRef<Path>) -> Result<Video, Box<dyn Error>> {
        Video::abrir_con(ruta, Ajustes::default(), None)
    }

    /// Lo mismo que `abrir`, con ajustes de reproducción (bucle, velocidad, tramo, audio) y,
    /// si se pasa `limite` (ancho y alto), con los cuadros **reducidos** para que no pasen de
    /// ese tamaño (conservando la proporción). Un video 4K en una composición de 1080p se
    /// reduce al principio: se convierte y se copia a la GPU cuatro veces menos información.
    /// Un video más chico que el límite no se agranda.
    pub fn abrir_con(
        ruta: impl AsRef<Path>,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
    ) -> Result<Video, Box<dyn Error>> {
        Video::abrir_origen(
            &Origen::Archivo(ruta.as_ref().to_path_buf()),
            ajustes,
            limite,
        )
    }

    /// Abre cualquier origen de imagen: un archivo, una cámara o una transmisión de red.
    /// Los orígenes en vivo (cámara, red) no tienen duración: el tramo, la velocidad y el
    /// salto no hacen nada, y no hay bucle porque la imagen nunca termina.
    pub fn abrir_origen(
        origen_de_imagen: &Origen,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
    ) -> Result<Video, Box<dyn Error>> {
        Video::abrir_con_guardia(origen_de_imagen, ajustes, limite, None)
    }

    /// Abre una captura de pantalla o ventana ya autorizada. El video se queda con la sesión
    /// del portal: al cerrarse el video, la sesión se cierra.
    pub fn abrir_captura(
        sesion: crate::captura::SesionDeCaptura,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
        audio: Option<AudioDeCaptura>,
    ) -> Result<Video, Box<dyn Error>> {
        let origen = Origen::PipeWire {
            fd: sesion.fd(),
            nodo: sesion.nodo,
            audio,
        };
        Video::abrir_con_guardia(&origen, ajustes, limite, Some(Box::new(sesion)))
    }

    fn abrir_con_guardia(
        origen_de_imagen: &Origen,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
        guardia: Option<Box<dyn std::any::Any + Send>>,
    ) -> Result<Video, Box<dyn Error>> {
        gst::init()?;

        // La tubería se arma **elemento por elemento**, sin pasar por un texto estilo
        // `gst-launch`. Si la ruta se pegara dentro de un texto, un espacio o una comilla
        // en el nombre del archivo se interpretarían como sintaxis y romperían la tubería.
        // Con `property(...)` la ruta es un dato y nunca se interpreta.
        let pipeline = gst::Pipeline::new();
        // `fuente_viva` es el elemento que produce la imagen de una cámara; los demás
        // orígenes se abren directamente con `uridecodebin`.
        let (origen, fuente_viva) = match origen_de_imagen {
            Origen::Archivo(ruta) => {
                // GStreamer necesita una URI (`file:///...`), no una ruta suelta.
                // `filename_to_uri` la codifica bien: un espacio pasa a ser `%20`, y lo mismo
                // con acentos, `#`, `%`...
                let ruta_absoluta = std::fs::canonicalize(ruta)?;
                let uri = gst::glib::filename_to_uri(&ruta_absoluta, None)?;
                let origen = gst::ElementFactory::make("uridecodebin")
                    .property("uri", uri.as_str())
                    .build()?;
                (origen, None)
            }
            Origen::Url(url) => {
                let origen = gst::ElementFactory::make("uridecodebin")
                    .property("uri", url.as_str())
                    .build()?;
                (origen, None)
            }
            Origen::PipeWire { fd, nodo, .. } => {
                let captura = gst::ElementFactory::make("pipewiresrc")
                    .property("fd", *fd)
                    .property("path", nodo.to_string())
                    .property("do-timestamp", true)
                    .build()?;
                // Igual que con la cámara: memoria común, no DMA-BUF.
                let filtro_de_captura = gst::ElementFactory::make("capsfilter")
                    .property("caps", gst::Caps::from_str("video/x-raw")?)
                    .build()?;
                let origen = gst::ElementFactory::make("decodebin").build()?;
                (origen, Some((captura, filtro_de_captura)))
            }
            Origen::Camara(dispositivo) => {
                let camara = gst::ElementFactory::make("v4l2src")
                    .property("device", dispositivo.as_str())
                    .build()?;
                // Algunas cámaras (la virtual de OBS, por ejemplo) ofrecen los cuadros como
                // memoria DMA-BUF, que el resto de la tubería no sabe recibir. Este filtro
                // pide memoria común (`video/x-raw`) o los formatos comprimidos de siempre
                // (MJPEG, H.264), que `decodebin` resuelve solo.
                let filtro_de_camara = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::from_str("video/x-raw; image/jpeg; video/x-h264")?,
                    )
                    .build()?;
                let origen = gst::ElementFactory::make("decodebin").build()?;
                (origen, Some((camara, filtro_de_camara)))
            }
        };
        // Primero se reduce (si hace falta) y recién después se convierte a RGBA: convertir un
        // cuadro más chico cuesta menos.
        let escalar = gst::ElementFactory::make("videoscale").build()?;
        let mut tamano = gst::Caps::builder("video/x-raw");
        if let Some((ancho, alto)) = limite {
            tamano = tamano
                .field("width", gst::IntRange::new(1, ancho.max(1) as i32))
                .field("height", gst::IntRange::new(1, alto.max(1) as i32));
        }
        let filtro = gst::ElementFactory::make("capsfilter")
            .property("caps", tamano.build())
            .build()?;
        let convertir = gst::ElementFactory::make("videoconvert").build()?;
        // `caps(...)` obliga al appsink a recibir RGBA, para no convertir por nuestra cuenta.
        let appsink = gst_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .build(),
            )
            .build();
        // Un video de archivo se reproduce al ritmo de sus marcas de tiempo (`sync`); una imagen
        // en vivo se entrega apenas llega.
        appsink.set_property("sync", !origen_de_imagen.es_en_vivo());
        pipeline.add_many([&origen, &escalar, &filtro, &convertir, appsink.upcast_ref()])?;
        if let Some((camara, filtro_de_camara)) = &fuente_viva {
            pipeline.add_many([camara, filtro_de_camara])?;
            gst::Element::link_many([camara, filtro_de_camara, &origen])?;
        }
        gst::Element::link_many([&escalar, &filtro, &convertir, appsink.upcast_ref()])?;

        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        // `uridecodebin` no sabe qué trae el archivo hasta abrirlo: cuando descubre un flujo
        // (video, audio...) crea una "pata" (*pad*) y avisa. Acá se conecta solo la de video.
        let entrada_video = escalar
            .static_pad("sink")
            .ok_or("videoscale no tiene entrada")?;
        let entrada_para_aviso = entrada_video.clone();
        let ajustes_compartidos = Arc::new(Mutex::new(ajustes));
        let volumen: Arc<Mutex<Option<gst::Element>>> = Arc::new(Mutex::new(None));
        let muestras: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        conectar_patas(
            &origen,
            &pipeline,
            entrada_video,
            Arc::clone(&ajustes_compartidos),
            Arc::clone(&volumen),
            Arc::clone(&muestras),
        );
        // El sonido de una captura no viene en el video: es otra fuente de PipeWire.
        if let Origen::PipeWire {
            audio: Some(audio), ..
        } = origen_de_imagen
        {
            if let Err(e) = agregar_audio_de_captura(&pipeline, audio, &volumen, &ajustes) {
                // Sin sonido, pero el video sigue.
                warn_audio(&e.to_string());
            }
        }
        // Cuando ya se descubrieron todos los flujos del archivo: si ninguno era de video,
        // se avisa con un mensaje claro (si no, GStreamer solo diría "Internal data stream error").
        let error_sin_video = Arc::clone(&error);
        origen.connect_no_more_pads(move |_| {
            if !entrada_para_aviso.is_linked()
                && let Ok(mut guardado) = error_sin_video.lock()
            {
                *guardado = Some("el archivo no tiene una pista de video".into());
            }
        });

        // Canal con capacidad para 2 frames. Si el consumidor va lento, el
        // canal se llena y descartamos frames en vez de acumular memoria.
        let (emisor, frames): (SyncSender<Frame>, Receiver<Frame>) = sync_channel(2);

        // Este closure se ejecuta en un hilo propio de GStreamer cada vez que
        // llega un frame. `move` le transfiere el `emisor` (ownership).
        // `new_preroll` entrega el cuadro que queda listo al saltar de posición con el video
        // en pausa: sin esto, buscar con la pausa puesta no actualizaría la imagen.
        let emisor_preroll = emisor.clone();
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let frame = frame_de_muestra(&muestra).ok_or(gst::FlowError::Error)?;

                    // try_send no bloquea: si el canal está lleno, el frame se pierde.
                    let _ = emisor.try_send(frame);
                    Ok(gst::FlowSuccess::Ok)
                })
                .new_preroll(move |sink| {
                    let muestra = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    let frame = frame_de_muestra(&muestra).ok_or(gst::FlowError::Error)?;
                    let _ = emisor_preroll.try_send(frame);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        pipeline.set_state(gst::State::Playing)?;

        // El "bus" es por donde GStreamer avisa eventos: fin del video, errores...
        // Un hilo lo escucha: al llegar el fin (EOS) vuelve al segundo 0 (loop).
        // `Arc<AtomicBool>` es un booleano que varios hilos pueden leer/escribir
        // sin pisarse; `Arc` cuenta cuántos lo usan y lo libera al final.
        let terminar = Arc::new(AtomicBool::new(false));
        let terminado = Arc::new(AtomicBool::new(false));
        let hilo_bus = {
            let terminar = Arc::clone(&terminar);
            let terminado = Arc::clone(&terminado);
            let ajustes = Arc::clone(&ajustes_compartidos);
            let error = Arc::clone(&error);
            let pipeline = pipeline.clone(); // clonar acá es barato: es una referencia contada
            let bus = pipeline.bus().ok_or("la tubería no tiene bus")?;
            std::thread::spawn(move || {
                let mut salto_inicial_hecho = false;
                while !terminar.load(Ordering::Relaxed) {
                    // Espera hasta 100 ms un mensaje; así revisa `terminar` seguido.
                    let Some(msg) = bus.timed_pop_filtered(
                        gst::ClockTime::from_mseconds(100),
                        &[
                            gst::MessageType::Eos,
                            gst::MessageType::Error,
                            gst::MessageType::AsyncDone,
                        ],
                    ) else {
                        continue;
                    };
                    match msg.view() {
                        gst::MessageView::AsyncDone(_) => {
                            // Ya se conoce la duración: si el clip pide otra velocidad o un
                            // tramo, se hace el salto una sola vez, al empezar.
                            if !salto_inicial_hecho {
                                salto_inicial_hecho = true;
                                let a = ajustes.lock().map(|a| *a).unwrap_or_default();
                                if a.pide_salto() {
                                    aplicar_transporte(&pipeline, &a, None);
                                }
                            }
                        }
                        gst::MessageView::Eos(_) => {
                            let a = ajustes.lock().map(|a| *a).unwrap_or_default();
                            if a.bucle {
                                // Vuelve a la entrada del tramo (o al principio).
                                if !aplicar_transporte(&pipeline, &a, None) {
                                    let _ = pipeline.seek_simple(
                                        gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                        gst::ClockTime::ZERO,
                                    );
                                }
                            } else {
                                // Sin bucle queda quieto en el último cuadro.
                                terminado.store(true, Ordering::Relaxed);
                            }
                        }
                        // Un error de la rama de audio (por ejemplo, sin placa de sonido) no
                        // tiene que tirar abajo el video.
                        gst::MessageView::Error(e)
                            if msg
                                .src()
                                .is_some_and(|s| s.path_string().contains(PREFIJO_AUDIO)) =>
                        {
                            warn_audio(&e.error().to_string());
                        }
                        gst::MessageView::Error(e) => {
                            // Se guarda para que quien reproduce lo pueda mostrar.
                            if let Ok(mut guardado) = error.lock() {
                                guardado.get_or_insert_with(|| e.error().to_string());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            })
        };

        Ok(Video {
            pipeline,
            frames: Mutex::new(frames),
            terminar,
            hilo_bus: Some(hilo_bus),
            error,
            ajustes: ajustes_compartidos,
            volumen,
            muestras,
            terminado,
            _guardia: Mutex::new(guardia),
        })
    }

    /// Entrega las muestras de audio (mono, 48 kHz, de −1 a 1) que llegaron desde la última vez,
    /// para analizar el sonido del clip. Salen **antes** del volumen: sirven aunque el clip
    /// esté silenciado. Vacío si el video no tiene audio o es una fuente en vivo. Si nadie las
    /// pide, solo se guardan las últimas `MAXIMO_DE_MUESTRAS`.
    pub fn tomar_muestras_de_audio(&self) -> Vec<f32> {
        self.muestras
            .lock()
            .map(|mut m| std::mem::take(&mut *m))
            .unwrap_or_default()
    }

    /// Cambia los ajustes de un video que ya está sonando: volumen y silencio al instante;
    /// velocidad y tramo con un salto desde donde está (queda dentro del tramo nuevo).
    pub fn ajustar(&self, nuevos: Ajustes) {
        let anteriores = match self.ajustes.lock() {
            Ok(mut a) => std::mem::replace(&mut *a, nuevos),
            Err(_) => return,
        };
        aplicar_audio(&self.volumen, &nuevos);
        let cambio_de_tramo = anteriores.velocidad != nuevos.velocidad
            || anteriores.inicio != nuevos.inicio
            || anteriores.fin != nuevos.fin;
        // Si estaba quieto en el final y se activa el bucle, vuelve a empezar.
        let reanudar = self.terminado.load(Ordering::Relaxed) && nuevos.bucle;
        if reanudar {
            self.terminado.store(false, Ordering::Relaxed);
            aplicar_transporte(&self.pipeline, &nuevos, None);
        } else if cambio_de_tramo {
            let posicion = self.pipeline.query_position::<gst::ClockTime>();
            aplicar_transporte(&self.pipeline, &nuevos, posicion);
        }
    }

    /// Pausa el video (`true`) o lo hace seguir (`false`). En pausa el último cuadro queda
    /// en pantalla.
    pub fn pausar(&self, en_pausa: bool) {
        let estado = if en_pausa {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        let _ = self.pipeline.set_state(estado);
    }

    /// Dónde va el video y cuánto dura, en segundos. `None` si todavía no se sabe.
    pub fn posicion(&self) -> Option<(f64, f64)> {
        let posicion = self.pipeline.query_position::<gst::ClockTime>()?;
        let duracion = self.pipeline.query_duration::<gst::ClockTime>()?;
        Some((posicion.seconds_f64(), duracion.seconds_f64()))
    }

    /// Salta a un punto del video: `fraccion` de 0 a 1 sobre su duración (siempre dentro
    /// del tramo elegido).
    pub fn buscar(&self, fraccion: f32) {
        let Some(duracion) = self.pipeline.query_duration::<gst::ClockTime>() else {
            return;
        };
        let a = self.ajustes.lock().map(|a| *a).unwrap_or_default();
        let destino = gst::ClockTime::from_nseconds(
            (duracion.nseconds() as f64 * fraccion.clamp(0.0, 1.0) as f64) as u64,
        );
        self.terminado.store(false, Ordering::Relaxed);
        aplicar_transporte(&self.pipeline, &a, Some(destino));
    }

    /// El error que informó GStreamer después de arrancar, si hubo alguno. Un archivo que
    /// no existe falla en `abrir`; uno dañado o con un formato que no se puede
    /// decodificar recién falla acá, un instante después.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok()?.clone()
    }

    /// Devuelve el frame más nuevo disponible, o `None` si no llegó ninguno desde
    /// la última vez. Si se acumularon varios (la app fue más lenta que el video),
    /// descarta los viejos: mostrar el último evita que el video se atrase.
    pub fn ultimo_frame(&self) -> Option<Frame> {
        let canal = self.frames.lock().ok()?;
        canal.try_iter().last()
    }
}

/// Los nombres de los elementos de la rama de audio empiezan así: sirve para reconocer sus
/// errores en el bus y no confundirlos con los del video.
const PREFIJO_AUDIO: &str = "audio_";

fn warn_audio(motivo: &str) {
    eprintln!("aviso: el audio de un clip falló y sigue sin sonido: {motivo}");
}

/// `uridecodebin` no sabe qué trae el archivo hasta abrirlo: cuando descubre un flujo
/// (video, audio...) crea una "pata" (*pad*) y avisa. La de video se conecta a `entrada`; la
/// de audio, a una rama propia (convertir, volumen, salida de sonido).
fn conectar_patas(
    origen: &gst::Element,
    pipeline: &gst::Pipeline,
    entrada: gst::Pad,
    ajustes: Arc<Mutex<Ajustes>>,
    volumen: Arc<Mutex<Option<gst::Element>>>,
    muestras: Arc<Mutex<Vec<f32>>>,
) {
    // Una referencia **débil** a la tubería: el callback vive dentro de uno de sus elementos,
    // y una referencia fuerte formaría un ciclo que impediría liberarla.
    let pipeline = pipeline.downgrade();
    origen.connect_pad_added(move |_, pata| {
        let caps = pata.current_caps().unwrap_or_else(|| pata.query_caps(None));
        let nombre = caps
            .structure(0)
            .map(|estructura| estructura.name().to_string())
            .unwrap_or_default();
        if nombre.starts_with("video/") {
            if !entrada.is_linked() {
                let _ = pata.link(&entrada);
            }
        } else if nombre.starts_with("audio/")
            && let Some(pipeline) = pipeline.upgrade()
        {
            let a = ajustes.lock().map(|a| *a).unwrap_or_default();
            if let Err(e) = agregar_audio(&pipeline, pata, &volumen, &a, Some(&muestras)) {
                warn_audio(&e.to_string());
            }
        }
    });
}

/// Conecta solo la pata de video a `entrada` (para las miniaturas, que no necesitan audio).
fn conectar_pata_de_video(origen: &gst::Element, entrada: gst::Pad) {
    origen.connect_pad_added(move |_, pata| {
        if entrada.is_linked() {
            return;
        }
        let caps = pata.current_caps().unwrap_or_else(|| pata.query_caps(None));
        let es_video = caps
            .structure(0)
            .is_some_and(|estructura| estructura.name().starts_with("video/"));
        if es_video {
            let _ = pata.link(&entrada);
        }
    });
}

/// Arma la rama de audio: convertir → (mantener el tono si cambia la velocidad) → volumen →
/// salida de sonido del sistema. Todo con nombres que empiezan con `PREFIJO_AUDIO`.
///
/// `muestras` es `Some` solo en los **videos de archivo**: se arma además una derivación para
/// analizar el sonido (ver más abajo). `None` es el sonido de una captura (`en_vivo`).
///
/// Sin `muestras`, es el sonido de una captura. Cambia dos cosas, medidas con
/// `examples/prueba_audio.rs` (cortes en 8 s con Firefox sonando): sin `scaletempo`, que
/// procesa por bloques y corta una fuente en vivo; y con la salida en `sync=false`, que
/// reproduce los bloques a medida que llegan en vez de compararlos con el reloj y
/// resincronizar («Unexpected discontinuity in audio time»). Con `sync=true` y `scaletempo`
/// daba 7 cortes; con las dos cosas, 0. Un video de archivo sí necesita las dos: `scaletempo`
/// para mantener el tono al cambiar la velocidad y la sincronía con su propio reloj.
fn agregar_audio(
    pipeline: &gst::Pipeline,
    pata: &gst::Pad,
    volumen_compartido: &Arc<Mutex<Option<gst::Element>>>,
    ajustes: &Ajustes,
    muestras: Option<&Arc<Mutex<Vec<f32>>>>,
) -> Result<(), Box<dyn Error>> {
    // Sin derivación es el sonido de una captura (en vivo).
    let en_vivo = muestras.is_none();
    let nombrado = |fabrica: &str, nombre: &str| {
        gst::ElementFactory::make(fabrica)
            .name(format!("{PREFIJO_AUDIO}{nombre}"))
            .build()
    };
    let convertir = nombrado("audioconvert", "convertir")?;
    let convertir2 = nombrado("audioconvert", "convertir2")?;
    let muestreo = nombrado("audioresample", "muestreo")?;
    let volumen = nombrado("volume", "volumen")?;
    let salida = nombrado("autoaudiosink", "salida")?;

    salida.set_property("sync", !en_vivo);

    // Con derivación, el sonido se reparte en dos con un `tee`; cada rama lleva su `queue` para
    // que una no frene a la otra.
    let reparto = muestras.map(|_| nombrado("tee", "derivar")).transpose()?;
    let cola = muestras
        .map(|_| nombrado("queue", "cola_sonido"))
        .transpose()?;

    // Lo que sigue al primer `audioconvert` en la rama que se oye.
    let mut rama: Vec<&gst::Element> = Vec::new();
    if let Some(cola) = &cola {
        rama.push(cola);
    }
    // Mantiene el tono al cambiar la velocidad (solo videos de archivo).
    let tono = (!en_vivo)
        .then(|| nombrado("scaletempo", "tono"))
        .transpose()?;
    if let Some(tono) = &tono {
        rama.push(tono);
    }
    rama.extend([&convertir2, &muestreo, &volumen, &salida]);

    let mut todos: Vec<&gst::Element> = vec![&convertir];
    todos.extend(reparto.as_ref());
    todos.extend(rama.iter().copied());
    pipeline.add_many(todos.iter().copied())?;
    match &reparto {
        Some(reparto) => {
            gst::Element::link_many([&convertir, reparto])?;
            reparto.link(rama[0])?;
            gst::Element::link_many(rama.iter().copied())?;
        }
        None => {
            let mut cadena = vec![&convertir];
            cadena.extend(rama.iter().copied());
            gst::Element::link_many(cadena)?;
        }
    }
    for e in &todos {
        e.sync_state_with_parent()?;
    }
    pata.link(
        &convertir
            .static_pad("sink")
            .ok_or("audioconvert no tiene entrada")?,
    )?;

    // La derivación sale del `tee`, antes de `scaletempo` y de `volume`: así el análisis no
    // depende de la velocidad ni del silencio del clip.
    if let (Some(reparto), Some(muestras)) = (&reparto, muestras) {
        derivar_muestras(pipeline, reparto, muestras)?;
    }

    if let Ok(mut guardado) = volumen_compartido.lock() {
        *guardado = Some(volumen);
    }
    aplicar_audio(volumen_compartido, ajustes);
    Ok(())
}

/// Cuántas muestras como máximo se guardan sin que nadie las pida (2 s a 48 kHz).
const MAXIMO_DE_MUESTRAS: usize = 96_000;

/// La rama de análisis: `queue ! audioconvert ! audioresample ! mono F32 48 kHz ! appsink`.
/// Las muestras llegan al ritmo de la reproducción (el `appsink` sigue el reloj) y se juntan
/// en `muestras`.
fn derivar_muestras(
    pipeline: &gst::Pipeline,
    reparto: &gst::Element,
    muestras: &Arc<Mutex<Vec<f32>>>,
) -> Result<(), Box<dyn Error>> {
    let nombrado = |fabrica: &str, nombre: &str| {
        gst::ElementFactory::make(fabrica)
            .name(format!("{PREFIJO_AUDIO}{nombre}"))
            .build()
    };
    let cola = nombrado("queue", "cola_analisis")?;
    let convertir = nombrado("audioconvert", "convertir_analisis")?;
    let muestreo = nombrado("audioresample", "muestreo_analisis")?;
    let formato = gst::ElementFactory::make("capsfilter")
        .name(format!("{PREFIJO_AUDIO}formato_analisis"))
        .property(
            "caps",
            gst::Caps::builder("audio/x-raw")
                .field("format", "F32LE")
                .field("channels", 1i32)
                .field("rate", 48_000i32)
                .field("layout", "interleaved")
                .build(),
        )
        .build()?;
    let appsink = gst_app::AppSink::builder()
        .name(format!("{PREFIJO_AUDIO}analisis"))
        .sync(true)
        .build();
    // Que no necesite «preroll»: si esta rama no recibe nada (el audio terminó antes), no tiene
    // que frenar los cambios de estado de toda la tubería.
    appsink.set_property("async", false);

    let destino = Arc::clone(muestras);
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = muestra.buffer().ok_or(gst::FlowError::Error)?;
                let mapa = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                if let Ok(mut guardadas) = destino.lock() {
                    guardadas.extend(
                        mapa.chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                    // Si nadie las pide, se tiran las más viejas: no crece sin límite.
                    if guardadas.len() > MAXIMO_DE_MUESTRAS {
                        let sobran = guardadas.len() - MAXIMO_DE_MUESTRAS;
                        guardadas.drain(..sobran);
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let elementos = [&cola, &convertir, &muestreo, &formato, appsink.upcast_ref()];
    pipeline.add_many(elementos)?;
    gst::Element::link_many(elementos)?;
    reparto.link(&cola)?;
    for e in elementos {
        e.sync_state_with_parent()?;
    }
    Ok(())
}

/// Agrega a la tubería el sonido de una captura: una fuente de PipeWire (el escritorio o una
/// aplicación) conectada a la misma rama de audio que usan los videos, así vale el volumen y
/// el silencio del clip. El elemento se llama `audio_fuente`: si falla, el bus lo reconoce
/// como un error de audio y el video sigue.
fn agregar_audio_de_captura(
    pipeline: &gst::Pipeline,
    audio: &AudioDeCaptura,
    volumen_compartido: &Arc<Mutex<Option<gst::Element>>>,
    ajustes: &Ajustes,
) -> Result<(), Box<dyn Error>> {
    let fuente = gst::ElementFactory::make("pipewiresrc")
        .name(format!("{PREFIJO_AUDIO}fuente"))
        .property("do-timestamp", true)
        .build()?;
    match audio {
        // Sin destino y pidiendo «capturar la salida»: PipeWire toma el monitor de la salida
        // de sonido por defecto.
        AudioDeCaptura::Escritorio => {
            fuente.set_property(
                "stream-properties",
                gst::Structure::builder("props")
                    .field("stream.capture.sink", true)
                    .build(),
            );
        }
        // Solo el flujo de esa aplicación: no entra el sonido de este programa.
        AudioDeCaptura::Aplicacion(nombre) => {
            let serial = crate::audio::serial_de_aplicacion(nombre)
                .ok_or_else(|| format!("«{nombre}» no está sonando: la captura va sin audio"))?;
            fuente.set_property("target-object", serial.to_string());
        }
    }
    pipeline.add(&fuente)?;
    let pata = fuente
        .static_pad("src")
        .ok_or("pipewiresrc no tiene salida")?;
    agregar_audio(pipeline, &pata, volumen_compartido, ajustes, None)?;
    fuente.sync_state_with_parent()?;
    Ok(())
}

/// Pone el volumen y el silencio en la rama de audio (si el video la tiene).
fn aplicar_audio(volumen: &Arc<Mutex<Option<gst::Element>>>, a: &Ajustes) {
    if let Ok(guardado) = volumen.lock()
        && let Some(elemento) = guardado.as_ref()
    {
        elemento.set_property("volume", a.volumen.clamp(0.0, 2.0) as f64);
        elemento.set_property("mute", a.silenciado);
    }
}

/// Salta con los ajustes dados: velocidad y tramo. `desde` es la posición de la que se
/// parte (queda dentro del tramo); `None` empieza por la entrada del tramo. Devuelve
/// `false` si no se pudo (por ejemplo, si todavía no se conoce la duración).
fn aplicar_transporte(
    pipeline: &gst::Pipeline,
    a: &Ajustes,
    desde: Option<gst::ClockTime>,
) -> bool {
    let Some(duracion) = pipeline.query_duration::<gst::ClockTime>() else {
        return false;
    };
    let (inicio, fin) = limites_del_tramo(a, duracion.nseconds());
    let fin_ns = fin.unwrap_or(duracion.nseconds());
    let comienzo = desde.map_or(inicio, |d| d.nseconds().clamp(inicio, fin_ns));
    let (tipo_de_fin, fin) = match fin {
        Some(f) => (gst::SeekType::Set, Some(gst::ClockTime::from_nseconds(f))),
        None => (gst::SeekType::None, gst::ClockTime::NONE),
    };
    pipeline
        .seek(
            a.velocidad.clamp(0.1, 4.0) as f64,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::SeekType::Set,
            Some(gst::ClockTime::from_nseconds(comienzo)),
            tipo_de_fin,
            fin,
        )
        .is_ok()
}

/// Convierte una muestra de GStreamer (un frame RGBA) en un `Frame` propio.
fn frame_de_muestra(muestra: &gst::Sample) -> Option<Frame> {
    let buffer = muestra.buffer()?;
    let info = gstreamer_video::VideoInfo::from_caps(muestra.caps()?).ok()?;
    let mapa = buffer.map_readable().ok()?;
    Some(Frame {
        ancho: info.width(),
        alto: info.height(),
        datos: mapa.as_slice().to_vec(),
    })
}

/// Cuánto esperar a que GStreamer abra el archivo o termine un salto, en segundos.
const ESPERA_DE_MINIATURA: u64 = 10;

/// Achica un frame a `ancho_destino` píxeles de ancho, conservando la proporción.
///
/// Cada píxel nuevo es el **promedio** del bloque de píxeles originales que cubre (mucho
/// mejor que descartar píxeles, que deja la imagen dentada). Si el frame ya es más angosto
/// que `ancho_destino`, se devuelve igual.
pub fn reducir(frame: Frame, ancho_destino: u32) -> Frame {
    if ancho_destino == 0 || frame.ancho <= ancho_destino || frame.alto == 0 {
        return frame;
    }
    let (ancho_origen, alto_origen) = (frame.ancho as usize, frame.alto as usize);
    let ancho_nuevo = ancho_destino as usize;
    let alto_nuevo = (alto_origen * ancho_nuevo / ancho_origen).max(1);

    let mut datos = vec![0u8; ancho_nuevo * alto_nuevo * 4];
    for y in 0..alto_nuevo {
        // El rango de filas y de columnas originales que cubre este píxel nuevo (al menos una).
        let (y0, y1) = (
            y * alto_origen / alto_nuevo,
            ((y + 1) * alto_origen / alto_nuevo).max(y * alto_origen / alto_nuevo + 1),
        );
        for x in 0..ancho_nuevo {
            let (x0, x1) = (
                x * ancho_origen / ancho_nuevo,
                ((x + 1) * ancho_origen / ancho_nuevo).max(x * ancho_origen / ancho_nuevo + 1),
            );
            let mut suma = [0u32; 4];
            for fila in y0..y1.min(alto_origen) {
                for columna in x0..x1.min(ancho_origen) {
                    let i = (fila * ancho_origen + columna) * 4;
                    for (canal, acumulado) in suma.iter_mut().enumerate() {
                        *acumulado += frame.datos[i + canal] as u32;
                    }
                }
            }
            let cantidad = ((y1.min(alto_origen) - y0) * (x1.min(ancho_origen) - x0)).max(1) as u32;
            let destino = (y * ancho_nuevo + x) * 4;
            for (canal, acumulado) in suma.iter().enumerate() {
                datos[destino + canal] = (acumulado / cantidad) as u8;
            }
        }
    }
    Frame {
        ancho: ancho_nuevo as u32,
        alto: alto_nuevo as u32,
        datos,
    }
}

/// Por qué falló una miniatura, en palabras: primero si el archivo no tiene video, y si no,
/// el error que GStreamer dejó en el bus.
fn motivo_del_fallo(pipeline: &gst::Pipeline, sin_video: &AtomicBool) -> String {
    if sin_video.load(Ordering::Relaxed) {
        return "el archivo no tiene una pista de video".into();
    }
    pipeline
        .bus()
        .and_then(|bus| {
            // El error llega al bus un instante después de que falla el cambio de estado.
            bus.timed_pop_filtered(
                gst::ClockTime::from_mseconds(200),
                &[gst::MessageType::Error],
            )
        })
        .and_then(|mensaje| match mensaje.view() {
            gst::MessageView::Error(e) => Some(e.error().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "no se pudo abrir el archivo".into())
}

/// Un cuadro representativo de un video, achicado a `ancho` píxeles de ancho (el alto
/// conserva la proporción).
///
/// El primer cuadro de un video suele ser negro, así que se salta al 12 % de su duración.
/// Bloquea hasta tenerlo: hay que llamarla desde un hilo aparte, no desde el principal.
pub fn miniatura(ruta: impl AsRef<Path>, ancho: u32) -> Result<Frame, Box<dyn Error>> {
    gst::init()?;
    let ruta_absoluta = std::fs::canonicalize(ruta.as_ref())?;
    let uri = gst::glib::filename_to_uri(&ruta_absoluta, None)?;

    // No se le pide a GStreamer que achique (`videoscale` con un solo lado fijado no
    // conserva la proporción): se toma el cuadro entero y se achica con `reducir`.
    let pipeline = gst::Pipeline::new();
    let origen = gst::ElementFactory::make("uridecodebin")
        .property("uri", uri.as_str())
        .build()?;
    let convertir = gst::ElementFactory::make("videoconvert").build()?;
    let appsink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .build(),
        )
        .build();
    pipeline.add_many([&origen, &convertir, appsink.upcast_ref()])?;
    convertir.link(&appsink)?;

    let entrada = convertir
        .static_pad("sink")
        .ok_or("videoconvert no tiene entrada")?;
    let sin_video = Arc::new(AtomicBool::new(false));
    {
        let marca = Arc::clone(&sin_video);
        let entrada = entrada.clone();
        origen.connect_no_more_pads(move |_| {
            if !entrada.is_linked() {
                marca.store(true, Ordering::Relaxed);
            }
        });
    }
    conectar_pata_de_video(&origen, entrada);

    let espera = gst::ClockTime::from_seconds(ESPERA_DE_MINIATURA);
    let tomar = || -> Option<Frame> {
        // En pausa alcanza: no hace falta reproducir, solo tener un cuadro listo ("preroll").
        pipeline.set_state(gst::State::Paused).ok()?;
        pipeline.state(espera).0.ok()?;

        // Se salta al 12 % del video. Si no se puede (por ejemplo, un video sin duración
        // conocida), se usa el primer cuadro.
        if let Some(duracion) = pipeline.query_duration::<gst::ClockTime>()
            && pipeline
                .seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                    duracion / 8,
                )
                .is_ok()
        {
            pipeline.state(espera).0.ok()?;
        }
        let muestra = appsink.try_pull_preroll(espera)?;
        frame_de_muestra(&muestra)
    };

    let frame = tomar();
    // El motivo se lee antes de apagar la tubería: después, el bus ya no tiene nada.
    let motivo = if frame.is_none() {
        Some(motivo_del_fallo(&pipeline, &sin_video))
    } else {
        None
    };
    let _ = pipeline.set_state(gst::State::Null);

    match frame {
        Some(frame) => Ok(reducir(frame, ancho)),
        None => Err(motivo.unwrap_or_default().into()),
    }
}

impl Drop for Video {
    /// Se ejecuta solo cuando el `Video` deja de existir: frena el hilo del bus y apaga la tubería.
    fn drop(&mut self) {
        self.terminar.store(true, Ordering::Relaxed);
        if let Some(hilo) = self.hilo_bus.take() {
            let _ = hilo.join();
        }
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un frame de prueba de `ancho` × `alto` donde cada píxel vale `valor(x, y)` en sus 4 canales.
    fn frame(ancho: u32, alto: u32, valor: impl Fn(u32, u32) -> u8) -> Frame {
        let mut datos = Vec::new();
        for y in 0..alto {
            for x in 0..ancho {
                datos.extend([valor(x, y); 4]);
            }
        }
        Frame { ancho, alto, datos }
    }

    #[test]
    fn reducir_conserva_la_proporcion() {
        let r = reducir(frame(1920, 1080, |_, _| 10), 240);
        assert_eq!((r.ancho, r.alto), (240, 135));
        assert_eq!(r.datos.len(), 240 * 135 * 4);
        // Un frame vertical también.
        let v = reducir(frame(1080, 1920, |_, _| 10), 108);
        assert_eq!((v.ancho, v.alto), (108, 192));
    }

    #[test]
    fn reducir_promedia_los_bloques() {
        // 4×2 reducido a 2×1: cada píxel nuevo es el promedio de un bloque de 2×2.
        // Columnas 0-1 valen 0 y 100 → promedio 50; columnas 2-3 valen 200 y 200 → 200.
        let origen = frame(4, 2, |x, _| [0, 100, 200, 200][x as usize]);
        let r = reducir(origen, 2);
        assert_eq!((r.ancho, r.alto), (2, 1));
        assert_eq!(&r.datos[0..4], &[50, 50, 50, 50]);
        assert_eq!(&r.datos[4..8], &[200, 200, 200, 200]);
    }

    #[test]
    fn reducir_un_color_liso_lo_deja_igual() {
        let r = reducir(frame(100, 60, |_, _| 77), 10);
        assert!(r.datos.iter().all(|&p| p == 77));
    }

    #[test]
    fn reducir_no_agranda() {
        let r = reducir(frame(100, 50, |_, _| 1), 200);
        assert_eq!((r.ancho, r.alto), (100, 50));
        let igual = reducir(frame(100, 50, |_, _| 1), 100);
        assert_eq!((igual.ancho, igual.alto), (100, 50));
    }

    #[test]
    fn el_tramo_completo_no_tiene_fin_y_empieza_en_cero() {
        let a = Ajustes::default();
        assert_eq!(limites_del_tramo(&a, 10_000_000_000), (0, None));
        assert!(!a.pide_salto());
    }

    #[test]
    fn un_tramo_se_convierte_en_instantes() {
        let a = Ajustes {
            inicio: 0.25,
            fin: 0.75,
            ..Ajustes::default()
        };
        assert_eq!(
            limites_del_tramo(&a, 10_000_000_000),
            (2_500_000_000, Some(7_500_000_000))
        );
        assert!(a.pide_salto());
    }

    #[test]
    fn cambiar_la_velocidad_pide_un_salto_aunque_el_tramo_sea_completo() {
        let a = Ajustes {
            velocidad: 2.0,
            ..Ajustes::default()
        };
        assert!(a.pide_salto());
    }

    #[test]
    fn las_fuentes_en_vivo_no_esperan_al_reloj() {
        assert!(!Origen::Archivo("a.mp4".into()).es_en_vivo());
        assert!(Origen::Camara("/dev/video0".into()).es_en_vivo());
        assert!(
            Origen::PipeWire {
                fd: -1,
                nodo: 5,
                audio: None
            }
            .es_en_vivo()
        );
        assert!(Origen::Url("rtsp://x/y".into()).es_en_vivo());
        assert!(Origen::Url("SRT://x:9000".into()).es_en_vivo());
        // Un archivo servido por http sí se reproduce a su ritmo.
        assert!(!Origen::Url("http://x/a.mp4".into()).es_en_vivo());
    }

    /// Prueba con PipeWire de verdad: una «aplicación» emite un tono y se captura solo a ella.
    /// Suena un instante por los parlantes. Se corre a mano:
    /// `cargo test -p vm_video audio_de_una_aplicacion -- --ignored`
    #[test]
    #[ignore = "necesita PipeWire y hace sonar un tono"]
    fn audio_de_una_aplicacion_llega_por_la_rama_de_audio() {
        use std::process::{Command, Stdio};
        use std::sync::atomic::AtomicUsize;

        gst::init().unwrap();
        let mut tono = Command::new("gst-launch-1.0")
            .args([
                "audiotestsrc",
                "is-live=true",
                "volume=0.05",
                "!",
                "audioconvert",
                "!",
                "pulsesink",
                "stream-properties=props,application.name=PruebaHypnotica",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("no se pudo lanzar gst-launch-1.0");
        std::thread::sleep(std::time::Duration::from_secs(2));

        let pipeline = gst::Pipeline::new();
        let volumen: Arc<Mutex<Option<gst::Element>>> = Arc::new(Mutex::new(None));
        let resultado = agregar_audio_de_captura(
            &pipeline,
            &AudioDeCaptura::Aplicacion("PruebaHypnotica".into()),
            &volumen,
            &Ajustes::default(),
        );
        // Cuenta los buffers que pasan por el volumen: si llegan, el sonido entró.
        let buffers = Arc::new(AtomicUsize::new(0));
        if let Some(v) = volumen.lock().unwrap().as_ref() {
            let cuenta = Arc::clone(&buffers);
            v.static_pad("src")
                .unwrap()
                .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                    cuenta.fetch_add(1, Ordering::Relaxed);
                    gst::PadProbeReturn::Ok
                });
        }
        let _ = pipeline.set_state(gst::State::Playing);
        // `SEGUNDOS_DE_PRUEBA` alarga la captura para medir cortes con `GST_DEBUG=audiobasesink:2`.
        let segundos = std::env::var("SEGUNDOS_DE_PRUEBA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        std::thread::sleep(std::time::Duration::from_secs(segundos));
        let _ = pipeline.set_state(gst::State::Null);
        let _ = tono.kill();

        resultado.expect("no se pudo armar la rama de audio");
        let n = buffers.load(Ordering::Relaxed);
        assert!(n > 10, "llegaron pocos buffers de audio: {n}");
    }

    #[test]
    fn pedir_audio_de_una_aplicacion_que_no_suena_da_un_error_claro() {
        gst::init().unwrap();
        let pipeline = gst::Pipeline::new();
        let volumen = Arc::new(Mutex::new(None));
        let e = agregar_audio_de_captura(
            &pipeline,
            &AudioDeCaptura::Aplicacion("NoExisteEstaApp".into()),
            &volumen,
            &Ajustes::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("NoExisteEstaApp"), "{e}");
    }

    /// Rutas de las imágenes de prueba (relativas a este crate).
    fn imagen_de_prueba(nombre: &str) -> std::path::PathBuf {
        let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        match nombre {
            "semitransparente.png" => raiz.join("pruebas").join(nombre),
            otra => raiz.join("../../assets").join(otra),
        }
    }

    /// Una imagen con el bucle apagado se decodifica **una vez** y queda quieta. Con el bucle
    /// (como un video) volvía a decodificarse unas 190 veces por segundo para mostrar algo fijo.
    /// Se corre a mano: `cargo test -p vm_video imagen -- --ignored`
    #[test]
    #[ignore = "necesita GStreamer"]
    fn una_imagen_sin_bucle_entrega_un_solo_cuadro() {
        let ajustes = Ajustes {
            bucle: false,
            ..Ajustes::default()
        };
        let video = Video::abrir_con(imagen_de_prueba("prueba.png"), ajustes, None).unwrap();
        let (mut cuadros, mut tamano) = (0, (0, 0));
        let inicio = std::time::Instant::now();
        while inicio.elapsed() < std::time::Duration::from_millis(1500) {
            if let Some(f) = video.ultimo_frame() {
                cuadros += 1;
                tamano = (f.ancho, f.alto);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(cuadros, 1, "debía llegar un solo cuadro");
        assert_eq!(tamano, (950, 712));
        assert!(video.error().is_none(), "{:?}", video.error());
    }

    #[test]
    #[ignore = "necesita GStreamer"]
    fn la_miniatura_de_una_imagen_conserva_la_proporcion() {
        let m = miniatura(imagen_de_prueba("prueba.png"), 160).unwrap();
        assert_eq!(m.ancho, 160);
        // 712 · 160 / 950 = 119,9: la proporción se conserva (con un píxel de margen).
        assert!((m.alto as i32 - 120).abs() <= 1, "alto {}", m.alto);
    }

    /// El alfa de un PNG llega al cuadro: si no, una imagen con transparencia taparía lo que
    /// hay debajo.
    #[test]
    #[ignore = "necesita GStreamer"]
    fn una_imagen_con_transparencia_conserva_el_alfa() {
        let ajustes = Ajustes {
            bucle: false,
            ..Ajustes::default()
        };
        let video =
            Video::abrir_con(imagen_de_prueba("semitransparente.png"), ajustes, None).unwrap();
        let inicio = std::time::Instant::now();
        let frame = loop {
            if let Some(f) = video.ultimo_frame() {
                break f;
            }
            assert!(
                inicio.elapsed() < std::time::Duration::from_secs(5),
                "no llegó ningún cuadro"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (rojo, alfa) = (frame.datos[0], frame.datos[3]);
        assert!(rojo > 200, "el color se perdió: rojo = {rojo}");
        assert!(
            (120..=136).contains(&alfa),
            "el alfa debía ser ~128 y es {alfa}"
        );
    }

    /// El video con un tono de 330 Hz (3 s) de la carpeta `pruebas/`.
    fn video_con_tono() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("pruebas/tono_330hz.mp4")
    }

    /// Junta las muestras de un video durante `segundos`.
    fn juntar_muestras(video: &Video, segundos: f32) -> Vec<f32> {
        let inicio = std::time::Instant::now();
        let mut todas = Vec::new();
        while inicio.elapsed().as_secs_f32() < segundos {
            todas.extend(video.tomar_muestras_de_audio());
            // Se pide también el cuadro, como hace el programa, para que el video avance.
            let _ = video.ultimo_frame();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        todas.extend(video.tomar_muestras_de_audio());
        todas
    }

    /// Las muestras que salen de un clip tienen la frecuencia del tono y llegan **aunque el
    /// clip esté silenciado** (la derivación sale antes del volumen). Se estima la frecuencia
    /// contando cuántas veces la onda cruza el cero. Se corre a mano:
    /// `cargo test -p vm_video derivacion -- --ignored`
    #[test]
    #[ignore = "necesita GStreamer"]
    fn derivacion_del_audio_de_un_clip_silenciado_da_la_frecuencia_del_tono() {
        let ajustes = Ajustes {
            bucle: false,
            silenciado: true,
            ..Ajustes::default()
        };
        let video = Video::abrir_con(video_con_tono(), ajustes, None).unwrap();
        let muestras = juntar_muestras(&video, 2.5);
        assert!(
            muestras.len() > 48_000,
            "llegó poco audio: {} muestras",
            muestras.len()
        );
        // Frecuencia por cruces de cero, ignorando el arranque (los primeros 0,2 s).
        let util = &muestras[9_600..];
        let cruces = util
            .windows(2)
            .filter(|p| (p[0] < 0.0) != (p[1] < 0.0))
            .count();
        let segundos = util.len() as f32 / 48_000.0;
        let frecuencia = cruces as f32 / 2.0 / segundos;
        assert!(
            (frecuencia - 330.0).abs() < 10.0,
            "la frecuencia debía ser ~330 Hz y es {frecuencia:.1}"
        );
        // Un tono de amplitud 0,5 tiene un valor eficaz de ~0,35 (el AAC pierde un poco).
        let rms = (util.iter().map(|m| m * m).sum::<f32>() / util.len() as f32).sqrt();
        assert!((0.25..0.45).contains(&rms), "valor eficaz {rms:.3}");
    }

    #[test]
    #[ignore = "necesita GStreamer"]
    fn un_video_sin_audio_no_da_muestras_y_sigue_andando() {
        // Una imagen no tiene audio.
        let ajustes = Ajustes {
            bucle: false,
            ..Ajustes::default()
        };
        let video = Video::abrir_con(imagen_de_prueba("prueba.png"), ajustes, None).unwrap();
        let muestras = juntar_muestras(&video, 1.0);
        assert!(
            muestras.is_empty(),
            "no debía haber audio: {}",
            muestras.len()
        );
        assert!(video.error().is_none(), "{:?}", video.error());
    }

    /// La derivación no debe romper la reproducción normal de un video con audio.
    #[test]
    #[ignore = "necesita GStreamer"]
    fn un_video_con_audio_sigue_entregando_cuadros_y_sin_errores() {
        let video = Video::abrir_con(video_con_tono(), Ajustes::default(), None).unwrap();
        let inicio = std::time::Instant::now();
        let mut cuadros = 0;
        while inicio.elapsed() < std::time::Duration::from_millis(2500) {
            if video.ultimo_frame().is_some() {
                cuadros += 1;
            }
            let _ = video.tomar_muestras_de_audio();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // 30 cuadros por segundo durante ~2,5 s.
        assert!(cuadros > 40, "llegaron pocos cuadros: {cuadros}");
        assert!(video.error().is_none(), "{:?}", video.error());
    }
}
