//! Saca la miniatura de un video y la guarda como imagen PPM.
//! Se ejecuta con: cargo run -p vm_video --example prueba_miniatura -- video.mp4 160 salida.ppm

use std::io::Write;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argumentos: Vec<String> = std::env::args().collect();
    let ruta = argumentos.get(1).ok_or("falta la ruta del video")?;
    let ancho: u32 = argumentos.get(2).map_or(Ok(160), |a| a.parse())?;
    let salida = argumentos.get(3).ok_or("falta el archivo de salida")?;

    let inicio = Instant::now();
    let frame = vm_video::decodificador::miniatura(ruta, ancho)?;
    println!(
        "miniatura de {}x{} en {:.2} s",
        frame.ancho,
        frame.alto,
        inicio.elapsed().as_secs_f32()
    );

    // PPM: un formato de imagen trivial (sin alfa), útil para probar sin más dependencias.
    let mut archivo = std::fs::File::create(salida)?;
    write!(archivo, "P6\n{} {}\n255\n", frame.ancho, frame.alto)?;
    for pixel in frame.datos.as_chunks::<4>().0 {
        archivo.write_all(&pixel[..3])?;
    }
    Ok(())
}
