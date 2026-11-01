//! Prueba aislada del transporte: audio, velocidad, tramo, bucle, pausa y búsqueda.
//! Se ejecuta con: cargo run -p vm_video --example prueba_transporte -- ruta.mp4
//! (el video tiene que durar unos 10 segundos y, para probar el audio, tener sonido).

use std::time::{Duration, Instant};

use vm_video::decodificador::{Ajustes, Video};

/// Cuenta los frames que llegan durante `segundos` y devuelve la posición al final.
fn mirar(video: &Video, segundos: f32) -> (u32, Option<(f64, f64)>) {
    let inicio = Instant::now();
    let mut cuenta = 0;
    while inicio.elapsed() < Duration::from_secs_f32(segundos) {
        if video.ultimo_frame().is_some() {
            cuenta += 1;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (cuenta, video.posicion())
}

fn mostrar(titulo: &str, resultado: (u32, Option<(f64, f64)>)) {
    match resultado.1 {
        Some((p, d)) => println!(
            "{titulo}: {} frames, posición {p:.2} s de {d:.2} s",
            resultado.0
        ),
        None => println!("{titulo}: {} frames, sin posición", resultado.0),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ruta = std::env::args().nth(1).expect("falta la ruta del video");

    // 1. Normal.
    let v = Video::abrir(&ruta)?;
    mostrar("normal 2 s", mirar(&v, 2.0));
    if let Some(e) = v.error() {
        println!("ERROR: {e}");
    }

    // 2. Pausa: no deben llegar frames, y la posición no avanza.
    v.pausar(true);
    std::thread::sleep(Duration::from_millis(300));
    let antes = v.posicion();
    let (frames, despues) = mirar(&v, 1.0);
    println!(
        "en pausa 1 s: {frames} frames; posición {:?} -> {:?}",
        antes.map(|x| x.0),
        despues.map(|x| x.0)
    );
    v.pausar(false);

    // 3. Buscar a la mitad (con la pausa puesta, tiene que llegar un frame igual).
    v.pausar(true);
    v.buscar(0.5);
    let (frames, pos) = mirar(&v, 1.0);
    println!(
        "buscar 50 % en pausa: {frames} frames, posición {:?}",
        pos.map(|x| x.0)
    );
    v.pausar(false);
    drop(v);

    // 4. Velocidad x2: en 2 s tiene que avanzar unos 4 s del video.
    let v = Video::abrir_con(
        &ruta,
        Ajustes {
            velocidad: 2.0,
            ..Ajustes::default()
        },
        None,
    )?;
    mostrar("velocidad x2, 2.5 s", mirar(&v, 2.5));
    drop(v);

    // 5. Tramo 20 %–40 % con bucle: la posición se queda dentro.
    let v = Video::abrir_con(
        &ruta,
        Ajustes {
            inicio: 0.2,
            fin: 0.4,
            ..Ajustes::default()
        },
        None,
    )?;
    let mut fuera = 0;
    let mut vueltas = 0;
    let mut anterior = 0.0;
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(7) {
        v.ultimo_frame();
        if let Some((p, d)) = v.posicion() {
            let f = p / d;
            if !(0.19..=0.41).contains(&f) && t0.elapsed() > Duration::from_secs(1) {
                fuera += 1;
            }
            if f + 0.05 < anterior {
                vueltas += 1;
            }
            anterior = f;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("tramo 20-40 %: lecturas fuera del tramo = {fuera}, vueltas del bucle = {vueltas}");
    drop(v);

    // 6. Sin bucle: llega al final del tramo y se queda quieto.
    let v = Video::abrir_con(
        &ruta,
        Ajustes {
            inicio: 0.0,
            fin: 0.2,
            bucle: false,
            ..Ajustes::default()
        },
        None,
    )?;
    let (_, _) = mirar(&v, 3.5);
    let (frames, pos) = mirar(&v, 1.0);
    println!(
        "sin bucle, después del final: {frames} frames, posición {:?}",
        pos.map(|x| x.0)
    );

    // 7. Cambiar los ajustes en vivo: velocidad y silencio.
    drop(v);
    let v = Video::abrir(&ruta)?;
    mirar(&v, 1.0);
    v.ajustar(Ajustes {
        velocidad: 3.0,
        silenciado: true,
        volumen: 0.5,
        ..Ajustes::default()
    });
    mostrar("ajustar en vivo (x3 y silencio), 2 s", mirar(&v, 2.0));
    println!("error final: {:?}", v.error());
    Ok(())
}
