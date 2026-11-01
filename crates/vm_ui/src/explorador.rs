//! La lógica del explorador de archivos: qué carpetas y videos hay en una carpeta y en qué
//! orden se muestran. Sin egui ni Bevy: se prueba con tests.

use std::path::{Path, PathBuf};

use vm_core::{EXTENSIONES_DE_IMAGEN, es_ruta_de_imagen};

/// Las extensiones de video que se aceptan (en minúsculas).
pub(crate) const EXTENSIONES_VIDEO: [&str; 6] = ["mp4", "mov", "mkv", "avi", "webm", "m4v"];

/// Una carpeta, un video o una imagen dentro de la carpeta que se está mirando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntradaArchivo {
    pub nombre: String,
    pub ruta: PathBuf,
    pub es_carpeta: bool,
    /// Tamaño en bytes (0 para las carpetas).
    pub bytes: u64,
}

/// ¿La ruta es un archivo de video, por su extensión?
pub(crate) fn es_video(ruta: &Path) -> bool {
    ruta.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES_VIDEO.contains(&e.to_ascii_lowercase().as_str()))
}

/// ¿Es un archivo que se puede poner en un clip: un video o una imagen?
pub(crate) fn es_medio(ruta: &Path) -> bool {
    es_video(ruta) || es_ruta_de_imagen(ruta)
}

/// Todas las extensiones que ofrece el selector de archivos: videos e imágenes.
pub(crate) fn extensiones_de_medios() -> Vec<&'static str> {
    EXTENSIONES_VIDEO
        .iter()
        .chain(EXTENSIONES_DE_IMAGEN.iter())
        .copied()
        .collect()
}

/// Lista una carpeta: primero las subcarpetas y después los videos y las imágenes, cada grupo
/// por orden alfabético sin distinguir mayúsculas. Los archivos y carpetas ocultos (que
/// empiezan con punto) y los archivos que no son videos ni imágenes no se muestran.
pub(crate) fn listar(carpeta: &Path) -> std::io::Result<Vec<EntradaArchivo>> {
    let mut entradas = Vec::new();
    for entrada in std::fs::read_dir(carpeta)? {
        let Ok(entrada) = entrada else { continue };
        let nombre = entrada.file_name().to_string_lossy().into_owned();
        if nombre.starts_with('.') {
            continue;
        }
        let ruta = entrada.path();
        // `metadata` sigue los enlaces simbólicos: un enlace a una carpeta cuenta como carpeta.
        let Ok(meta) = std::fs::metadata(&ruta) else {
            continue;
        };
        if meta.is_dir() {
            entradas.push(EntradaArchivo {
                nombre,
                ruta,
                es_carpeta: true,
                bytes: 0,
            });
        } else if meta.is_file() && es_medio(&ruta) {
            entradas.push(EntradaArchivo {
                nombre,
                ruta,
                es_carpeta: false,
                bytes: meta.len(),
            });
        }
    }
    entradas.sort_by(|a, b| {
        b.es_carpeta
            .cmp(&a.es_carpeta)
            .then_with(|| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()))
    });
    Ok(entradas)
}

/// ¿Parece la dirección de una transmisión de red? Un protocolo conocido, `://` y algo
/// después, sin espacios.
pub(crate) fn es_url_de_red(texto: &str) -> bool {
    const PROTOCOLOS: [&str; 7] = ["rtsp", "rtsps", "srt", "http", "https", "udp", "rtmp"];
    let t = texto.trim();
    t.split_once("://").is_some_and(|(protocolo, resto)| {
        PROTOCOLOS.contains(&protocolo.to_ascii_lowercase().as_str())
            && !resto.is_empty()
            && !t.contains(char::is_whitespace)
    })
}

/// Cuántas carpetas, como máximo, se recorren al buscar un archivo (para no quedarse
/// trabado en un disco enorme).
const MAXIMO_DE_CARPETAS: usize = 5_000;

/// Busca un archivo por su **nombre** dentro de `raiz` y sus subcarpetas (hasta `profundidad`
/// niveles). Sirve para reencontrar videos que se movieron de lugar. Devuelve el primero que
/// encuentra, mirando primero lo más cercano a `raiz`.
pub(crate) fn buscar_por_nombre(
    raiz: &Path,
    nombre: &std::ffi::OsStr,
    profundidad: usize,
) -> Option<PathBuf> {
    let mut pendientes = std::collections::VecDeque::from([(raiz.to_path_buf(), 0usize)]);
    let mut recorridas = 0;
    while let Some((carpeta, nivel)) = pendientes.pop_front() {
        recorridas += 1;
        if recorridas > MAXIMO_DE_CARPETAS {
            return None;
        }
        let Ok(contenido) = std::fs::read_dir(&carpeta) else {
            continue;
        };
        let mut subcarpetas = Vec::new();
        for entrada in contenido.flatten() {
            let ruta = entrada.path();
            if entrada.file_name() == nombre && ruta.is_file() {
                return Some(ruta);
            }
            let oculta = entrada.file_name().to_string_lossy().starts_with('.');
            if !oculta && nivel < profundidad && ruta.is_dir() {
                subcarpetas.push(ruta);
            }
        }
        subcarpetas.sort();
        pendientes.extend(subcarpetas.into_iter().map(|c| (c, nivel + 1)));
    }
    None
}

/// Las entradas cuyo nombre contiene `filtro` (sin distinguir mayúsculas). Un filtro vacío
/// las deja todas.
pub(crate) fn filtrar<'a>(
    entradas: &'a [EntradaArchivo],
    filtro: &str,
) -> impl Iterator<Item = &'a EntradaArchivo> {
    let filtro = filtro.trim().to_lowercase();
    entradas
        .iter()
        .filter(move |e| filtro.is_empty() || e.nombre.to_lowercase().contains(&filtro))
}

/// Un tamaño legible: «850 KB», «12,3 MB», «1,2 GB».
pub(crate) fn tamano_legible(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    let (valor, unidad) = if b >= KB * KB * KB {
        (b / (KB * KB * KB), "GB")
    } else if b >= KB * KB {
        (b / (KB * KB), "MB")
    } else {
        (b / KB, "KB")
    };
    if valor >= 100.0 || unidad == "KB" {
        format!("{valor:.0} {unidad}")
    } else {
        format!("{valor:.1} {unidad}").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una carpeta temporal única para cada test.
    fn carpeta_de_prueba(nombre: &str) -> PathBuf {
        let c = std::env::temp_dir().join(format!("hypnotica_explorador_{nombre}"));
        let _ = std::fs::remove_dir_all(&c);
        std::fs::create_dir_all(&c).unwrap();
        c
    }

    fn crear(c: &Path, nombre: &str) {
        std::fs::write(c.join(nombre), b"x").unwrap();
    }

    #[test]
    fn solo_las_extensiones_de_video_cuentan_sin_importar_mayusculas() {
        assert!(es_video(Path::new("a.mp4")));
        assert!(es_video(Path::new("/x/B.MOV")));
        assert!(!es_video(Path::new("a.txt")));
        assert!(!es_video(Path::new("sin_extension")));
    }

    #[test]
    fn se_listan_carpetas_primero_y_despues_videos_en_orden() {
        let c = carpeta_de_prueba("orden");
        std::fs::create_dir(c.join("zeta")).unwrap();
        std::fs::create_dir(c.join("Alfa")).unwrap();
        crear(&c, "b.mp4");
        crear(&c, "A.mov");
        crear(&c, "notas.txt");
        crear(&c, ".oculto.mp4");
        std::fs::create_dir(c.join(".carpeta_oculta")).unwrap();

        let nombres: Vec<String> = listar(&c).unwrap().into_iter().map(|e| e.nombre).collect();
        assert_eq!(nombres, ["Alfa", "zeta", "A.mov", "b.mp4"]);
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn el_explorador_muestra_videos_e_imagenes_pero_no_otros_archivos() {
        let c = carpeta_de_prueba("imagenes");
        for nombre in [
            "clip.mp4",
            "logo.PNG",
            "foto.jpg",
            "foto2.jpeg",
            "fondo.webp",
            "viejo.bmp",
            "datos.tiff",
            "notas.txt",
        ] {
            crear(&c, nombre);
        }
        let nombres: Vec<String> = listar(&c).unwrap().into_iter().map(|e| e.nombre).collect();
        // Los formatos que GStreamer no abre acá (BMP, TIFF) no se ofrecen.
        assert_eq!(
            nombres,
            [
                "clip.mp4",
                "fondo.webp",
                "foto.jpg",
                "foto2.jpeg",
                "logo.PNG"
            ]
        );
        assert!(es_medio(Path::new("a.png")) && es_medio(Path::new("a.mkv")));
        assert!(!es_medio(Path::new("a.txt")));
        // El selector de archivos ofrece las dos familias.
        let todas = extensiones_de_medios();
        assert!(todas.contains(&"mp4") && todas.contains(&"png") && todas.contains(&"webp"));
    }

    #[test]
    fn una_carpeta_que_no_existe_da_error() {
        assert!(listar(Path::new("/no/existe/de_ninguna_manera")).is_err());
    }

    #[test]
    fn el_filtro_busca_en_el_nombre_sin_distinguir_mayusculas() {
        let e = |n: &str| EntradaArchivo {
            nombre: n.into(),
            ruta: PathBuf::from(n),
            es_carpeta: false,
            bytes: 0,
        };
        let todas = vec![e("Fuego.mp4"), e("agua.mov"), e("fuegos_artificiales.mkv")];
        let r: Vec<&str> = filtrar(&todas, " FUEGO ")
            .map(|e| e.nombre.as_str())
            .collect();
        assert_eq!(r, ["Fuego.mp4", "fuegos_artificiales.mkv"]);
        assert_eq!(filtrar(&todas, "").count(), 3);
    }

    #[test]
    fn los_tamanos_se_muestran_legibles() {
        assert_eq!(tamano_legible(850 * 1024), "850 KB");
        assert_eq!(tamano_legible(12 * 1024 * 1024 + 300 * 1024), "12,3 MB");
        assert_eq!(tamano_legible(150 * 1024 * 1024), "150 MB");
        assert_eq!(tamano_legible(1_288_490_189), "1,2 GB");
    }

    #[test]
    fn se_encuentra_un_archivo_por_nombre_en_las_subcarpetas() {
        let c = carpeta_de_prueba("buscar");
        std::fs::create_dir_all(c.join("a/b/c")).unwrap();
        std::fs::create_dir_all(c.join("otra")).unwrap();
        crear(&c.join("a/b/c"), "perdido.mp4");
        crear(&c.join("otra"), "otro.mp4");
        let nombre = std::ffi::OsString::from("perdido.mp4");
        let hallado = buscar_por_nombre(&c, &nombre, 5).unwrap();
        assert_eq!(hallado, c.join("a/b/c/perdido.mp4"));
        // Con menos profundidad no llega.
        assert_eq!(buscar_por_nombre(&c, &nombre, 1), None);
        // Un nombre que no existe no se encuentra.
        assert_eq!(
            buscar_por_nombre(&c, std::ffi::OsStr::new("nada.mp4"), 5),
            None
        );
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn la_busqueda_prefiere_lo_mas_cercano_a_la_raiz() {
        let c = carpeta_de_prueba("buscar_cercano");
        std::fs::create_dir_all(c.join("a/profunda")).unwrap();
        crear(&c.join("a/profunda"), "v.mp4");
        crear(&c.join("a"), "v.mp4");
        let hallado = buscar_por_nombre(&c, std::ffi::OsStr::new("v.mp4"), 5).unwrap();
        assert_eq!(hallado, c.join("a/v.mp4"));
        let _ = std::fs::remove_dir_all(&c);
    }

    #[test]
    fn se_reconocen_las_direcciones_de_transmision() {
        assert!(es_url_de_red("rtsp://192.168.1.5:8554/camara"));
        assert!(es_url_de_red("  SRT://servidor:9000 "));
        assert!(es_url_de_red("https://ejemplo.com/stream.m3u8"));
        assert!(!es_url_de_red("rtsp://"));
        assert!(!es_url_de_red("ftp://ejemplo.com/a.mp4"));
        assert!(!es_url_de_red("un video.mp4"));
        assert!(!es_url_de_red("http://con espacio"));
    }
}
