//! La deformación completa de una superficie: el corner pin (homografía) más, si la tiene,
//! su malla de puntos de control. Rust puro.
//!
//! Un punto `(u, v)` de la superficie (de 0 a 1) cae en pantalla en
//! `homografía(u, v) + desplazamiento_de_la_malla(u, v)`. El editor necesita también el
//! camino inverso: dado un punto de la pantalla, qué punto de la superficie hay ahí.

use vm_core::Malla;

use crate::homografia::Homografia;

/// Dónde cae en pantalla el punto `(u, v)` de la superficie.
pub fn proyectar(h: &Homografia, malla: Option<&Malla>, u: f32, v: f32) -> [f32; 2] {
    let [x, y] = h.proyectar(u, v);
    match malla {
        Some(m) => {
            let d = m.desplazamiento_en(u, v);
            [x + d[0], y + d[1]]
        }
        None => [x, y],
    }
}

/// Cuántas veces se repite el cálculo de `desproyectar`. Alcanza de sobra para una malla
/// que cambia de forma de manera suave.
const ITERACIONES: usize = 12;

/// Lo inverso de `proyectar`: qué punto `(u, v)` de la superficie está en `(x, y)` de la
/// pantalla.
///
/// Sin malla es exacto (la homografía inversa). Con malla no hay una fórmula cerrada, así
/// que se **aproxima por iteración**: se parte de la respuesta sin malla y, en cada vuelta,
/// se descuenta el desplazamiento que tendría el punto encontrado. Converge cuando la malla
/// cambia suavemente, que es lo que hace la interpolación Catmull-Rom.
pub fn desproyectar(h: &Homografia, malla: Option<&Malla>, x: f32, y: f32) -> [f32; 2] {
    let Some(m) = malla else {
        return h.desproyectar(x, y);
    };
    let mut uv = h.desproyectar(x, y);
    for _ in 0..ITERACIONES {
        let d = m.desplazamiento_en(uv[0], uv[1]);
        uv = h.desproyectar(x - d[0], y - d[1]);
    }
    uv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn homografia() -> Homografia {
        // Un trapecio con perspectiva, en píxeles de una pantalla grande.
        Homografia::desde_esquinas([
            [-700.0, 400.0],
            [900.0, 250.0],
            [650.0, -420.0],
            [-820.0, -300.0],
        ])
    }

    fn malla_deformada() -> Malla {
        let mut m = Malla::nueva(5, 4);
        m.poner(2, 1, [60.0, -35.0]);
        m.poner(1, 2, [-40.0, 25.0]);
        m.poner(3, 2, [30.0, 50.0]);
        m.poner(2, 0, [0.0, 45.0]);
        m
    }

    fn casi(a: [f32; 2], b: [f32; 2], tolerancia: f32) -> bool {
        (a[0] - b[0]).abs() < tolerancia && (a[1] - b[1]).abs() < tolerancia
    }

    #[test]
    fn sin_malla_es_la_homografia() {
        let h = homografia();
        for (u, v) in [(0.0, 0.0), (0.3, 0.7), (1.0, 1.0)] {
            assert_eq!(proyectar(&h, None, u, v), h.proyectar(u, v));
        }
    }

    #[test]
    fn una_malla_sin_deformar_no_cambia_nada() {
        let h = homografia();
        let m = Malla::nueva(4, 4);
        for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.9, 0.8)] {
            assert!(casi(proyectar(&h, Some(&m), u, v), h.proyectar(u, v), 1e-3));
        }
    }

    #[test]
    fn las_esquinas_de_la_superficie_siguen_donde_las_pone_el_corner_pin() {
        let h = homografia();
        let m = malla_deformada();
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            assert!(
                casi(proyectar(&h, Some(&m), u, v), h.proyectar(u, v), 1e-3),
                "({u},{v})"
            );
        }
    }

    #[test]
    fn un_punto_de_control_desplazado_se_ve_en_su_lugar_mas_el_desplazamiento() {
        let h = homografia();
        let m = malla_deformada();
        let [u, v] = m.uv_de_nodo(2, 1);
        let base = h.proyectar(u, v);
        let con_malla = proyectar(&h, Some(&m), u, v);
        assert!(casi(con_malla, [base[0] + 60.0, base[1] - 35.0], 1e-2));
    }

    #[test]
    fn desproyectar_deshace_a_proyectar_con_malla() {
        let h = homografia();
        let m = malla_deformada();
        for (u, v) in [
            (0.5, 0.33),
            (0.25, 0.5),
            (0.75, 0.66),
            (0.5, 0.5),
            (0.1, 0.9),
            (0.9, 0.1),
        ] {
            let [x, y] = proyectar(&h, Some(&m), u, v);
            let [ru, rv] = desproyectar(&h, Some(&m), x, y);
            assert!(
                (ru - u).abs() < 2e-3 && (rv - v).abs() < 2e-3,
                "({u},{v}) -> ({x},{y}) -> ({ru},{rv})"
            );
        }
    }

    #[test]
    fn desproyectar_sin_malla_es_exacto() {
        let h = homografia();
        let [x, y] = h.proyectar(0.4, 0.6);
        assert!(casi(desproyectar(&h, None, x, y), [0.4, 0.6], 1e-3));
    }
}
