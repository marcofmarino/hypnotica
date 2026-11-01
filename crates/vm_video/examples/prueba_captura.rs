//! Prueba aislada de la captura por el portal: abre el selector del escritorio y, si se
//! elige algo, durante 20 segundos cuenta **segundo a segundo** los cuadros que llegan (si la
//! imagen se congela, los números caen a 0) y los avisos de GStreamer (cortes de audio,
//! desfases, errores). Movés una ventana mientras tanto.
//!
//! El audio se elige con la variable `AUDIO`:
//!   AUDIO=escritorio     todo el escritorio
//!   AUDIO=Firefox        solo esa aplicación (tiene que estar sonando; nombre como en la lista)
//!   (sin AUDIO)          sin sonido
//!
//! Se ejecuta con: AUDIO=Firefox cargo run --release -p vm_video --example prueba_captura

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use vm_video::audio::{AudioDeCaptura, listar_aplicaciones_de_audio};
use vm_video::captura::iniciar;
use vm_video::decodificador::{Ajustes, Video};

const SEGUNDOS: u64 = 20;

fn main() {
    gst::init().expect("no se pudo iniciar GStreamer");
    // Los avisos no se imprimen todos: se cuentan por tipo (categoría + comienzo del texto).
    let avisos: Arc<Mutex<BTreeMap<String, u32>>> = Arc::new(Mutex::new(BTreeMap::new()));
    gst::log::remove_default_log_function();
    gst::log::set_default_threshold(gst::DebugLevel::Warning);
    let contador = Arc::clone(&avisos);
    gst::log::add_log_function(move |categoria, nivel, _, _, _, _, mensaje| {
        if nivel > gst::DebugLevel::Warning {
            return;
        }
        let texto = mensaje.get().map(|m| m.to_string()).unwrap_or_default();
        let resumen: String = texto.chars().take(70).collect();
        if let Ok(mut m) = contador.lock() {
            *m.entry(format!("{}: {resumen}", categoria.name()))
                .or_default() += 1;
        }
    });

    let audio = match std::env::var("AUDIO") {
        Ok(v) if v.eq_ignore_ascii_case("escritorio") => Some(AudioDeCaptura::Escritorio),
        Ok(v) if !v.is_empty() => Some(AudioDeCaptura::Aplicacion(v)),
        _ => None,
    };
    println!("audio pedido: {audio:?}");
    println!(
        "aplicaciones sonando ahora: {:?}",
        listar_aplicaciones_de_audio()
            .into_iter()
            .map(|a| a.nombre)
            .collect::<Vec<_>>()
    );

    println!("pidiendo la captura al portal (el escritorio debería mostrar su selector)…");
    let sesion = match iniciar(None, false) {
        Ok(s) => s,
        Err(e) => return println!("el portal respondió: {e}"),
    };
    println!(
        "elegido: nodo {} tamaño {:?} token {:?}",
        sesion.nodo, sesion.tamano, sesion.token
    );
    let video = match Video::abrir_captura(sesion, Ajustes::default(), Some((1920, 1080)), audio) {
        Ok(v) => v,
        Err(e) => return println!("no se pudo abrir el video: {e}"),
    };
    println!("movés una ventana o el mouse y dejá sonar el audio: {SEGUNDOS} s");

    let inicio = Instant::now();
    let mut vistos = 0;
    for segundo in 1..=SEGUNDOS {
        let mut cuadros = 0;
        let hasta = Duration::from_secs(segundo);
        while inicio.elapsed() < hasta {
            if video.ultimo_frame().is_some() {
                cuadros += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let total: u32 = avisos.lock().map(|m| m.values().sum()).unwrap_or(0);
        println!(
            "segundo {segundo:>2}: {cuadros:>3} cuadros | avisos nuevos: {:>2} | error: {:?}",
            total - vistos,
            video.error()
        );
        vistos = total;
    }

    println!("\nresumen de avisos de GStreamer (cuántas veces):");
    match avisos.lock() {
        Ok(m) if m.is_empty() => println!("  ninguno"),
        Ok(m) => {
            for (aviso, veces) in m.iter() {
                println!("  {veces:>4} × {aviso}");
            }
        }
        Err(_) => {}
    }
}
