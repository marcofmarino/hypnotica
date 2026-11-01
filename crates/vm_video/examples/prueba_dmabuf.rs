//! Prueba de DMA-BUF: ¿se puede traer una captura de pantalla o ventana **sin pasar por la
//! CPU**? El compositor entrega los cuadros como buffers de GPU (DMA-BUF); acá se importan a
//! una textura de `wgpu` 30 (la versión de Bevy 0.20) y se copian dentro de la GPU.
//!
//! Qué mide: si el portal entrega DMA-BUF, con qué formato y modificador, si `wgpu` los
//! importa, cuántos cuadros por segundo llegan y cuánto CPU gasta (para compararlo con el
//! camino actual por memoria común, `MODO=cpu`).
//!
//! La primera vez el portal muestra su selector; después se reabre solo con el token que se
//! guarda en `~/.cache/hypnotica/prueba_dmabuf.token`. Movés una ventana o ponés un video
//! en lo capturado: el portal solo manda cuadros cuando algo cambia.
//!
//!   cargo run --release -p vm_video --example prueba_dmabuf
//!
//! Variables: `SIN_CURSOR=1` (no dibujar el cursor en la captura), `ELEGIR=1` (volver a elegir qué capturar), `MODO=cpu` (el camino actual, para comparar), `MODO=gpu` (solo comprobar la GPU), `MODO=sondeo` (qué entrega PipeWire y con qué huecos), `CAPS=0|1|2` (probar solo esa
//! negociación), `ADAPTADOR=texto` (elegir la GPU por nombre), `SEGUNDOS=15`,
//! `SALIDA=/ruta/prefijo` (dónde guardar las imágenes importadas).

use std::os::fd::{BorrowedFd, IntoRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators as gst_alloc;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use vm_video::captura::{SesionDeCaptura, iniciar};
use vm_video::decodificador::{Ajustes, Video};

/// Un cuadro recibido como DMA-BUF, con lo que hace falta para importarlo.
struct CuadroDma {
    fd: OwnedFd,
    ancho: u32,
    alto: u32,
    fourcc: String,
    modificador: u64,
    stride: u64,
    offset: u64,
    /// ¿El stride y el offset vienen de la `VideoMeta` del buffer (y no de una suposición)?
    con_meta: bool,
    /// Se mantiene vivo hasta terminar de usar el buffer: si no, PipeWire lo podría reciclar.
    _muestra: gst::Sample,
}

/// Lo que pasa en el hilo de GStreamer, para mostrarlo desde el principal.
#[derive(Default)]
struct Contadores {
    recibidos: u64,
    memoria_comun: u64,
    ultimo_caps: Option<String>,
    falla: Option<String>,
}

fn main() {
    gst::init().expect("no se pudo iniciar GStreamer");
    let segundos: u64 = std::env::var("SEGUNDOS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    // `MODO=gpu`: solo comprueba la GPU, sin abrir el portal.
    if std::env::var("MODO").is_ok_and(|m| m == "gpu") {
        if let Some((_, _, info)) = abrir_gpu() {
            println!("dispositivo abierto con la importación de DMA-BUF: {info}");
        }
        return;
    }
    let ruta_token = ruta_del_token();

    println!("== sesión del portal ==");
    // `ELEGIR=1` ignora el token guardado: el portal vuelve a mostrar su selector.
    let token = if std::env::var("ELEGIR").is_ok() {
        None
    } else {
        leer_token(&ruta_token)
    };
    let sesion = match iniciar(token.as_deref(), std::env::var_os("SIN_CURSOR").is_some()) {
        Ok(s) => s,
        Err(e) => return println!("el portal respondió: {e}"),
    };
    if let Some(token) = &sesion.token {
        guardar_token(&ruta_token, token);
    }
    println!("nodo {} tamaño {:?}", sesion.nodo, sesion.tamano);

    if std::env::var("MODO").is_ok_and(|m| m == "cpu") {
        return modo_cpu(sesion, segundos);
    }
    if std::env::var("MODO").is_ok_and(|m| m == "sondeo") {
        return modo_sondeo(sesion, segundos);
    }
    modo_dmabuf(sesion, segundos);
}

// ---------------------------------------------------------------------------------------
// El camino actual (memoria común), para comparar
// ---------------------------------------------------------------------------------------

fn modo_cpu(sesion: SesionDeCaptura, segundos: u64) {
    println!("\n== MODO CPU: el camino de hoy (videoconvert + videoscale + copia) ==");
    let video = match Video::abrir_captura(sesion, Ajustes::default(), Some((1920, 1080)), None) {
        Ok(v) => v,
        Err(e) => return println!("no se pudo abrir: {e}"),
    };
    let inicio = Instant::now();
    let cpu0 = cpu_segundos();
    let (mut cuadros, mut ultimo_informe) = (0u64, Instant::now());
    while inicio.elapsed() < Duration::from_secs(segundos) {
        if video.ultimo_frame().is_some() {
            cuadros += 1;
        }
        if ultimo_informe.elapsed() >= Duration::from_secs(1) {
            println!("  {cuadros} cuadros hasta ahora");
            ultimo_informe = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    resumen(cuadros, inicio.elapsed(), cpu_segundos() - cpu0);
    println!("(no incluye la copia a la GPU que hace Bevy después: el CPU real es mayor)");
}

// ---------------------------------------------------------------------------------------
// Sondeo: qué entrega PipeWire de verdad
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Sondeo {
    total: u64,
    dma: u64,
    comun: u64,
    ultimo: Option<Instant>,
    /// El hueco más largo entre dos cuadros seguidos (en este segundo y en total).
    hueco_segundo_ms: u128,
    hueco_total_ms: u128,
    huecos_de_100: u64,
    huecos_de_250: u64,
    /// Cada combinación distinta de caps (formato, tamaño, modificador) y cuántas veces llegó.
    caps: std::collections::BTreeMap<String, u64>,
}

/// Mide qué entrega PipeWire para la ventana o pantalla elegida, aceptando DMA-BUF **y**
/// memoria común. Sirve para entender por qué una aplicación anda distinta de otra: tipo de
/// memoria, formato, tamaño, cuántos cuadros por segundo y qué tan grandes son los huecos
/// entre cuadros (lo que se ve como «saltos»). Mové el mouse sobre lo capturado mientras corre.
fn modo_sondeo(sesion: SesionDeCaptura, segundos: u64) {
    use std::str::FromStr;
    println!("\n== SONDEO ({segundos} s): mové el mouse y la ventana capturada ==");
    // Primero DMA-BUF y, si no se puede, memoria común: lo que negocie PipeWire.
    let caps = std::env::var("CAPS_SONDEO").unwrap_or_else(|_| {
        "video/x-raw(memory:DMABuf),format=BGRx; video/x-raw,format=BGRx".into()
    });
    println!("caps pedidas: {caps}");
    let fd = unsafe { BorrowedFd::borrow_raw(sesion.fd()) }
        .try_clone_to_owned()
        .expect("no se pudo duplicar el fd");
    let fuente = gst::ElementFactory::make("pipewiresrc")
        .property("fd", fd.into_raw_fd())
        .property("path", sesion.nodo.to_string())
        .property("do-timestamp", true)
        .build()
        .expect("pipewiresrc");
    let filtro = gst::ElementFactory::make("capsfilter")
        .property("caps", gst::Caps::from_str(&caps).expect("caps ilegibles"))
        .build()
        .expect("capsfilter");
    let appsink = gst_app::AppSink::builder()
        .sync(false)
        .max_buffers(2)
        .drop(true)
        .build();
    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([&fuente, &filtro, appsink.upcast_ref()])
        .unwrap();
    gst::Element::link_many([&fuente, &filtro, appsink.upcast_ref()]).unwrap();

    let datos = Arc::new(Mutex::new(Sondeo::default()));
    let en_callback = Arc::clone(&datos);
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let es_dma = muestra.buffer().is_some_and(|b| {
                    b.peek_memory(0)
                        .downcast_memory_ref::<gst_alloc::DmaBufMemory>()
                        .is_some()
                });
                let caps = muestra.caps().map(|c| c.to_string()).unwrap_or_default();
                let ahora = Instant::now();
                let mut d = en_callback.lock().unwrap();
                d.total += 1;
                if es_dma {
                    d.dma += 1;
                } else {
                    d.comun += 1;
                }
                if let Some(antes) = d.ultimo {
                    let hueco = ahora.duration_since(antes).as_millis();
                    d.hueco_segundo_ms = d.hueco_segundo_ms.max(hueco);
                    d.hueco_total_ms = d.hueco_total_ms.max(hueco);
                    if hueco >= 100 {
                        d.huecos_de_100 += 1;
                    }
                    if hueco >= 250 {
                        d.huecos_de_250 += 1;
                    }
                }
                d.ultimo = Some(ahora);
                *d.caps
                    .entry(format!(
                        "{} · {caps}",
                        if es_dma { "DMA-BUF" } else { "memoria común" }
                    ))
                    .or_default() += 1;
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let bus = pipeline.bus().expect("la tubería no tiene bus");
    if pipeline.set_state(gst::State::Playing).is_err() {
        let motivo = bus
            .timed_pop_filtered(
                gst::ClockTime::from_mseconds(500),
                &[gst::MessageType::Error],
            )
            .map(|m| match m.view() {
                gst::MessageView::Error(e) => format!("{} ({:?})", e.error(), e.debug()),
                _ => "?".into(),
            })
            .unwrap_or_else(|| "sin mensaje".into());
        return println!("RESULTADO: no arrancó: {motivo}");
    }

    let inicio = Instant::now();
    let mut mensajes: Vec<String> = Vec::new();
    let (mut anterior_total, mut anterior_dma) = (0u64, 0u64);
    for segundo in 1..=segundos {
        let hasta = Duration::from_secs(segundo);
        while inicio.elapsed() < hasta {
            while let Some(m) =
                bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Warning])
            {
                if mensajes.len() < 6 {
                    mensajes.push(match m.view() {
                        gst::MessageView::Error(e) => format!("ERROR: {}", e.error()),
                        gst::MessageView::Warning(w) => format!("aviso: {}", w.error()),
                        _ => "?".into(),
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut d = datos.lock().unwrap();
        println!(
            "  segundo {segundo:>2}: {:>3} cuadros ({:>3} DMA-BUF) · hueco máximo {:>4} ms",
            d.total - anterior_total,
            d.dma - anterior_dma,
            d.hueco_segundo_ms
        );
        (anterior_total, anterior_dma) = (d.total, d.dma);
        d.hueco_segundo_ms = 0;
    }
    let _ = pipeline.set_state(gst::State::Null);

    let d = datos.lock().unwrap();
    println!("\n== RESULTADO ==");
    let s = inicio.elapsed().as_secs_f64();
    println!(
        "{} cuadros en {s:.1} s = {:.1} por segundo · DMA-BUF {} · memoria común {}",
        d.total,
        d.total as f64 / s,
        d.dma,
        d.comun
    );
    println!(
        "hueco más largo entre cuadros: {} ms · huecos de 100 ms o más: {} · de 250 ms o más: {}",
        d.hueco_total_ms, d.huecos_de_100, d.huecos_de_250
    );
    println!("formas distintas de lo que llegó:");
    for (caps, veces) in &d.caps {
        println!("  {veces:>5} × {caps}");
    }
    if mensajes.is_empty() {
        println!("sin mensajes de error ni de aviso en el bus");
    } else {
        println!("mensajes del bus:");
        for m in &mensajes {
            println!("  {m}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// DMA-BUF
// ---------------------------------------------------------------------------------------

/// Las negociaciones que se prueban, de la más estricta a la más abierta.
const NEGOCIACIONES: [&str; 4] = [
    // Lineal (sin compresión ni mosaicos): lo más fácil de importar entre GPU distintas.
    "video/x-raw(memory:DMABuf),format=DMA_DRM,drm-format=XR24:0x0",
    // Cualquier modificador.
    "video/x-raw(memory:DMABuf),format=DMA_DRM",
    // La forma de antes de los modificadores: un formato común con la memoria DMA-BUF.
    "video/x-raw(memory:DMABuf),format=BGRx",
    // Lo más abierto: solo pide que sea DMA-BUF.
    "video/x-raw(memory:DMABuf)",
];

fn modo_dmabuf(sesion: SesionDeCaptura, segundos: u64) {
    println!("\n== GPU ==");
    let Some((dispositivo, cola, info)) = abrir_gpu() else {
        return;
    };
    println!("usando: {info}");

    println!("\n== negociación con PipeWire ==");
    let solo: Option<usize> = std::env::var("CAPS").ok().and_then(|v| v.parse().ok());
    let contadores = Arc::new(Mutex::new(Contadores::default()));
    let ultimo: Arc<Mutex<Option<CuadroDma>>> = Arc::new(Mutex::new(None));
    let mut elegido = None;
    for (i, caps) in NEGOCIACIONES.iter().enumerate() {
        if solo.is_some_and(|s| s != i) {
            continue;
        }
        println!("[{i}] {caps}");
        match armar(&sesion, caps, &contadores, &ultimo) {
            Ok(pipeline) => {
                if esperar_primer_cuadro(&contadores) {
                    println!("    → llegan cuadros");
                    elegido = Some(pipeline);
                    break;
                }
                let c = contadores.lock().unwrap();
                println!(
                    "    → sin cuadros usables en 5 s\n      memoria común: {} · falla: {:?}\n      caps recibidas: {}",
                    c.memoria_comun,
                    c.falla,
                    c.ultimo_caps.as_deref().unwrap_or("(ninguna)")
                );
                drop(c);
                let _ = pipeline.set_state(gst::State::Null);
            }
            Err(e) => println!("    → no se pudo armar: {e}"),
        }
    }
    let Some(pipeline) = elegido else {
        return println!(
            "\nRESULTADO: PipeWire no entregó DMA-BUF con ninguna negociación. Hace falta \
             mover algo en lo capturado (el portal solo manda cuadros al haber cambios)."
        );
    };

    println!("\n== importación a wgpu ({segundos} s) ==");
    let salida = std::env::var("SALIDA").unwrap_or_else(|_| "/tmp/prueba_dmabuf".into());
    let inicio = Instant::now();
    let cpu0 = cpu_segundos();
    let mut estado = Importador::nuevo(&dispositivo);
    let (mut importados, mut errores) = (0u64, 0u64);
    let mut primer_error: Option<String> = None;
    let mut ultimo_informe = Instant::now();
    let mut guardadas = 0;
    let mut visto_caps = false;

    while inicio.elapsed() < Duration::from_secs(segundos) {
        let cuadro = ultimo.lock().unwrap().take();
        let Some(cuadro) = cuadro else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        if !visto_caps {
            visto_caps = true;
            println!(
                "primer cuadro: {}x{} fourcc {} modificador {:#x} stride {} offset {} (de VideoMeta: {})",
                cuadro.ancho,
                cuadro.alto,
                cuadro.fourcc,
                cuadro.modificador,
                cuadro.stride,
                cuadro.offset,
                if cuadro.con_meta {
                    "sí"
                } else {
                    "NO, supuesto"
                }
            );
        }
        // La primera imagen y una a los 3 s: se leen para comprobar que el contenido es real.
        let leer = (guardadas == 0 && importados == 0)
            || (guardadas == 1 && inicio.elapsed() > Duration::from_secs(3));
        match estado.importar_y_copiar(&dispositivo, &cola, cuadro, leer) {
            Ok(pixeles) => {
                importados += 1;
                if let Some((ancho, alto, datos)) = pixeles {
                    guardadas += 1;
                    guardar_imagen(&format!("{salida}_{guardadas}"), ancho, alto, &datos);
                }
            }
            Err(e) => {
                errores += 1;
                primer_error.get_or_insert(e);
            }
        }
        if ultimo_informe.elapsed() >= Duration::from_secs(1) {
            let c = contadores.lock().unwrap();
            println!(
                "  recibidos {} · importados {importados} · errores {errores}",
                c.recibidos
            );
            ultimo_informe = Instant::now();
        }
    }
    // Se espera a que la GPU termine lo pendiente antes de medir y cerrar.
    let _ = dispositivo.poll(wgpu::PollType::wait_indefinitely());
    let tiempo = inicio.elapsed();
    let cpu = cpu_segundos() - cpu0;
    let _ = pipeline.set_state(gst::State::Null);

    println!("\n== RESULTADO ==");
    {
        let c = contadores.lock().unwrap();
        println!(
            "recibidos {} · de memoria común {} · caps: {}",
            c.recibidos,
            c.memoria_comun,
            c.ultimo_caps.as_deref().unwrap_or("?")
        );
    }
    resumen(importados, tiempo, cpu);
    match primer_error {
        Some(e) => println!("PRIMER ERROR de importación: {e}  ({errores} en total)"),
        None if importados > 0 => println!("sin errores de importación"),
        None => println!("no se importó ningún cuadro"),
    }
    if guardadas > 0 {
        println!("imágenes importadas guardadas en {salida}_N.png: miralas para ver si son reales");
    }
}

/// Arma `pipewiresrc ! capsfilter ! appsink` pidiendo DMA-BUF y deja los cuadros en `ultimo`.
fn armar(
    sesion: &SesionDeCaptura,
    caps: &str,
    contadores: &Arc<Mutex<Contadores>>,
    ultimo: &Arc<Mutex<Option<CuadroDma>>>,
) -> Result<gst::Pipeline, String> {
    use std::str::FromStr;
    let hacer = |e: Result<gst::Element, _>| e.map_err(|e: gst::glib::BoolError| e.to_string());
    // `pipewiresrc` se queda con el descriptor que recibe y lo cierra al terminar: cada intento
    // usa un duplicado, y el de la sesión sigue sirviendo para el siguiente.
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
                gst::Caps::from_str(caps).map_err(|e| e.to_string())?,
            )
            .build(),
    )?;
    let appsink = gst_app::AppSink::builder()
        .sync(false)
        .max_buffers(2)
        .drop(true)
        .build();
    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([&fuente, &filtro, appsink.upcast_ref()])
        .map_err(|e| e.to_string())?;
    gst::Element::link_many([&fuente, &filtro, appsink.upcast_ref()]).map_err(|e| e.to_string())?;

    {
        let mut c = contadores.lock().unwrap();
        *c = Contadores::default();
    }
    let contadores = Arc::clone(contadores);
    let ultimo = Arc::clone(ultimo);
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let muestra = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                match cuadro_dma(&muestra) {
                    Ok(cuadro) => {
                        let mut c = contadores.lock().unwrap();
                        c.recibidos += 1;
                        c.ultimo_caps = muestra.caps().map(|c| c.to_string());
                        drop(c);
                        *ultimo.lock().unwrap() = Some(cuadro);
                    }
                    Err(motivo) => {
                        let mut c = contadores.lock().unwrap();
                        if c.ultimo_caps.is_none() {
                            c.ultimo_caps = muestra.caps().map(|c| c.to_string());
                        }
                        if motivo.starts_with("memoria común") {
                            c.memoria_comun += 1;
                        }
                        c.falla = Some(motivo);
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    if pipeline.set_state(gst::State::Playing).is_err() {
        // El bus guarda el motivo (el `Err` de arriba solo dice «falló el cambio de estado»).
        let motivo = pipeline
            .bus()
            .and_then(|bus| {
                bus.timed_pop_filtered(
                    gst::ClockTime::from_mseconds(500),
                    &[gst::MessageType::Error],
                )
            })
            .map(|m| match m.view() {
                gst::MessageView::Error(e) => format!("{} ({:?})", e.error(), e.debug()),
                _ => "?".into(),
            })
            .unwrap_or_else(|| "sin mensaje en el bus".into());
        let _ = pipeline.set_state(gst::State::Null);
        return Err(motivo);
    }
    Ok(pipeline)
}

/// Saca de una muestra el descriptor y los datos del buffer, o dice por qué no se puede.
fn cuadro_dma(muestra: &gst::Sample) -> Result<CuadroDma, String> {
    let caps = muestra.caps().ok_or("la muestra no tiene caps")?;
    let estructura = caps.structure(0).ok_or("caps vacías")?;
    let ancho = estructura.get::<i32>("width").map_err(|e| e.to_string())? as u32;
    let alto = estructura.get::<i32>("height").map_err(|e| e.to_string())? as u32;
    // `drm-format` es un texto «XR24:0x0100000000000001» (fourcc : modificador).
    let (fourcc, modificador) = match estructura.get::<&str>("drm-format") {
        Ok(texto) => match texto.split_once(':') {
            Some((f, m)) => {
                let m = u64::from_str_radix(m.trim_start_matches("0x"), 16)
                    .map_err(|e| format!("modificador ilegible «{m}»: {e}"))?;
                (f.to_string(), m)
            }
            // Sin «:» no hay modificador en las caps: se supone lineal (0).
            None => (texto.to_string(), 0),
        },
        Err(_) => (
            estructura.get::<&str>("format").unwrap_or("?").to_string(),
            0,
        ),
    };

    let buffer = muestra.buffer().ok_or("la muestra no tiene buffer")?;
    let memoria = buffer.peek_memory(0);
    let dma = memoria
        .downcast_memory_ref::<gst_alloc::DmaBufMemory>()
        .ok_or("memoria común: el buffer no es DMA-BUF")?;
    // Se duplica el descriptor: la importación se queda con el suyo, y el del buffer sigue
    // siendo de GStreamer.
    let fd = unsafe { BorrowedFd::borrow_raw(dma.fd()) }
        .try_clone_to_owned()
        .map_err(|e| format!("no se pudo duplicar el fd: {e}"))?;
    let (stride, offset, con_meta) = match buffer.meta::<gst_video::VideoMeta>() {
        Some(meta) => (meta.stride()[0] as u64, meta.offset()[0] as u64, true),
        // Sin `VideoMeta` se supone un buffer compacto de 4 bytes por píxel.
        None => (ancho as u64 * 4, 0, false),
    };
    Ok(CuadroDma {
        fd,
        ancho,
        alto,
        fourcc,
        modificador,
        stride,
        offset,
        con_meta,
        _muestra: muestra.clone(),
    })
}

fn esperar_primer_cuadro(contadores: &Arc<Mutex<Contadores>>) -> bool {
    let limite = Instant::now() + Duration::from_secs(5);
    while Instant::now() < limite {
        if contadores.lock().unwrap().recibidos > 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// ---------------------------------------------------------------------------------------
// wgpu
// ---------------------------------------------------------------------------------------

/// Abre la GPU con la feature de importación de DMA-BUF (`ADAPTADOR` elige por nombre).
fn abrir_gpu() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    // En esta versión de wgpu no hay `Default`: se parte del constructor «sin pantalla».
    let mut descripcion = wgpu::InstanceDescriptor::new_without_display_handle();
    descripcion.backends = wgpu::Backends::VULKAN;
    let instancia = wgpu::Instance::new(descripcion);
    let filtro = std::env::var("ADAPTADOR").ok().map(|s| s.to_lowercase());
    let adaptadores = pollster::block_on(instancia.enumerate_adapters(wgpu::Backends::VULKAN));
    println!("adaptadores Vulkan:");
    let mut elegido = None;
    for a in adaptadores {
        let info = a.get_info();
        let tiene = a
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF);
        println!(
            "  {} ({:?}) · importa DMA-BUF: {}",
            info.name,
            info.device_type,
            if tiene { "sí" } else { "NO" }
        );
        let coincide = filtro
            .as_ref()
            .is_none_or(|f| info.name.to_lowercase().contains(f));
        if elegido.is_none() && coincide && tiene {
            elegido = Some((a, info.name));
        }
    }
    let Some((adaptador, nombre)) = elegido else {
        println!(
            "RESULTADO: ninguna GPU puede importar DMA-BUF (falta VULKAN_EXTERNAL_MEMORY_DMA_BUF)."
        );
        return None;
    };
    let resultado = pollster::block_on(adaptador.request_device(&wgpu::DeviceDescriptor {
        label: Some("prueba_dmabuf"),
        required_features: wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }));
    match resultado {
        Ok((d, c)) => Some((d, c, nombre)),
        Err(e) => {
            println!("RESULTADO: no se pudo abrir el dispositivo: {e}");
            None
        }
    }
}

/// Importa cada cuadro y lo copia, dentro de la GPU, a una textura propia y estable.
struct Importador {
    destino: Option<(wgpu::Texture, u32, u32)>,
}

impl Importador {
    fn nuevo(_: &wgpu::Device) -> Self {
        Importador { destino: None }
    }

    /// Devuelve los píxeles (BGRA/RGBA tal cual) si `leer` es verdadero.
    fn importar_y_copiar(
        &mut self,
        dispositivo: &wgpu::Device,
        cola: &wgpu::Queue,
        cuadro: CuadroDma,
        leer: bool,
    ) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        use wgpu::hal::api::Vulkan;
        let formato = match cuadro.fourcc.as_str() {
            // DRM es «little endian»: XR24 es B,G,R,X en memoria.
            "XR24" | "AR24" | "BGRx" | "BGRA" => wgpu::TextureFormat::Bgra8Unorm,
            "XB24" | "AB24" | "RGBx" | "RGBA" => wgpu::TextureFormat::Rgba8Unorm,
            otro => return Err(format!("formato {otro} sin soporte en la prueba")),
        };
        let tamano = wgpu::Extent3d {
            width: cuadro.ancho,
            height: cuadro.alto,
            depth_or_array_layers: 1,
        };
        let descripcion = wgpu::TextureDescriptor {
            label: Some("dmabuf importado"),
            size: tamano,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: formato,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let descripcion_hal = wgpu::hal::TextureDescriptor {
            label: Some("dmabuf importado"),
            size: tamano,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: formato,
            usage: wgpu::TextureUses::COPY_SRC,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: vec![],
        };

        // SAFETY: el fd es un DMA-BUF válido que describe `cuadro` (viene del buffer de
        // PipeWire, que se mantiene vivo en `_muestra`); wgpu se queda con el descriptor.
        let importada = unsafe {
            let hal = dispositivo
                .as_hal::<Vulkan>()
                .ok_or("el dispositivo no es de Vulkan")?;
            let textura = hal
                .texture_from_dmabuf_fd(
                    cuadro.fd,
                    &descripcion_hal,
                    cuadro.modificador,
                    cuadro.stride,
                    cuadro.offset,
                )
                .map_err(|e| format!("texture_from_dmabuf_fd: {e:?}"))?;
            dispositivo.create_texture_from_hal::<Vulkan>(
                textura,
                &descripcion,
                wgpu::TextureUses::COPY_SRC,
            )
        };

        let mut codificador = dispositivo.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("copiar dmabuf"),
        });
        // La textura propia (la que usaría el programa): se recrea si cambia el tamaño.
        if self
            .destino
            .as_ref()
            .is_none_or(|(_, a, h)| *a != cuadro.ancho || *h != cuadro.alto)
        {
            let t = dispositivo.create_texture(&wgpu::TextureDescriptor {
                label: Some("destino"),
                size: tamano,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: formato,
                usage: wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.destino = Some((t, cuadro.ancho, cuadro.alto));
        }
        let destino = &self.destino.as_ref().unwrap().0;
        codificador.copy_texture_to_texture(
            importada.as_image_copy(),
            destino.as_image_copy(),
            tamano,
        );

        // Para comprobar el contenido se lee la textura importada con la CPU (solo un par de
        // cuadros: es una verificación, no el camino normal).
        let lectura = leer.then(|| {
            let por_fila = (cuadro.ancho * 4).div_ceil(256) * 256;
            let buffer = dispositivo.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lectura"),
                size: por_fila as u64 * cuadro.alto as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            codificador.copy_texture_to_buffer(
                importada.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(por_fila),
                        rows_per_image: Some(cuadro.alto),
                    },
                },
                tamano,
            );
            (buffer, por_fila)
        });
        cola.submit([codificador.finish()]);

        let Some((buffer, por_fila)) = lectura else {
            return Ok(None);
        };
        let (emisor, receptor) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = emisor.send(r);
        });
        let _ = dispositivo.poll(wgpu::PollType::wait_indefinitely());
        receptor
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("no se pudo leer el buffer: {e:?}"))?;
        let datos = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| format!("no se pudo mirar el buffer: {e:?}"))?;
        let mut compacto = Vec::with_capacity(cuadro.ancho as usize * cuadro.alto as usize * 4);
        for fila in 0..cuadro.alto as usize {
            let inicio = fila * por_fila as usize;
            compacto.extend_from_slice(&datos[inicio..inicio + cuadro.ancho as usize * 4]);
        }
        // A BGRA, para guardar siempre igual.
        if formato == wgpu::TextureFormat::Rgba8Unorm {
            for px in compacto.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        Ok(Some((cuadro.ancho, cuadro.alto, compacto)))
    }
}

// ---------------------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------------------

/// Guarda una imagen BGRA como PNG (por un PPM y `magick`, que no sumamos como dependencia).
fn guardar_imagen(prefijo: &str, ancho: u32, alto: u32, bgra: &[u8]) {
    // El 4.º byte de XR24 («X») no se usa: se mira qué valor trae, porque Bevy lo leería como
    // alfa y un 0 dejaría la capa invisible.
    let alfas = bgra.chunks_exact(4).map(|p| p[3]);
    let (minimo, maximo) = alfas.fold((255u8, 0u8), |(a, b), v| (a.min(v), b.max(v)));
    println!("byte alfa/X de la imagen: mínimo {minimo}, máximo {maximo}");
    let ppm = format!("{prefijo}.ppm");
    let mut datos = format!("P6\n{ancho} {alto}\n255\n").into_bytes();
    for px in bgra.chunks_exact(4) {
        datos.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    if std::fs::write(&ppm, datos).is_err() {
        return;
    }
    let png = format!("{prefijo}.png");
    let ok = std::process::Command::new("magick")
        .args([&ppm, &png])
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        let _ = std::fs::remove_file(&ppm);
    }
}

fn resumen(cuadros: u64, tiempo: Duration, cpu: f64) {
    let s = tiempo.as_secs_f64();
    println!(
        "{cuadros} cuadros en {s:.1} s = {:.1} por segundo · CPU: {cpu:.2} s = {:.0} % de un núcleo",
        cuadros as f64 / s,
        cpu / s * 100.0
    );
}

/// Segundos de CPU (usuario + sistema) que lleva este proceso. Se supone 100 ticks por
/// segundo, que es lo habitual en Linux.
fn cpu_segundos() -> f64 {
    let Ok(texto) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    // Lo que sigue al «)» del nombre del proceso: los campos desde el 3.º; utime y stime son
    // el 14.º y el 15.º.
    let resto = texto.rsplit_once(')').map_or("", |(_, r)| r);
    let campos: Vec<&str> = resto.split_whitespace().collect();
    let tick = |i: usize| {
        campos
            .get(i)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (tick(11) + tick(12)) / 100.0
}

fn ruta_del_token() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("hypnotica").join("prueba_dmabuf.token")
}

fn leer_token(ruta: &PathBuf) -> Option<String> {
    std::fs::read_to_string(ruta)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn guardar_token(ruta: &PathBuf, token: &str) {
    if let Some(carpeta) = ruta.parent() {
        let _ = std::fs::create_dir_all(carpeta);
    }
    let _ = std::fs::write(ruta, token);
}
