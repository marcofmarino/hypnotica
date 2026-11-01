//! Entrada de audio para que la imagen **reaccione al sonido** (ver `docs/05`).
//!
//! Cada parámetro modulado (`vm_core::Modulacion`) dice de dónde sale su sonido: el escritorio,
//! el micrófono, una aplicación o el clip de una capa. Este plugin abre **una captura por cada
//! fuente distinta en uso** (y cierra las que dejan de usarse), la lee con una tubería de
//! GStreamer **sin reproducirla**, y `vm_core::AnalizadorDeAudio` la convierte en cuatro números
//! de 0 a 1 (`Senales`). Como no hay salida de sonido, no hay nada que pueda cortarse ni hacer eco.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use vm_composicion::{CapaComposicion, SenalesDeEntrada};
use vm_core::{AnalizadorDeAudio, Senales};
use vm_proyecto::ProyectoActual;
use vm_video::Reproductor;
use vm_video::audio::{AplicacionDeAudio, listar_aplicaciones_de_audio, serial_de_aplicacion};

pub use vm_core::FuenteDeAudio;

/// Muestras por segundo a las que se analiza: no hace falta más para tres bandas.
const TASA: u32 = 48_000;

/// `HYPNOTICA_DIAGNOSTICO_AUDIO=1` registra cada segundo las señales que se están oyendo (para
/// comprobar la entrada sin mirar la interfaz).
fn diagnostico_activo() -> bool {
    use std::sync::OnceLock;
    static VALOR: OnceLock<bool> = OnceLock::new();
    *VALOR.get_or_init(|| std::env::var_os("HYPNOTICA_DIAGNOSTICO_AUDIO").is_some())
}

/// Si no llegó sonido en este tiempo, las señales empiezan a caer a 0.
const SIN_DATOS_DESPUES_DE: Duration = Duration::from_millis(300);

/// Cada cuánto se reintenta abrir una fuente que falló (por ejemplo, una aplicación que
/// todavía no está sonando).
const REINTENTO: Duration = Duration::from_secs(2);

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SenalesDeAudio>()
            .init_resource::<AplicacionesQueSuenan>()
            .add_systems(Update, (escuchar_las_fuentes, recibir_aplicaciones));
    }
}

/// Lo que se oye de una fuente.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EstadoDeFuente {
    pub senales: Senales,
    /// ¿Hay una captura abierta y llegando sonido?
    pub activa: bool,
    /// Por qué no se pudo abrir o se cortó.
    pub error: Option<String>,
}

/// Lo que se está oyendo ahora, por fuente. Solo tiene las fuentes que alguna modulación usa.
/// La interfaz lo lee para mostrar el medidor y los errores de cada modulación.
#[derive(Resource, Default)]
pub struct SenalesDeAudio {
    estados: HashMap<FuenteDeAudio, EstadoDeFuente>,
}

impl SenalesDeAudio {
    /// El estado de una fuente, si se la está escuchando.
    pub fn de(&self, fuente: &FuenteDeAudio) -> Option<&EstadoDeFuente> {
        self.estados.get(fuente)
    }
}

// ---------------------------------------------------------------------------------------
// La captura
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct EstadoDeCaptura {
    senales: Senales,
    /// Cuándo llegó el último sonido.
    ultimo_dato: Option<Instant>,
    error: Option<String>,
}

/// Una tubería abierta. Al soltarla se apaga (en otro hilo, para no trabar).
struct CapturaDeAudio {
    pipeline: gst::Pipeline,
    estado: Arc<Mutex<EstadoDeCaptura>>,
    terminar: Arc<AtomicBool>,
    hilo_bus: Option<JoinHandle<()>>,
}

impl CapturaDeAudio {
    /// Arma `pipewiresrc ! audioconvert ! audioresample ! (mono, 48 kHz) ! appsink`. No hay
    /// ninguna salida de sonido: lo que llega solo se analiza.
    fn abrir(fuente: &FuenteDeAudio) -> Result<CapturaDeAudio, String> {
        gst::init().map_err(|e| e.to_string())?;
        let fuente_de_pipewire = gst::ElementFactory::make("pipewiresrc")
            .property("do-timestamp", true)
            .build()
            .map_err(|e| e.to_string())?;
        match fuente {
            FuenteDeAudio::Ninguna | FuenteDeAudio::Capa(_) => {
                return Err("esta fuente no usa una captura de PipeWire".into());
            }
            // La entrada por defecto del sistema: el micrófono.
            FuenteDeAudio::Microfono => {}
            // Sin destino y pidiendo «capturar la salida»: el monitor de la salida de sonido
            // por defecto.
            FuenteDeAudio::Escritorio => fuente_de_pipewire.set_property(
                "stream-properties",
                gst::Structure::builder("props")
                    .field("stream.capture.sink", true)
                    .build(),
            ),
            // Solo el flujo de esa aplicación.
            FuenteDeAudio::Aplicacion(nombre) => {
                let serial = serial_de_aplicacion(nombre)
                    .ok_or_else(|| format!("«{nombre}» no está sonando"))?;
                fuente_de_pipewire.set_property("target-object", serial.to_string());
            }
        }

        let convertir = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|e| e.to_string())?;
        let remuestrear = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|e| e.to_string())?;
        let formato = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("channels", 1i32)
                    .field("rate", TASA as i32)
                    .field("layout", "interleaved")
                    .build(),
            )
            .build()
            .map_err(|e| e.to_string())?;
        let appsink = gst_app::AppSink::builder().sync(false).build();

        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many([
                &fuente_de_pipewire,
                &convertir,
                &remuestrear,
                &formato,
                appsink.upcast_ref(),
            ])
            .map_err(|e| e.to_string())?;
        gst::Element::link_many([
            &fuente_de_pipewire,
            &convertir,
            &remuestrear,
            &formato,
            appsink.upcast_ref(),
        ])
        .map_err(|e| e.to_string())?;

        // El análisis se hace en el hilo de GStreamer; el resultado queda en `estado`.
        let estado = Arc::new(Mutex::new(EstadoDeCaptura::default()));
        let destino = Arc::clone(&estado);
        let mut analizador = AnalizadorDeAudio::nuevo(TASA as f32);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = muestra.buffer().ok_or(gst::FlowError::Error)?;
                    let mapa = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let muestras: Vec<f32> = mapa
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    let senales = analizador.entrar(&muestras);
                    if let Ok(mut e) = destino.lock() {
                        e.senales = senales;
                        e.ultimo_dato = Some(Instant::now());
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        if pipeline.set_state(gst::State::Playing).is_err() {
            let motivo = pipeline
                .bus()
                .and_then(|bus| {
                    bus.timed_pop_filtered(
                        gst::ClockTime::from_mseconds(500),
                        &[gst::MessageType::Error],
                    )
                })
                .map(|m| match m.view() {
                    gst::MessageView::Error(e) => e.error().to_string(),
                    _ => "?".into(),
                })
                .unwrap_or_else(|| "no se pudo arrancar".into());
            let _ = pipeline.set_state(gst::State::Null);
            return Err(motivo);
        }

        let terminar = Arc::new(AtomicBool::new(false));
        let hilo_bus = vigilar_el_bus(&pipeline, &estado, &terminar);
        Ok(CapturaDeAudio {
            pipeline,
            estado,
            terminar,
            hilo_bus,
        })
    }

    /// Las señales de ahora: si hace rato que no llega sonido, caen hacia 0. Devuelve también
    /// el error del bus, si lo hubo.
    fn leer(&self) -> (Senales, Option<String>) {
        let Ok(e) = self.estado.lock() else {
            return (Senales::default(), None);
        };
        let senales = match e.ultimo_dato {
            Some(t) if t.elapsed() > SIN_DATOS_DESPUES_DE => e
                .senales
                .sin_sonido((t.elapsed() - SIN_DATOS_DESPUES_DE).as_secs_f32()),
            Some(_) => e.senales,
            None => Senales::default(),
        };
        (senales, e.error.clone())
    }
}

impl Drop for CapturaDeAudio {
    fn drop(&mut self) {
        self.terminar.store(true, Ordering::Relaxed);
        let pipeline = self.pipeline.clone();
        let hilo = self.hilo_bus.take();
        std::thread::spawn(move || {
            let _ = pipeline.set_state(gst::State::Null);
            if let Some(h) = hilo {
                let _ = h.join();
            }
        });
    }
}

/// Un hilo que lee el bus de GStreamer: un error o el fin de la transmisión (la aplicación
/// que se estaba escuchando dejó de sonar o se cerró) quedan anotados.
fn vigilar_el_bus(
    pipeline: &gst::Pipeline,
    estado: &Arc<Mutex<EstadoDeCaptura>>,
    terminar: &Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let bus = pipeline.bus()?;
    let estado = Arc::clone(estado);
    let terminar = Arc::clone(terminar);
    Some(std::thread::spawn(move || {
        while !terminar.load(Ordering::Relaxed) {
            let Some(mensaje) = bus.timed_pop_filtered(
                gst::ClockTime::from_mseconds(100),
                &[gst::MessageType::Error, gst::MessageType::Eos],
            ) else {
                continue;
            };
            let texto = match mensaje.view() {
                gst::MessageView::Error(e) => e.error().to_string(),
                gst::MessageView::Eos(_) => "la fuente dejó de emitir".to_string(),
                _ => continue,
            };
            if let Ok(mut e) = estado.lock() {
                e.error = Some(texto);
            }
            return;
        }
    }))
}

// ---------------------------------------------------------------------------------------
// Sistemas de Bevy
// ---------------------------------------------------------------------------------------

/// Una fuente que se está escuchando: su captura (si es externa) o su análisis (si es un clip).
#[derive(Default)]
struct Escucha {
    captura: Option<CapturaDeAudio>,
    /// Cuándo se puede volver a intentar abrir la captura. `None`: probar ya.
    proximo_intento: Option<Instant>,
    /// Solo para el sonido de un clip.
    analizador: Option<AnalizadorDeAudio>,
    ultimo_dato: Option<Instant>,
    estado: EstadoDeFuente,
}

/// Una fuente externa (escritorio, micrófono, aplicación): abre su captura cuando toca y lee
/// lo que llega.
fn actualizar_captura(fuente: &FuenteDeAudio, e: &mut Escucha) {
    if e.captura.is_none() && e.proximo_intento.is_none_or(|t| Instant::now() >= t) {
        match CapturaDeAudio::abrir(fuente) {
            Ok(captura) => {
                info!("entrada de audio abierta: {}", fuente.nombre());
                e.captura = Some(captura);
                e.estado.error = None;
            }
            Err(motivo) => {
                warn!(
                    "no se pudo abrir la entrada de audio ({}): {motivo}",
                    fuente.nombre()
                );
                e.estado.error = Some(motivo);
                e.proximo_intento = Some(Instant::now() + REINTENTO);
            }
        }
    }
    let Some(captura) = e.captura.as_ref() else {
        // Esperando para reintentar: lo último que se oyó se apaga de a poco.
        e.estado.activa = false;
        if e.estado.senales != Senales::default() {
            e.estado.senales = e.estado.senales.sin_sonido(0.05);
        }
        return;
    };
    let (senales, error) = captura.leer();
    if let Some(motivo) = error {
        // Se cortó: se cierra y se vuelve a intentar más tarde (la aplicación puede volver).
        warn!(
            "la entrada de audio se cortó ({}): {motivo}",
            fuente.nombre()
        );
        e.captura = None;
        e.proximo_intento = Some(Instant::now() + REINTENTO);
        e.estado.error = Some(motivo);
        e.estado.activa = false;
        return;
    }
    e.estado.senales = senales;
    e.estado.activa = true;
}

/// El sonido del clip que suena en una capa (`FuenteDeAudio::Capa`): saca las muestras de
/// adentro de su reproductor y las pasa por el mismo análisis que el resto de las fuentes.
fn actualizar_clip(
    indice: usize,
    e: &mut Escucha,
    capas: &Query<(&CapaComposicion, &Reproductor)>,
) {
    e.estado.error = None;
    let analizador = e
        .analizador
        .get_or_insert_with(|| AnalizadorDeAudio::nuevo(TASA as f32));
    let muestras = capas
        .iter()
        .find(|(capa, _)| capa.indice == indice)
        .map(|(_, reproductor)| reproductor.tomar_muestras_de_audio())
        .unwrap_or_default();
    if !muestras.is_empty() {
        e.estado.senales = analizador.entrar(&muestras);
        e.estado.activa = true;
        e.ultimo_dato = Some(Instant::now());
        return;
    }
    // Sin sonido (el clip no suena, no tiene audio o terminó): las señales caen hacia 0.
    let desde = *e.ultimo_dato.get_or_insert_with(Instant::now);
    if desde.elapsed() > SIN_DATOS_DESPUES_DE {
        e.estado.senales = analizador
            .senales()
            .sin_sonido((desde.elapsed() - SIN_DATOS_DESPUES_DE).as_secs_f32());
        e.estado.activa = false;
    }
}

/// Mantiene abierta una escucha por cada fuente que alguna modulación usa, cierra las que ya
/// nadie usa y publica lo que se oye: en el bus (que mueve los parámetros) y en
/// `SenalesDeAudio` (para la interfaz).
fn escuchar_las_fuentes(
    proyecto: Res<ProyectoActual>,
    capas: Query<(&CapaComposicion, &Reproductor)>,
    mut escuchas: Local<HashMap<FuenteDeAudio, Escucha>>,
    mut publicadas: ResMut<SenalesDeAudio>,
    mut bus: ResMut<SenalesDeEntrada>,
    mut ultimo_registro: Local<Option<Instant>>,
) {
    let en_uso = proyecto.0.composicion.fuentes_de_audio();
    // Las que ya nadie usa se sueltan (una captura se apaga sola al soltarla).
    escuchas.retain(|f, _| en_uso.contains(f));

    for fuente in &en_uso {
        let e = escuchas.entry(fuente.clone()).or_default();
        match fuente {
            FuenteDeAudio::Ninguna => {}
            FuenteDeAudio::Capa(i) => actualizar_clip(*i, e, &capas),
            _ => actualizar_captura(fuente, e),
        }
        // Se compara antes de escribir: sin cambios no se marca nada.
        if bus.de(fuente) != e.estado.senales {
            bus.poner(fuente.clone(), e.estado.senales);
        }
    }
    if bus.tiene_otras_que(&en_uso) {
        bus.conservar(&en_uso);
    }

    let hay_cambio = publicadas.estados.len() != escuchas.len()
        || escuchas
            .iter()
            .any(|(f, e)| publicadas.estados.get(f) != Some(&e.estado));
    if hay_cambio {
        publicadas.estados = escuchas
            .iter()
            .map(|(f, e)| (f.clone(), e.estado.clone()))
            .collect();
    }

    if diagnostico_activo() && ultimo_registro.is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
    {
        *ultimo_registro = Some(Instant::now());
        for (f, e) in escuchas.iter() {
            let s = e.estado.senales;
            info!(
                "audio {}: nivel {:.2} · graves {:.2} · medios {:.2} · agudos {:.2}",
                f.nombre(),
                s.nivel,
                s.graves,
                s.medios,
                s.agudos
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Las aplicaciones que suenan
// ---------------------------------------------------------------------------------------

/// Las aplicaciones que están emitiendo sonido ahora. La búsqueda (`pw-dump`) puede tardar,
/// así que va en un hilo aparte y el resultado se recoge frame a frame.
#[derive(Resource, Default)]
pub struct AplicacionesQueSuenan {
    pub lista: Vec<AplicacionDeAudio>,
    buscando: Option<Mutex<Receiver<Vec<AplicacionDeAudio>>>>,
    /// Ya se buscó alguna vez: así la lista se llena sola la primera vez que se necesita.
    pub buscadas: bool,
}

impl AplicacionesQueSuenan {
    /// Vuelve a buscar (si ya hay una búsqueda en curso, no hace nada).
    pub fn actualizar(&mut self) {
        if self.buscando.is_some() {
            return;
        }
        self.buscadas = true;
        let (emisor, receptor) = channel();
        std::thread::spawn(move || {
            let _ = emisor.send(listar_aplicaciones_de_audio());
        });
        self.buscando = Some(Mutex::new(receptor));
    }
}

fn recibir_aplicaciones(mut aplicaciones: ResMut<AplicacionesQueSuenan>) {
    // Se mira primero sin pedir escritura: no marcar el recurso como modificado en cada frame.
    if aplicaciones.buscando.is_none() {
        return;
    }
    let respuesta = match aplicaciones.buscando.as_ref().map(|r| r.lock()) {
        Some(Ok(receptor)) => receptor.try_recv(),
        _ => Err(TryRecvError::Disconnected),
    };
    match respuesta {
        Ok(lista) => {
            aplicaciones.lista = lista;
            aplicaciones.buscando = None;
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => aplicaciones.buscando = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prueba con PipeWire de verdad: una «aplicación» emite un tono de 330 Hz y se escucha
    /// solo a ella. El tono cae en los medios. Hace sonar un tono un instante:
    /// `cargo test -p vm_audio tono -- --ignored`
    #[test]
    #[ignore = "necesita PipeWire y hace sonar un tono"]
    fn un_tono_de_una_aplicacion_enciende_los_medios() {
        use std::process::{Command, Stdio};
        gst::init().unwrap();
        let mut tono = Command::new("gst-launch-1.0")
            .args([
                "audiotestsrc",
                "is-live=true",
                "freq=330",
                "volume=0.3",
                "!",
                "audioconvert",
                "!",
                "pulsesink",
                "stream-properties=props,application.name=PruebaAudioReactivo",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("no se pudo lanzar gst-launch-1.0");
        std::thread::sleep(Duration::from_secs(2));

        let resultado =
            CapturaDeAudio::abrir(&FuenteDeAudio::Aplicacion("PruebaAudioReactivo".into()));
        let captura = resultado.expect("no se pudo abrir la captura");
        std::thread::sleep(Duration::from_secs(2));
        let (senales, error) = captura.leer();
        drop(captura);
        let _ = tono.kill();

        assert!(error.is_none(), "{error:?}");
        assert!(
            senales.medios > 0.5,
            "los medios debían estar altos: {senales:?}"
        );
        assert!(senales.graves < 0.45, "los graves no: {senales:?}");
        assert!(senales.agudos < 0.45, "los agudos no: {senales:?}");
    }

    /// Igual, pero escuchando todo el escritorio (el monitor de la salida por defecto): el tono
    /// tiene que notarse en los medios aunque haya otros sonidos.
    #[test]
    #[ignore = "necesita PipeWire y hace sonar un tono"]
    fn un_tono_en_el_escritorio_tambien_se_oye() {
        use std::process::{Command, Stdio};
        gst::init().unwrap();
        let mut tono = Command::new("gst-launch-1.0")
            .args([
                "audiotestsrc",
                "is-live=true",
                "freq=330",
                "volume=0.3",
                "!",
                "audioconvert",
                "!",
                "pulsesink",
                "stream-properties=props,application.name=PruebaEscritorio",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("no se pudo lanzar gst-launch-1.0");
        std::thread::sleep(Duration::from_secs(2));
        let captura =
            CapturaDeAudio::abrir(&FuenteDeAudio::Escritorio).expect("no se pudo abrir la captura");
        std::thread::sleep(Duration::from_secs(2));
        let (senales, error) = captura.leer();
        drop(captura);
        let _ = tono.kill();
        assert!(error.is_none(), "{error:?}");
        assert!(
            senales.medios > 0.5,
            "los medios debían notarse: {senales:?}"
        );
    }

    #[test]
    fn pedir_una_aplicacion_que_no_suena_da_un_error_claro() {
        let e = CapturaDeAudio::abrir(&FuenteDeAudio::Aplicacion("NoExisteEstaApp".into()))
            .err()
            .expect("debía fallar");
        assert!(e.contains("NoExisteEstaApp"), "{e}");
    }
}
