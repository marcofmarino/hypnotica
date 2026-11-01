//! Prueba aislada de los orígenes en vivo: una URL y una cámara.
//! Se ejecuta con: cargo run -p vm_video --example prueba_en_vivo -- <url> [/dev/videoN]

use std::time::{Duration, Instant};

use vm_video::decodificador::{Ajustes, Origen, Video, listar_dispositivos};

fn contar(video: &Video, segundos: u64) -> u32 {
    let inicio = Instant::now();
    let mut cuenta = 0;
    while inicio.elapsed() < Duration::from_secs(segundos) {
        if video.ultimo_frame().is_some() {
            cuenta += 1;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cuenta
}

fn main() {
    println!("dispositivos de video: {:?}", listar_dispositivos());
    let argumentos: Vec<String> = std::env::args().skip(1).collect();

    if let Some(url) = argumentos.first() {
        match Video::abrir_origen(
            &Origen::Url(url.clone()),
            Ajustes::default(),
            Some((1920, 1080)),
        ) {
            Ok(v) => {
                let n = contar(&v, 3);
                println!(
                    "URL {url}: {n} frames en 3 s, error: {:?}, posición: {:?}",
                    v.error(),
                    v.posicion()
                );
            }
            Err(e) => println!("URL {url}: no se pudo abrir: {e}"),
        }
    }

    let camara = argumentos
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/dev/video99".into());
    match Video::abrir_origen(&Origen::Camara(camara.clone()), Ajustes::default(), None) {
        Ok(v) => {
            let n = contar(&v, 2);
            println!("cámara {camara}: {n} frames en 2 s, error: {:?}", v.error());
        }
        Err(e) => println!("cámara {camara}: no se pudo abrir (esperado si no existe): {e}"),
    }
}
