//! Conversión entre coordenadas de la pantalla (píxeles de egui) y de la salida.
//!
//! Matemática pura: usa solo arrays, sin egui ni Bevy, para poder probarla con tests.
//!
//! - **Pantalla:** el origen es la esquina superior izquierda de la ventana y `y` crece
//!   hacia abajo.
//! - **Salida:** el origen es el centro del monitor y `y` crece hacia arriba (así
//!   trabaja la cámara 2D de Bevy).

/// Lleva un punto de la pantalla (dentro del rectángulo donde se dibuja la vista previa)
/// a coordenadas de la salida.
///
/// - `rect_min`, `rect_tam`: esquina superior izquierda y tamaño del rectángulo en pantalla.
/// - `salida_tam`: tamaño de la salida en sus propias unidades (el del monitor).
pub fn pantalla_a_salida(
    punto: [f32; 2],
    rect_min: [f32; 2],
    rect_tam: [f32; 2],
    salida_tam: [f32; 2],
) -> [f32; 2] {
    // `u` y `v` van de 0 a 1 dentro del rectángulo (0,0 = esquina superior izquierda).
    let u = (punto[0] - rect_min[0]) / rect_tam[0];
    let v = (punto[1] - rect_min[1]) / rect_tam[1];
    [
        (u - 0.5) * salida_tam[0],
        // En la salida `y` crece hacia arriba, de ahí el signo invertido.
        (0.5 - v) * salida_tam[1],
    ]
}

/// Lo inverso de `pantalla_a_salida`.
pub fn salida_a_pantalla(
    punto: [f32; 2],
    rect_min: [f32; 2],
    rect_tam: [f32; 2],
    salida_tam: [f32; 2],
) -> [f32; 2] {
    let u = punto[0] / salida_tam[0] + 0.5;
    let v = 0.5 - punto[1] / salida_tam[1];
    [rect_min[0] + u * rect_tam[0], rect_min[1] + v * rect_tam[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: [f32; 2] = [100.0, 50.0];
    const TAM: [f32; 2] = [480.0, 270.0];
    const SALIDA: [f32; 2] = [1920.0, 1080.0];

    fn casi(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn el_centro_de_la_vista_previa_es_el_origen_de_la_salida() {
        let centro = [MIN[0] + TAM[0] / 2.0, MIN[1] + TAM[1] / 2.0];
        assert!(casi(
            pantalla_a_salida(centro, MIN, TAM, SALIDA),
            [0.0, 0.0]
        ));
    }

    #[test]
    fn las_esquinas_del_rectangulo_son_las_del_monitor() {
        // Arriba a la izquierda de la pantalla = (-ancho/2, +alto/2) en la salida.
        assert!(casi(
            pantalla_a_salida(MIN, MIN, TAM, SALIDA),
            [-960.0, 540.0]
        ));
        // Abajo a la derecha de la pantalla = (+ancho/2, -alto/2) en la salida.
        let abajo_der = [MIN[0] + TAM[0], MIN[1] + TAM[1]];
        assert!(casi(
            pantalla_a_salida(abajo_der, MIN, TAM, SALIDA),
            [960.0, -540.0]
        ));
    }

    #[test]
    fn ida_y_vuelta() {
        for p in [[0.0, 0.0], [-300.5, 120.25], [959.0, -539.0]] {
            let en_pantalla = salida_a_pantalla(p, MIN, TAM, SALIDA);
            assert!(casi(pantalla_a_salida(en_pantalla, MIN, TAM, SALIDA), p));
        }
    }
}
