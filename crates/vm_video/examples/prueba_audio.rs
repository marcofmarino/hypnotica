//! Diagnóstico del audio de una aplicación, SIN el selector de pantalla: captura el sonido
//! de una aplicación con la misma cadena que usa el programa y prueba varias variantes
//! seguidas (8 s cada una), midiendo cortes, subdesbordes y cómo llegan los bloques.
//!
//! La aplicación tiene que estar sonando (un video del navegador, por ejemplo):
//!   APP=Firefox cargo run --release -p vm_video --example prueba_audio
//! Sin `APP`, muestra la lista de aplicaciones que suenan.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use vm_video::audio::{listar_aplicaciones_de_audio, serial_de_aplicacion};

const SEGUNDOS: u64 = 8;

/// Lo que se mide en una variante.
#[derive(Default)]
struct Medidas {
    /// Instante de llegada y duración (ms) de cada bloque no vacío.
    bloques: Vec<(Instant, f64)>,
    /// Bloques sin datos (tamaño 0).
    vacios: u32,
}

fn main() {
    gst::init().expect("no se pudo iniciar GStreamer");
    let apps = listar_aplicaciones_de_audio();
    println!(
        "aplicaciones sonando: {:?}",
        apps.iter().map(|a| a.nombre.as_str()).collect::<Vec<_>>()
    );
    let Ok(app) = std::env::var("APP") else {
        return println!("indicá cuál con APP=nombre");
    };
    let Some(serial) = serial_de_aplicacion(&app) else {
        return println!("«{app}» no está sonando");
    };
    println!("capturando «{app}» (serial {serial}), {SEGUNDOS} s por variante\n");

    // Los avisos de GStreamer se cuentan, no se imprimen.
    let avisos: Arc<Mutex<BTreeMap<&'static str, u32>>> = Arc::new(Mutex::new(BTreeMap::new()));
    gst::log::remove_default_log_function();
    gst::log::set_default_threshold(gst::DebugLevel::Warning);
    let contador = Arc::clone(&avisos);
    gst::log::add_log_function(move |_, nivel, _, _, _, _, mensaje| {
        if nivel > gst::DebugLevel::Warning {
            return;
        }
        let texto = mensaje.get().map(|m| m.to_string()).unwrap_or_default();
        let tipo = if texto.contains("Unexpected discontinuity") {
            "cortes"
        } else if texto.contains("underflow") {
            "subdesbordes"
        } else if texto.contains("clock skew") {
            "deriva"
        } else {
            return;
        };
        if let Ok(mut m) = contador.lock() {
            *m.entry(tipo).or_default() += 1;
        }
    });

    // (nombre, propiedades de pipewiresrc, elementos antes de la salida, propiedades de la salida)
    let variantes: [(&str, &str, &str, &str); 6] = [
        ("actual", "do-timestamp=true", "", ""),
        ("sin_marcas_de_tiempo", "do-timestamp=false", "", ""),
        ("salida_sin_sync", "do-timestamp=true", "", "sync=false"),
        (
            "sin_marcas_y_sin_sync",
            "do-timestamp=false",
            "",
            "sync=false",
        ),
        (
            "salida_con_mas_buffer",
            "do-timestamp=true",
            "",
            "buffer-time=800000 latency-time=100000",
        ),
        (
            "cola_de_300_ms",
            "do-timestamp=true",
            "! queue max-size-time=2000000000 max-size-buffers=0 max-size-bytes=0 \
             min-threshold-time=300000000",
            "",
        ),
    ];

    println!(
        "{:<24} {:>6} {:>12} {:>7} {:>8} {:>10} {:>12} {:>12}",
        "variante",
        "cortes",
        "subdesbordes",
        "deriva",
        "vacíos",
        "bloque ms",
        "llegada máx",
        "bloques/s"
    );
    for (nombre, fuente, antes, salida) in variantes {
        avisos.lock().unwrap().clear();
        // Se anuncia antes de sonar, para poder anotar cuál suena bien y cuál no.
        println!("▶ {nombre}: escuchá ahora ({SEGUNDOS} s)…");
        let descripcion = format!(
            "pipewiresrc name=fuente target-object={serial} {fuente} {antes} ! audioconvert \
             ! audioresample ! volume ! pulsesink {salida}"
        );
        let pipeline = match gst::parse::launch(&descripcion) {
            Ok(p) => p,
            Err(e) => {
                println!("{nombre:<24} no se pudo armar: {e}");
                continue;
            }
        };
        let medidas = Arc::new(Mutex::new(Medidas::default()));
        let pipeline_bin = pipeline.downcast_ref::<gst::Bin>().unwrap();
        if let Some(fuente) = pipeline_bin.by_name("fuente") {
            let m = Arc::clone(&medidas);
            fuente.static_pad("src").unwrap().add_probe(
                gst::PadProbeType::BUFFER,
                move |_, info| {
                    if let Some(gst::PadProbeData::Buffer(b)) = &info.data
                        && let Ok(mut m) = m.lock()
                    {
                        if b.size() == 0 {
                            m.vacios += 1;
                        } else {
                            let ms = b.duration().map_or(0.0, |d| d.mseconds() as f64);
                            m.bloques.push((Instant::now(), ms));
                        }
                    }
                    gst::PadProbeReturn::Ok
                },
            );
        }
        let _ = pipeline.set_state(gst::State::Playing);
        std::thread::sleep(Duration::from_secs(SEGUNDOS));
        let _ = pipeline.set_state(gst::State::Null);

        let m = medidas.lock().unwrap();
        let n = m.bloques.len();
        let medio = if n > 0 {
            m.bloques.iter().map(|b| b.1).sum::<f64>() / n as f64
        } else {
            0.0
        };
        let llegada_max = m
            .bloques
            .windows(2)
            .map(|p| p[1].0.duration_since(p[0].0).as_millis())
            .max()
            .unwrap_or(0);
        let a = avisos.lock().unwrap();
        let g = |k: &str| a.get(k).copied().unwrap_or(0);
        println!(
            "{nombre:<24} {:>6} {:>12} {:>7} {:>8} {:>10.1} {:>10} ms {:>12.1}",
            g("cortes"),
            g("subdesbordes"),
            g("deriva"),
            m.vacios,
            medio,
            llegada_max,
            n as f64 / SEGUNDOS as f64
        );
    }
    println!(
        "\n(cortes y subdesbordes más bajos = mejor; «bloque ms» es lo que dura cada bloque que llega)"
    );
}
