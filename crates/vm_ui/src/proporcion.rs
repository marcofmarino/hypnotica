//! Igualar la superficie de la salida al recorte de la entrada: que tenga su proporción
//! (ancho/alto) o su tamaño real en píxeles. Matemática pura, sin egui ni Bevy.
//!
//! Las esquinas de la salida están en píxeles del proyector, con la `y` hacia arriba y en
//! el orden sup-izq, sup-der, inf-der, inf-izq (como en `vm_core`).

use vm_core::{Entrada, centroide};

/// El ancho y el alto de la superficie en píxeles del proyector: el promedio de lados
/// opuestos (con perspectiva, un lado es más largo que el otro). Sirve para pasar entre
/// «% de la superficie» y «píxeles» al ajustar el borde suave.
pub fn medidas_de_superficie(esquinas: &[[f32; 2]; 4]) -> (f32, f32) {
    let largo = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let ancho = (largo(esquinas[0], esquinas[1]) + largo(esquinas[3], esquinas[2])) / 2.0;
    let alto = (largo(esquinas[0], esquinas[3]) + largo(esquinas[1], esquinas[2])) / 2.0;
    (ancho, alto)
}

/// El tamaño en píxeles del recorte de la entrada, dada la resolución de la composición.
pub fn tamano_de_entrada(entrada: Entrada, ancho_comp: f32, alto_comp: f32) -> [f32; 2] {
    [
        (entrada[2] - entrada[0]) * ancho_comp,
        (entrada[3] - entrada[1]) * alto_comp,
    ]
}

/// La proporción (ancho / alto) del recorte de la entrada, o `None` si no tiene alto.
pub fn proporcion_de_entrada(entrada: Entrada, ancho_comp: f32, alto_comp: f32) -> Option<f32> {
    let [ancho, alto] = tamano_de_entrada(entrada, ancho_comp, alto_comp);
    (ancho > 0.0 && alto > 0.0).then(|| ancho / alto)
}

/// La caja (mínimo, máximo) que contiene las esquinas.
fn caja(esquinas: &[[f32; 2]; 4]) -> ([f32; 2], [f32; 2]) {
    let mut minimo = esquinas[0];
    let mut maximo = esquinas[0];
    for e in esquinas {
        minimo = [minimo[0].min(e[0]), minimo[1].min(e[1])];
        maximo = [maximo[0].max(e[0]), maximo[1].max(e[1])];
    }
    (minimo, maximo)
}

/// Un rectángulo alineado con los ejes, centrado en `centro`, de `ancho` × `alto`.
fn rectangulo_centrado(centro: [f32; 2], ancho: f32, alto: f32) -> [[f32; 2]; 4] {
    let (mx, my) = (ancho / 2.0, alto / 2.0);
    [
        [centro[0] - mx, centro[1] + my],
        [centro[0] + mx, centro[1] + my],
        [centro[0] + mx, centro[1] - my],
        [centro[0] - mx, centro[1] - my],
    ]
}

/// La superficie pasa a ser un rectángulo con esa `proporcion` (ancho / alto): el más
/// grande que entra en la caja de la superficie actual, con el mismo centro. Si la
/// superficie tenía perspectiva, la pierde (queda rectangular).
pub fn con_proporcion(esquinas: &[[f32; 2]; 4], proporcion: f32) -> [[f32; 2]; 4] {
    let (min, max) = caja(esquinas);
    let (ancho, alto) = (max[0] - min[0], max[1] - min[1]);
    let centro = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    if proporcion <= 0.0 || ancho <= 0.0 || alto <= 0.0 {
        return *esquinas;
    }
    // Se prueba a lo ancho de la caja; si no entra en alto, se limita por el alto.
    let (nuevo_ancho, nuevo_alto) = if ancho / proporcion <= alto {
        (ancho, ancho / proporcion)
    } else {
        (alto * proporcion, alto)
    };
    rectangulo_centrado(centro, nuevo_ancho, nuevo_alto)
}

/// La superficie pasa a ser un rectángulo de `tamano` píxeles (ancho, alto), con el mismo
/// centro que la caja de la superficie actual. Así cada píxel del recorte de la entrada
/// ocupa un píxel del proyector.
pub fn con_tamano(esquinas: &[[f32; 2]; 4], tamano: [f32; 2]) -> [[f32; 2]; 4] {
    if tamano[0] <= 0.0 || tamano[1] <= 0.0 {
        return *esquinas;
    }
    let (min, max) = caja(esquinas);
    let centro = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    rectangulo_centrado(centro, tamano[0], tamano[1])
}

/// El centro de la superficie: el promedio de sus cuatro esquinas. Es el mismo punto desde el
/// que la escala achica y agranda.
pub fn centro_de_superficie(esquinas: &[[f32; 2]; 4]) -> [f32; 2] {
    centroide(esquinas)
}

/// La superficie movida para que su centro quede en `nuevo`. Todas las esquinas se desplazan
/// lo mismo, así que la distancia del centro a cada una (la forma y el tamaño) no cambia.
pub fn con_centro(esquinas: &[[f32; 2]; 4], nuevo: [f32; 2]) -> [[f32; 2]; 4] {
    let actual = centro_de_superficie(esquinas);
    let delta = [nuevo[0] - actual[0], nuevo[1] - actual[1]];
    esquinas.map(|e| [e[0] + delta[0], e[1] + delta[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANTALLA: [[f32; 2]; 4] = [
        [-960.0, 540.0],
        [960.0, 540.0],
        [960.0, -540.0],
        [-960.0, -540.0],
    ];

    fn ancho_alto(e: &[[f32; 2]; 4]) -> (f32, f32) {
        (e[1][0] - e[0][0], e[0][1] - e[3][1])
    }

    #[test]
    fn el_tamano_y_la_proporcion_salen_del_recorte() {
        // Mitad del ancho y todo el alto de una composición de 1920×1080.
        let entrada = [0.0, 0.0, 0.5, 1.0];
        assert_eq!(tamano_de_entrada(entrada, 1920.0, 1080.0), [960.0, 1080.0]);
        let p = proporcion_de_entrada(entrada, 1920.0, 1080.0).unwrap();
        assert!((p - 960.0 / 1080.0).abs() < 1e-6);
        assert_eq!(
            proporcion_de_entrada([0.2, 0.2, 0.2, 0.9], 1920.0, 1080.0),
            None
        );
    }

    #[test]
    fn una_proporcion_mas_angosta_deja_el_alto_y_recorta_el_ancho() {
        let r = con_proporcion(&PANTALLA, 1.0); // cuadrado
        let (ancho, alto) = ancho_alto(&r);
        assert!((ancho - 1080.0).abs() < 1e-3 && (alto - 1080.0).abs() < 1e-3);
        // Sigue centrado.
        assert!(((r[0][0] + r[1][0]) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn una_proporcion_mas_ancha_deja_el_ancho_y_recorta_el_alto() {
        let r = con_proporcion(&PANTALLA, 4.0);
        let (ancho, alto) = ancho_alto(&r);
        assert!((ancho - 1920.0).abs() < 1e-3 && (alto - 480.0).abs() < 1e-3);
    }

    #[test]
    fn el_resultado_respeta_el_orden_de_las_esquinas() {
        let r = con_proporcion(&PANTALLA, 1.0);
        assert!(r[0][0] < r[1][0] && r[0][1] > r[3][1]); // sup-izq, sup-der, inf-izq
        assert_eq!(r[1][1], r[0][1]);
        assert_eq!(r[2][0], r[1][0]);
    }

    #[test]
    fn el_tamano_real_se_centra_en_la_superficie_actual() {
        let desplazada = [
            [100.0, 300.0],
            [500.0, 300.0],
            [500.0, 100.0],
            [100.0, 100.0],
        ];
        let r = con_tamano(&desplazada, [200.0, 100.0]);
        assert_eq!(r[0], [200.0, 250.0]);
        assert_eq!(r[2], [400.0, 150.0]);
    }

    #[test]
    fn con_datos_invalidos_no_cambia_nada() {
        assert_eq!(con_proporcion(&PANTALLA, 0.0), PANTALLA);
        assert_eq!(con_tamano(&PANTALLA, [0.0, 10.0]), PANTALLA);
    }

    #[test]
    fn las_medidas_de_una_superficie_rectangular() {
        let (ancho, alto) = medidas_de_superficie(&PANTALLA);
        assert!((ancho - 1920.0).abs() < 1e-3 && (alto - 1080.0).abs() < 1e-3);
    }

    #[test]
    fn con_perspectiva_se_promedian_los_lados_opuestos() {
        // Arriba mide 1000, abajo 2000: el ancho es 1500.
        let trapecio = [
            [-500.0, 100.0],
            [500.0, 100.0],
            [1000.0, -100.0],
            [-1000.0, -100.0],
        ];
        let (ancho, _) = medidas_de_superficie(&trapecio);
        assert!((ancho - 1500.0).abs() < 1e-3, "{ancho}");
    }

    #[test]
    fn el_centro_es_el_promedio_de_las_esquinas() {
        assert_eq!(centro_de_superficie(&PANTALLA), [0.0, 0.0]);
        let corrida = PANTALLA.map(|e| [e[0] + 100.0, e[1] - 30.0]);
        assert_eq!(centro_de_superficie(&corrida), [100.0, -30.0]);
    }

    #[test]
    fn mover_el_centro_no_cambia_las_distancias_a_las_esquinas() {
        // Un trapecio, para que no sea simétrico.
        let trapecio = [
            [-300.0, 200.0],
            [250.0, 180.0],
            [400.0, -220.0],
            [-380.0, -150.0],
        ];
        let antes = centro_de_superficie(&trapecio);
        let distancias =
            |e: &[[f32; 2]; 4], c: [f32; 2]| e.map(|p| (p[0] - c[0]).hypot(p[1] - c[1]));
        let d0 = distancias(&trapecio, antes);

        let movida = con_centro(&trapecio, [0.0, 0.0]);
        let c = centro_de_superficie(&movida);
        assert!(c[0].abs() < 1e-3 && c[1].abs() < 1e-3, "{c:?}");
        for (a, b) in d0.iter().zip(distancias(&movida, c)) {
            assert!((a - b).abs() < 1e-3, "{a} {b}");
        }
        // Y las medidas (ancho y alto) tampoco cambian.
        let (a0, h0) = medidas_de_superficie(&trapecio);
        let (a1, h1) = medidas_de_superficie(&movida);
        assert!((a0 - a1).abs() < 1e-3 && (h0 - h1).abs() < 1e-3);
    }
}
