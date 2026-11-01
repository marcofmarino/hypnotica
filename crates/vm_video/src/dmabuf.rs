//! Captura de pantalla o ventana **por DMA-BUF**: el compositor entrega los cuadros como
//! buffers de GPU y se importan a una textura sin pasar por la CPU (ver
//! `docs/06_dmabuf_captura.md` y el ejemplo `prueba_dmabuf`, que lo probó aislado).
//!
//! Este módulo solo se ocupa de **recibir** los cuadros con GStreamer. Importarlos a la GPU
//! lo hace `importar.rs`, en el mundo de render de Bevy.
//!
//! Los dos lados se hablan por una [`Ranura`]: el hilo de GStreamer deja ahí el cuadro más
//! nuevo (si todavía había uno sin usar, se descarta: mostrar el último evita atrasarse) y el
//! mundo de render lo toma.

use std::os::fd::{BorrowedFd, IntoRawFd, OwnedFd};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators as gst_alloc;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::captura::SesionDeCaptura;

/// Cómo se le pide el formato a PipeWire. Es la única forma que negoció en las pruebas:
/// `format=DMA_DRM` (con o sin `drm-format`) falla con «no hay formatos en común».
const CAPS: &str = "video/x-raw(memory:DMABuf),format=BGRx";

/// Lo que hace falta saber de un cuadro para importarlo.
#[derive(Clone, Debug, PartialEq)]
pub struct DatosDeCuadro {
    pub ancho: u32,
    pub alto: u32,
    /// El formato de DRM como texto: `XR24` (BGRx en memoria), `AR24`, `XB24`, `AB24`.
    pub fourcc: String,
    /// El modificador de DRM. Las caps de PipeWire no siempre lo traen: sin él se supone
    /// lineal (0), que fue lo que llegó en las pruebas.
    pub modificador: u64,
    pub stride: u64,
    pub offset: u64,
}

/// Un cuadro recibido como DMA-BUF.
pub struct CuadroDmaBuf {
    /// Un duplicado del descriptor del buffer: la importación a Vulkan se queda con él.
    pub fd: OwnedFd,
    pub datos: DatosDeCuadro,
    /// El buffer de PipeWire. **Hay que mantenerlo vivo hasta que la GPU termine de leerlo**:
    /// si se suelta antes, PipeWire se lo devuelve al productor, que lo puede volver a pisar.
    pub muestra: gst::Sample,
}

#[derive(Default)]
struct Estado {
    ultimo: Option<CuadroDmaBuf>,
    /// Datos del último cuadro visto, aunque ya se haya tomado.
    visto: Option<DatosDeCuadro>,
    recibidos: u64,
    /// Un cuadro que no se pudo usar (no era DMA-BUF, formato ilegible...).
    rechazo: Option<String>,
    /// Cuántos cuadros se rechazaron en total: uno suelto no alcanza para dar la captura por
    /// perdida (ver `Ranura::sin_salida`).
    rechazados: u64,
    /// Un fallo de la importación en la GPU, informado por el mundo de render.
    fallo: Option<String>,
}

/// El lugar compartido entre el hilo de GStreamer y el mundo de render.
#[derive(Default)]
pub struct Ranura(Mutex<Estado>);

impl Ranura {
    /// Toma el cuadro más nuevo, si hay uno sin usar.
    pub fn tomar(&self) -> Option<CuadroDmaBuf> {
        self.0.lock().ok()?.ultimo.take()
    }

    /// Los datos del último cuadro que llegó (sin quitarlo): sirven para crear la textura
    /// destino con el tamaño y el formato correctos.
    pub fn visto(&self) -> Option<DatosDeCuadro> {
        self.0.lock().ok()?.visto.clone()
    }

    pub fn recibidos(&self) -> u64 {
        self.0.lock().map_or(0, |e| e.recibidos)
    }

    /// El mundo de render avisa que no pudo importar un cuadro.
    pub fn informar_fallo(&self, motivo: String) {
        if let Ok(mut e) = self.0.lock() {
            e.fallo.get_or_insert(motivo);
        }
    }

    pub fn fallo(&self) -> Option<String> {
        self.0.lock().ok()?.fallo.clone()
    }

    /// Si la captura no tiene salida por DMA-BUF: **nunca** llegó un cuadro bueno y, o se
    /// rechazaron muchos seguidos, o pasó un rato desde que se abrió y se rechazó alguno. Un
    /// solo cuadro raro al principio (pasa mientras se negocia el formato) no cuenta.
    pub fn sin_salida(&self, abierta_hace: std::time::Duration) -> Option<String> {
        let e = self.0.lock().ok()?;
        let sin_cuadros_buenos = e.recibidos == 0;
        let muchos = e.rechazados >= 10;
        let pasado_un_rato = abierta_hace >= std::time::Duration::from_secs(2) && e.rechazados > 0;
        (sin_cuadros_buenos && (muchos || pasado_un_rato))
            .then(|| e.rechazo.clone())
            .flatten()
    }

    /// Por qué se rechazó el último cuadro que no era utilizable.
    pub fn rechazo(&self) -> Option<String> {
        self.0.lock().ok()?.rechazo.clone()
    }

    fn poner(&self, cuadro: CuadroDmaBuf) {
        if let Ok(mut e) = self.0.lock() {
            e.recibidos += 1;
            e.visto = Some(cuadro.datos.clone());
            e.ultimo = Some(cuadro);
        }
    }

    fn rechazar(&self, motivo: String) {
        if let Ok(mut e) = self.0.lock() {
            e.rechazados += 1;
            e.rechazo = Some(motivo);
        }
    }
}

/// Una captura abierta por DMA-BUF. Mientras exista, los cuadros siguen llegando a `ranura`.
pub struct CapturaDmaBuf {
    pipeline: gst::Pipeline,
    pub ranura: Arc<Ranura>,
    /// Vigila el bus de GStreamer mientras dura la captura (ver `vigilar_el_bus`).
    terminar: Arc<AtomicBool>,
    hilo_bus: Option<JoinHandle<()>>,
    abierta: std::time::Instant,
    /// La sesión del portal tiene que seguir viva mientras dure la captura. Va al final: se
    /// suelta después de apagar la tubería.
    sesion: Option<SesionDeCaptura>,
}

impl CapturaDmaBuf {
    /// Arma `pipewiresrc ! capsfilter ! appsink` y empieza a recibir. Devuelve un error (con
    /// el motivo del bus de GStreamer) si PipeWire no puede entregar DMA-BUF: quien llama
    /// puede volver al camino por CPU.
    pub fn abrir(sesion: SesionDeCaptura) -> Result<CapturaDmaBuf, String> {
        gst::init().map_err(|e| e.to_string())?;
        let hacer = |e: Result<gst::Element, gst::glib::BoolError>| e.map_err(|e| e.to_string());

        // `pipewiresrc` se queda con el descriptor que recibe y lo cierra al terminar: se le
        // da un duplicado, y el de la sesión sigue siendo de ella.
        let fd_propio = unsafe { BorrowedFd::borrow_raw(sesion.fd()) }
            .try_clone_to_owned()
            .map_err(|e| format!("no se pudo duplicar el fd de la sesión: {e}"))?;
        let fuente = hacer(
            gst::ElementFactory::make("pipewiresrc")
                .property("fd", fd_propio.into_raw_fd())
                .property("path", sesion.nodo.to_string())
                .property("do-timestamp", true)
                .build(),
        )?;
        let filtro = hacer(
            gst::ElementFactory::make("capsfilter")
                .property(
                    "caps",
                    gst::Caps::from_str(CAPS).map_err(|e| e.to_string())?,
                )
                .build(),
        )?;
        // Una imagen en vivo se entrega apenas llega, igual que en `decodificador.rs`.
        let appsink = gst_app::AppSink::builder()
            .sync(false)
            .max_buffers(2)
            .drop(true)
            .build();
        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many([&fuente, &filtro, appsink.upcast_ref()])
            .map_err(|e| e.to_string())?;
        gst::Element::link_many([&fuente, &filtro, appsink.upcast_ref()])
            .map_err(|e| e.to_string())?;

        let ranura = Arc::new(Ranura::default());
        let destino = Arc::clone(&ranura);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    match cuadro_de_muestra(&muestra) {
                        Ok(cuadro) => destino.poner(cuadro),
                        Err(motivo) => destino.rechazar(motivo),
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        if pipeline.set_state(gst::State::Playing).is_err() {
            // El bus guarda el motivo; el `Err` solo dice «falló el cambio de estado».
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
                .unwrap_or_else(|| "sin mensaje en el bus".into());
            let _ = pipeline.set_state(gst::State::Null);
            return Err(motivo);
        }
        let terminar = Arc::new(AtomicBool::new(false));
        let hilo_bus = vigilar_el_bus(&pipeline, &ranura, &terminar);
        Ok(CapturaDmaBuf {
            pipeline,
            ranura,
            terminar,
            hilo_bus,
            abierta: std::time::Instant::now(),
            sesion: Some(sesion),
        })
    }

    /// Cuánto hace que se abrió la captura.
    pub fn abierta_hace(&self) -> std::time::Duration {
        self.abierta.elapsed()
    }
}

/// Un hilo que lee el bus de GStreamer **después** de arrancar. Sin esto, un error posterior
/// (PipeWire que no logra renegociar el formato cuando la ventana cambia de tamaño, el fin de
/// la transmisión...) pasaba desapercibido: la imagen quedaba congelada. Ahora se avisa por la
/// ranura y el `Reproductor` vuelve al camino por CPU.
fn vigilar_el_bus(
    pipeline: &gst::Pipeline,
    ranura: &Arc<Ranura>,
    terminar: &Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let bus = pipeline.bus()?;
    let ranura = Arc::clone(ranura);
    let terminar = Arc::clone(terminar);
    Some(std::thread::spawn(move || {
        while !terminar.load(Ordering::Relaxed) {
            let Some(mensaje) = bus.timed_pop_filtered(
                gst::ClockTime::from_mseconds(100),
                &[gst::MessageType::Error, gst::MessageType::Eos],
            ) else {
                continue;
            };
            match mensaje.view() {
                gst::MessageView::Error(e) => {
                    ranura.informar_fallo(format!("error de GStreamer: {}", e.error()));
                    return;
                }
                gst::MessageView::Eos(_) => {
                    ranura.informar_fallo("la transmisión de la captura terminó".into());
                    return;
                }
                _ => {}
            }
        }
    }))
}

impl Drop for CapturaDmaBuf {
    fn drop(&mut self) {
        // Apagar la tubería puede esperar un rato: se hace en otro hilo para no trabar a quien
        // cierra la captura (igual que con `Video`).
        let pipeline = self.pipeline.clone();
        let sesion = self.sesion.take();
        self.terminar.store(true, Ordering::Relaxed);
        let hilo_bus = self.hilo_bus.take();
        std::thread::spawn(move || {
            let _ = pipeline.set_state(gst::State::Null);
            if let Some(h) = hilo_bus {
                let _ = h.join();
            }
            drop(sesion);
        });
    }
}

/// Saca de una muestra el descriptor y los datos del buffer, o dice por qué no se puede.
fn cuadro_de_muestra(muestra: &gst::Sample) -> Result<CuadroDmaBuf, String> {
    let caps = muestra.caps().ok_or("la muestra no tiene caps")?;
    let estructura = caps.structure(0).ok_or("caps vacías")?;
    let ancho = estructura.get::<i32>("width").map_err(|e| e.to_string())? as u32;
    let alto = estructura.get::<i32>("height").map_err(|e| e.to_string())? as u32;
    // `drm-format` puede ser «XR24» o «XR24:0x0100000000000001» (fourcc : modificador).
    let (fourcc, modificador) = match estructura.get::<&str>("drm-format") {
        Ok(texto) => match texto.split_once(':') {
            Some((f, m)) => {
                let m = u64::from_str_radix(m.trim_start_matches("0x"), 16)
                    .map_err(|e| format!("modificador ilegible «{m}»: {e}"))?;
                (f.to_string(), m)
            }
            None => (texto.to_string(), 0),
        },
        Err(_) => (
            estructura.get::<&str>("format").unwrap_or("?").to_string(),
            0,
        ),
    };

    let buffer = muestra.buffer().ok_or("la muestra no tiene buffer")?;
    let dma = buffer
        .peek_memory(0)
        .downcast_memory_ref::<gst_alloc::DmaBufMemory>()
        .ok_or("el buffer no es DMA-BUF (memoria común)")?;
    let fd = unsafe { BorrowedFd::borrow_raw(dma.fd()) }
        .try_clone_to_owned()
        .map_err(|e| format!("no se pudo duplicar el fd del buffer: {e}"))?;
    let (stride, offset) = match buffer.meta::<gst_video::VideoMeta>() {
        Some(meta) => (meta.stride()[0] as u64, meta.offset()[0] as u64),
        // Sin `VideoMeta` se supone un buffer compacto de 4 bytes por píxel.
        None => (ancho as u64 * 4, 0),
    };
    Ok(CuadroDmaBuf {
        fd,
        datos: DatosDeCuadro {
            ancho,
            alto,
            fourcc,
            modificador,
            stride,
            offset,
        },
        muestra: muestra.clone(),
    })
}
