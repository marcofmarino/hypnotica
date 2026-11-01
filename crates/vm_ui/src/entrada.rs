//! Edición del rectángulo de entrada: qué parte de la composición muestra una superficie.
//!
//! Matemática pura (solo arrays): se prueba con tests, sin egui ni Bevy.
//! El rectángulo es `[x_min, y_min, x_max, y_max]` con valores de 0 a 1 y la `y` hacia
//! abajo, igual que `vm_core::Entrada`.

use vm_core::{Entrada, normalizar_entrada};

/// Las cuatro esquinas del rectángulo, en el orden: sup-izq, sup-der, inf-der, inf-izq.
pub fn esquinas(r: Entrada) -> [[f32; 2]; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

/// Mueve una esquina del rectángulo a `punto`, dejando fija la esquina opuesta.
/// `esquina` sigue el orden de `esquinas`. El resultado queda normalizado: si se arrastra
/// más allá de la opuesta, el rectángulo se "da vuelta"; y nunca sale de 0..1.
pub fn mover_esquina(r: Entrada, esquina: usize, punto: [f32; 2]) -> Entrada {
    let fija = esquinas(r)[(esquina + 2) % 4];
    normalizar_entrada([
        punto[0].min(fija[0]),
        punto[1].min(fija[1]),
        punto[0].max(fija[0]),
        punto[1].max(fija[1]),
    ])
}

/// Mueve varias esquinas del rectángulo a la vez, `delta` (en coordenadas de la composición).
/// Las esquinas siguen el orden de `esquinas`: 0 sup-izq, 1 sup-der, 2 inf-der, 3 inf-izq.
///
/// Cada lado del rectángulo se mueve si alguna de las esquinas elegidas está en él: con una
/// esquina cambian dos lados (se redimensiona); con las dos de un lado se mueve ese lado
/// (y, en el otro eje, el rectángulo se traslada porque hay esquinas a ambos lados); y con
/// todas se traslada el rectángulo entero sin deformarlo. El resultado queda normalizado.
pub fn mover_esquinas_elegidas(r: Entrada, elegidas: &[usize], delta: [f32; 2]) -> Entrada {
    if (0..4).all(|i| elegidas.contains(&i)) {
        return mover_rectangulo(r, delta);
    }
    let hay = |indices: [usize; 2]| elegidas.iter().any(|e| indices.contains(e));
    let mut nuevo = r;
    if hay([0, 3]) {
        nuevo[0] += delta[0]; // lado izquierdo
    }
    if hay([1, 2]) {
        nuevo[2] += delta[0]; // lado derecho
    }
    if hay([0, 1]) {
        nuevo[1] += delta[1]; // lado de arriba
    }
    if hay([2, 3]) {
        nuevo[3] += delta[1]; // lado de abajo
    }
    normalizar_entrada(nuevo)
}

/// Desplaza todo el rectángulo en `delta`, sin cambiar su tamaño y sin salirse de 0..1.
pub fn mover_rectangulo(r: Entrada, delta: [f32; 2]) -> Entrada {
    let (ancho, alto) = (r[2] - r[0], r[3] - r[1]);
    let x0 = (r[0] + delta[0]).clamp(0.0, 1.0 - ancho);
    let y0 = (r[1] + delta[1]).clamp(0.0, 1.0 - alto);
    [x0, y0, x0 + ancho, y0 + alto]
}

/// ¿El punto está dentro del rectángulo?
pub fn contiene(r: Entrada, punto: [f32; 2]) -> bool {
    punto[0] >= r[0] && punto[0] <= r[2] && punto[1] >= r[1] && punto[1] <= r[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Entrada = [0.2, 0.3, 0.6, 0.7];

    fn casi(a: Entrada, b: Entrada) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn las_esquinas_siguen_el_orden_esperado() {
        let e = esquinas(R);
        assert_eq!(e[0], [0.2, 0.3]); // sup-izq
        assert_eq!(e[2], [0.6, 0.7]); // inf-der
    }

    #[test]
    fn mover_la_sup_izq_deja_fija_la_inf_der() {
        let r = mover_esquina(R, 0, [0.1, 0.0]);
        assert!(casi(r, [0.1, 0.0, 0.6, 0.7]));
    }

    #[test]
    fn mover_la_inf_der_deja_fija_la_sup_izq() {
        let r = mover_esquina(R, 2, [0.9, 1.0]);
        assert!(casi(r, [0.2, 0.3, 0.9, 1.0]));
    }

    #[test]
    fn arrastrar_mas_alla_de_la_opuesta_da_vuelta_el_rectangulo() {
        // La sup-izq se arrastra hacia la derecha de la inf-der: el rectángulo se invierte.
        let r = mover_esquina(R, 0, [0.9, 0.9]);
        assert!(casi(r, [0.6, 0.7, 0.9, 0.9]));
    }

    #[test]
    fn mover_una_esquina_fuera_de_la_composicion_se_limita() {
        let r = mover_esquina(R, 1, [1.5, -0.4]);
        assert!(casi(r, [0.2, 0.0, 1.0, 0.7]));
    }

    #[test]
    fn mover_el_rectangulo_conserva_su_tamano() {
        let r = mover_rectangulo(R, [0.1, -0.1]);
        assert!(casi(r, [0.3, 0.2, 0.7, 0.6]));
    }

    #[test]
    fn mover_el_rectangulo_no_se_sale_de_la_composicion() {
        let r = mover_rectangulo(R, [5.0, -5.0]);
        assert!(casi(r, [0.6, 0.0, 1.0, 0.4]));
    }

    #[test]
    fn contiene_distingue_dentro_y_fuera() {
        assert!(contiene(R, [0.4, 0.5]));
        assert!(!contiene(R, [0.7, 0.5]));
    }

    #[test]
    fn dos_esquinas_del_mismo_lado_mueven_ese_lado() {
        // Las esquinas 1 y 2 son el lado derecho: solo se mueve ese lado.
        let r = mover_esquinas_elegidas(R, &[1, 2], [0.1, 0.0]);
        assert!(casi(r, [0.2, 0.3, 0.7, 0.7]), "{r:?}");
        // Las esquinas 0 y 1 son el lado de arriba: sube ese lado y, como las dos esquinas
        // están a ambos lados en `x`, el movimiento horizontal traslada el ancho entero.
        let r = mover_esquinas_elegidas(R, &[0, 1], [0.1, -0.1]);
        assert!(casi(r, [0.3, 0.2, 0.7, 0.7]), "{r:?}");
    }

    #[test]
    fn dos_esquinas_opuestas_trasladan_el_rectangulo() {
        // Con 0 y 2 se tocan los cuatro lados: el rectángulo se traslada.
        let r = mover_esquinas_elegidas(R, &[0, 2], [0.1, 0.1]);
        assert!(casi(r, [0.3, 0.4, 0.7, 0.8]), "{r:?}");
    }

    #[test]
    fn las_cuatro_esquinas_trasladan_el_rectangulo_sin_deformarlo() {
        let r = mover_esquinas_elegidas(R, &[0, 1, 2, 3], [0.1, -0.1]);
        assert!(casi(r, [0.3, 0.2, 0.7, 0.6]), "{r:?}");
        // Contra el borde se frena sin cambiar de tamaño.
        let r = mover_esquinas_elegidas(R, &[0, 1, 2, 3], [5.0, 0.0]);
        assert!(casi(r, [0.6, 0.3, 1.0, 0.7]), "{r:?}");
    }

    #[test]
    fn sin_esquinas_elegidas_no_cambia_nada() {
        assert!(casi(mover_esquinas_elegidas(R, &[], [0.3, 0.3]), R));
    }
}
