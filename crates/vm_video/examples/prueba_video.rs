//! Prueba aislada del módulo de video: cuenta los frames y verifica el loop.
//! Se ejecuta con: cargo run -p vm_video --example prueba_video

use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Se puede pasar la ruta como argumento: `cargo run -p vm_video --example prueba_video -- ruta.mp4`
    let ruta = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/video_prueba_1080.mp4".into());
    // Con LIMITE=1920x1080 los cuadros se reducen a ese tamaño (como hace la composición).
    let limite = std::env::var("LIMITE").ok().and_then(|t| {
        let (a, b) = t.split_once('x')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    // Con SIN_BUCLE=1 el video no vuelve a empezar al terminar (así se muestra una imagen).
    let ajustes = vm_video::decodificador::Ajustes {
        bucle: std::env::var_os("SIN_BUCLE").is_none(),
        ..Default::default()
    };
    let video = vm_video::decodificador::Video::abrir_con(&ruta, ajustes, limite)?;
    let inicio = Instant::now();
    let mut cuenta = 0;

    // Se miran 4 segundos: alcanza para saber si llegan frames.
    while inicio.elapsed() < Duration::from_secs(4) {
        if let Some(frame) = video.ultimo_frame() {
            cuenta += 1;
            if cuenta == 1 {
                println!("primer frame: {}x{}", frame.ancho, frame.alto);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if let Some(error) = video.error() {
        println!("error de GStreamer: {error}");
    }
    println!(
        "{cuenta} frames mostrados en {:.1} s",
        inicio.elapsed().as_secs_f32()
    );
    Ok(())
}
