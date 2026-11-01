//! Curvas de suavizado para las animaciones de la interfaz. Matemática pura.
//!
//! Una animación avanza con un tiempo `t` de 0 a 1; la curva decide cómo: una transición
//! lineal se siente mecánica, y una que arranca y frena suave se siente natural.

/// Limita `t` a 0..1.
pub(crate) fn acotar(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Arranca y frena suave (para ir de un valor a otro).
pub(crate) fn entrada_salida_cubica(t: f32) -> f32 {
    let t = acotar(t);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// El progreso (0..1) de una animación que empezó en `inicio` y dura `duracion` segundos,
/// vista en el instante `ahora`. Una duración nula termina enseguida.
pub(crate) fn progreso(inicio: f64, ahora: f64, duracion: f64) -> f32 {
    if duracion <= 0.0 {
        return 1.0;
    }
    acotar(((ahora - inicio) / duracion) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn casi(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn la_curva_empieza_en_cero_y_termina_en_uno() {
        let curva = entrada_salida_cubica;
        assert!(casi(curva(0.0), 0.0));
        assert!(casi(curva(1.0), 1.0));
        // Fuera de rango se acota.
        assert!(casi(curva(-3.0), 0.0));
        assert!(casi(curva(7.0), 1.0));
    }

    #[test]
    fn la_curva_nunca_baja() {
        let mut anterior = 0.0;
        for i in 0..=100 {
            let v = entrada_salida_cubica(i as f32 / 100.0);
            assert!(v >= anterior - 1e-6, "baja en {i}");
            anterior = v;
        }
    }

    #[test]
    fn la_curva_es_simetrica_a_mitad_de_tiempo() {
        assert!(casi(entrada_salida_cubica(0.5), 0.5));
    }

    #[test]
    fn el_progreso_va_de_0_a_1_en_la_duracion() {
        assert!(casi(progreso(10.0, 10.0, 2.0), 0.0));
        assert!(casi(progreso(10.0, 11.0, 2.0), 0.5));
        assert!(casi(progreso(10.0, 99.0, 2.0), 1.0));
        assert!(casi(progreso(10.0, 5.0, 2.0), 0.0)); // antes de empezar
        assert!(casi(progreso(10.0, 10.0, 0.0), 1.0)); // duración nula
    }
}
