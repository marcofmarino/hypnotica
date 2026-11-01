//! Patrones de prueba para alinear y calibrar proyectores. Rust puro: genera los píxeles
//! (RGBA de 8 bits, sRGB) y quien dibuja decide cómo mostrarlos.
//!
//! Sirven para: ver los bordes de cada proyector y su resolución (`Grilla`), comprobar que
//! los colores salen bien (`Barras`, `Rojo`, `Verde`, `Azul`), igualar brillo y gamma entre
//! proyectores (`Degradado`, `Blanco`, `Negro`) y saber cuánta luz sale con "negro".

/// Un patrón de prueba.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatronPrueba {
    /// Líneas cada 100 px, marco en los bordes, diagonales, cruz y círculo central.
    Grilla,
    /// Barras de color: blanco, amarillo, cian, verde, magenta, rojo, azul y negro.
    Barras,
    /// Rampa de grises (arriba, continua; abajo, en 16 escalones).
    Degradado,
    Blanco,
    Negro,
    Rojo,
    Verde,
    Azul,
}

impl PatronPrueba {
    /// Todos, en el orden en que se ofrecen en la interfaz.
    pub const TODOS: [PatronPrueba; 8] = [
        PatronPrueba::Grilla,
        PatronPrueba::Barras,
        PatronPrueba::Degradado,
        PatronPrueba::Blanco,
        PatronPrueba::Negro,
        PatronPrueba::Rojo,
        PatronPrueba::Verde,
        PatronPrueba::Azul,
    ];

    pub fn nombre(self) -> &'static str {
        match self {
            PatronPrueba::Grilla => "Grilla",
            PatronPrueba::Barras => "Barras de color",
            PatronPrueba::Degradado => "Degradado de grises",
            PatronPrueba::Blanco => "Blanco",
            PatronPrueba::Negro => "Negro",
            PatronPrueba::Rojo => "Rojo",
            PatronPrueba::Verde => "Verde",
            PatronPrueba::Azul => "Azul",
        }
    }

    /// ¿Lleva escrito el nombre del monitor y su resolución? Solo los patrones con detalle
    /// (en los de color liso un texto estorbaría para medir).
    pub fn lleva_rotulo(self) -> bool {
        matches!(self, PatronPrueba::Grilla | PatronPrueba::Barras)
    }
}

type Rgba = [u8; 4];

const NEGRO: Rgba = [0, 0, 0, 255];
const BLANCO: Rgba = [255, 255, 255, 255];

/// Los píxeles del patrón, de `ancho` × `alto`, fila por fila (RGBA, 4 bytes por píxel).
pub fn generar(patron: PatronPrueba, ancho: usize, alto: usize) -> Vec<u8> {
    let mut datos = vec![0u8; ancho * alto * 4];
    if ancho == 0 || alto == 0 {
        return datos;
    }
    for y in 0..alto {
        for x in 0..ancho {
            let color = match patron {
                PatronPrueba::Grilla => pixel_de_grilla(x, y, ancho, alto),
                PatronPrueba::Barras => pixel_de_barras(x, ancho),
                PatronPrueba::Degradado => pixel_de_degradado(x, y, ancho, alto),
                PatronPrueba::Blanco => BLANCO,
                PatronPrueba::Negro => NEGRO,
                PatronPrueba::Rojo => [255, 0, 0, 255],
                PatronPrueba::Verde => [0, 255, 0, 255],
                PatronPrueba::Azul => [0, 0, 255, 255],
            };
            let i = (y * ancho + x) * 4;
            datos[i..i + 4].copy_from_slice(&color);
        }
    }
    datos
}

/// Grosor (en píxeles) del marco que rodea la pantalla.
const GROSOR_MARCO: usize = 4;

fn pixel_de_grilla(x: usize, y: usize, ancho: usize, alto: usize) -> Rgba {
    // Marco blanco: muestra hasta dónde llega realmente la imagen del proyector.
    if x < GROSOR_MARCO || y < GROSOR_MARCO || x + GROSOR_MARCO >= ancho || y + GROSOR_MARCO >= alto
    {
        return BLANCO;
    }
    let (cx, cy) = (ancho as f32 / 2.0, alto as f32 / 2.0);
    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);

    // Círculo central: si se ve ovalado, la superficie está estirada.
    let radio = alto as f32 / 3.0;
    if ((dx * dx + dy * dy).sqrt() - radio).abs() < 1.5 {
        return [80, 255, 120, 255];
    }
    // Diagonales de esquina a esquina.
    let diagonal = (dx * alto as f32 - dy * ancho as f32).abs() / (ancho as f32).hypot(alto as f32);
    let otra = (dx * alto as f32 + dy * ancho as f32).abs() / (ancho as f32).hypot(alto as f32);
    if diagonal < 1.0 || otra < 1.0 {
        return [255, 210, 0, 255];
    }
    // Cruz central.
    if dx.abs() < 1.5 || dy.abs() < 1.5 {
        return BLANCO;
    }
    // Líneas cada 100 px, más claras cada 500.
    let en_linea = |v: usize, paso: usize| v.is_multiple_of(paso);
    if en_linea(x, 500) || en_linea(y, 500) {
        return [200, 200, 200, 255];
    }
    if en_linea(x, 100) || en_linea(y, 100) {
        return [90, 90, 90, 255];
    }
    NEGRO
}

/// Las 8 barras, de izquierda a derecha.
const BARRAS: [Rgba; 8] = [
    [255, 255, 255, 255],
    [255, 255, 0, 255],
    [0, 255, 255, 255],
    [0, 255, 0, 255],
    [255, 0, 255, 255],
    [255, 0, 0, 255],
    [0, 0, 255, 255],
    [0, 0, 0, 255],
];

fn pixel_de_barras(x: usize, ancho: usize) -> Rgba {
    BARRAS[(x * BARRAS.len() / ancho).min(BARRAS.len() - 1)]
}

/// Cantidad de escalones de la mitad de abajo del degradado.
const ESCALONES: usize = 16;

fn pixel_de_degradado(x: usize, y: usize, ancho: usize, alto: usize) -> Rgba {
    let t = if ancho > 1 {
        x as f32 / (ancho - 1) as f32
    } else {
        0.0
    };
    let t = if y * 2 < alto {
        t // arriba: continuo
    } else {
        // Abajo: en escalones parejos, del negro al blanco.
        let escalon = ((x * ESCALONES) / ancho).min(ESCALONES - 1);
        escalon as f32 / (ESCALONES - 1) as f32
    };
    let v = (t * 255.0).round() as u8;
    [v, v, v, 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(datos: &[u8], ancho: usize, x: usize, y: usize) -> Rgba {
        let i = (y * ancho + x) * 4;
        [datos[i], datos[i + 1], datos[i + 2], datos[i + 3]]
    }

    #[test]
    fn todos_los_patrones_tienen_el_tamano_pedido_y_son_opacos() {
        for p in PatronPrueba::TODOS {
            let d = generar(p, 64, 36);
            assert_eq!(d.len(), 64 * 36 * 4, "{p:?}");
            assert!(
                d.chunks(4).all(|px| px[3] == 255),
                "{p:?} tiene transparencia"
            );
        }
        assert!(generar(PatronPrueba::Grilla, 0, 10).is_empty());
    }

    #[test]
    fn los_colores_lisos_son_los_que_dicen() {
        let ancho = 8;
        assert_eq!(
            pixel(&generar(PatronPrueba::Blanco, ancho, 4), ancho, 3, 2),
            BLANCO
        );
        assert_eq!(
            pixel(&generar(PatronPrueba::Negro, ancho, 4), ancho, 3, 2),
            NEGRO
        );
        assert_eq!(
            pixel(&generar(PatronPrueba::Rojo, ancho, 4), ancho, 0, 0),
            [255, 0, 0, 255]
        );
        assert_eq!(
            pixel(&generar(PatronPrueba::Verde, ancho, 4), ancho, 0, 0),
            [0, 255, 0, 255]
        );
        assert_eq!(
            pixel(&generar(PatronPrueba::Azul, ancho, 4), ancho, 0, 0),
            [0, 0, 255, 255]
        );
    }

    #[test]
    fn las_barras_van_del_blanco_al_negro_en_partes_iguales() {
        let ancho = 80;
        let d = generar(PatronPrueba::Barras, ancho, 2);
        for (i, esperado) in BARRAS.iter().enumerate() {
            // El centro de cada barra de 10 px.
            assert_eq!(pixel(&d, ancho, i * 10 + 5, 1), *esperado, "barra {i}");
        }
    }

    #[test]
    fn la_grilla_tiene_marco_cruz_y_fondo_negro() {
        let (ancho, alto) = (1920, 1080);
        let d = generar(PatronPrueba::Grilla, ancho, alto);
        // El marco toca los cuatro bordes.
        for (x, y) in [(0, 500), (ancho - 1, 500), (900, 0), (900, alto - 1)] {
            assert_eq!(pixel(&d, ancho, x, y), BLANCO, "borde en ({x}, {y})");
        }
        // Cruz central.
        assert_eq!(pixel(&d, ancho, ancho / 2, 100), BLANCO);
        // Un punto cualquiera entre líneas está en negro.
        assert_eq!(pixel(&d, ancho, 150, 150 + 17), NEGRO);
        // Hay líneas de 100 px.
        assert_eq!(pixel(&d, ancho, 300, 130), [90, 90, 90, 255]);
    }

    #[test]
    fn el_degradado_sube_y_abajo_tiene_16_escalones() {
        let (ancho, alto) = (256, 40);
        let d = generar(PatronPrueba::Degradado, ancho, alto);
        // Arriba: sube sin bajar nunca, de 0 a 255.
        let mut anterior = 0u8;
        for x in 0..ancho {
            let v = pixel(&d, ancho, x, 5)[0];
            assert!(v >= anterior);
            anterior = v;
        }
        assert_eq!(pixel(&d, ancho, 0, 5)[0], 0);
        assert_eq!(pixel(&d, ancho, ancho - 1, 5)[0], 255);
        // Abajo: exactamente 16 valores distintos.
        let mut valores: Vec<u8> = (0..ancho).map(|x| pixel(&d, ancho, x, 30)[0]).collect();
        valores.dedup();
        assert_eq!(valores.len(), 16);
    }

    #[test]
    fn solo_los_patrones_con_detalle_llevan_rotulo() {
        assert!(PatronPrueba::Grilla.lleva_rotulo());
        assert!(!PatronPrueba::Blanco.lleva_rotulo());
    }
}
