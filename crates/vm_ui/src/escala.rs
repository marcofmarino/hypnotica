//! La escala de una superficie en el editor de la salida.
//!
//! Matemática pura (solo arrays y texto): se prueba con tests, sin egui ni Bevy.
//!
//! El porcentaje que se muestra es **relativo a un tamaño base**: cómo estaba la superficie
//! cuando se empezó a escalar. Así el campo muestra un valor real que persiste (si ponés 50
//! y después 100, volvés al tamaño de partida), en lugar de reiniciarse a 100 en cada
//! cambio. Si la superficie se modifica por otro lado (se mueve una esquina, por ejemplo),
//! la base ya no vale y se renueva.

use vm_core::{Malla, centroide, escalar};

/// Los límites del porcentaje, para no dejar una superficie diminuta ni gigantesca.
pub const PORCENTAJE_MINIMO: f32 = 1.0;
pub const PORCENTAJE_MAXIMO: f32 = 1000.0;

/// Cuánto pueden diferir (en unidades de la salida) las esquinas actuales de las esperadas
/// para seguir considerando que la base vale. Cubre el error de los decimales.
const TOLERANCIA: f32 = 0.05;

/// El cuadrilátero `base` cambiado de tamaño `factor` veces desde su centro.
pub fn escalar_cuadrilatero(base: [[f32; 2]; 4], factor: f32) -> [[f32; 2]; 4] {
    let t = escalar(&base, centroide(&base), factor);
    [t[0], t[1], t[2], t[3]]
}

/// El tamaño base de una superficie y el porcentaje que se le aplicó.
///
/// Si la superficie tiene malla, sus desplazamientos también se escalan: la deformación
/// crece o se achica con la superficie.
#[derive(Debug, Clone, PartialEq)]
pub struct EscalaBase {
    pub id: String,
    pub superficie: usize,
    pub base: [[f32; 2]; 4],
    pub malla_base: Option<Malla>,
    pub porcentaje: f32,
}

impl EscalaBase {
    /// Una base nueva: la superficie tal como está, a 100 %.
    pub fn nueva(
        id: String,
        superficie: usize,
        base: [[f32; 2]; 4],
        malla: Option<&Malla>,
    ) -> Self {
        EscalaBase {
            id,
            superficie,
            base,
            malla_base: malla.cloned(),
            porcentaje: 100.0,
        }
    }

    /// Las esquinas que corresponden al porcentaje actual.
    pub fn esquinas(&self) -> [[f32; 2]; 4] {
        escalar_cuadrilatero(self.base, self.porcentaje / 100.0)
    }

    /// La malla que corresponde al porcentaje actual (si la superficie tiene una).
    pub fn malla_escalada(&self) -> Option<Malla> {
        self.malla_base
            .as_ref()
            .map(|m| m.escalada(self.porcentaje / 100.0))
    }

    /// ¿Esta base sigue valiendo para esa superficie y esas esquinas? Deja de valer si se
    /// cambió de superficie o si las esquinas ya no son las que producía el porcentaje
    /// (alguien las movió).
    pub fn vale_para(
        &self,
        id: &str,
        superficie: usize,
        actuales: &[[f32; 2]; 4],
        malla_actual: Option<&Malla>,
    ) -> bool {
        self.id == id
            && self.superficie == superficie
            && self.malla_coincide(malla_actual)
            && self
                .esquinas()
                .iter()
                .zip(actuales)
                .all(|(a, b)| (a[0] - b[0]).abs() < TOLERANCIA && (a[1] - b[1]).abs() < TOLERANCIA)
    }
}

impl EscalaBase {
    /// ¿La malla actual es la que produciría el porcentaje (o ambas faltan)? Si se movió un
    /// punto de control, ya no lo es y la base deja de valer.
    fn malla_coincide(&self, actual: Option<&Malla>) -> bool {
        match (self.malla_escalada(), actual) {
            (None, None) => true,
            (Some(esperada), Some(actual)) => {
                esperada.columnas == actual.columnas
                    && esperada.filas == actual.filas
                    && esperada
                        .desplazamientos
                        .iter()
                        .zip(&actual.desplazamientos)
                        .all(|(a, b)| {
                            (a[0] - b[0]).abs() < TOLERANCIA && (a[1] - b[1]).abs() < TOLERANCIA
                        })
            }
            _ => false,
        }
    }
}

/// Lee un porcentaje escrito por una persona: acepta `90`, `90%`, ` 90 % ` y `90,5`.
/// Devuelve `None` si no es un número, o si no es positivo. El resultado se limita a los
/// extremos permitidos.
pub fn interpretar_porcentaje(texto: &str) -> Option<f32> {
    let limpio = texto.trim().trim_end_matches('%').trim().replace(',', ".");
    let valor: f32 = limpio.parse().ok()?;
    if !valor.is_finite() || valor <= 0.0 {
        return None;
    }
    Some(valor.clamp(PORCENTAJE_MINIMO, PORCENTAJE_MAXIMO))
}

/// El porcentaje como texto: sin decimales si es (casi) entero, con uno si no.
pub fn formatear_porcentaje(porcentaje: f32) -> String {
    if (porcentaje - porcentaje.round()).abs() < 0.05 {
        format!("{:.0}", porcentaje)
    } else {
        format!("{:.1}", porcentaje)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: [[f32; 2]; 4] = [
        [-100.0, 50.0],
        [100.0, 50.0],
        [100.0, -50.0],
        [-100.0, -50.0],
    ];

    #[test]
    fn el_texto_se_interpreta_con_variantes() {
        assert_eq!(interpretar_porcentaje("90"), Some(90.0));
        assert_eq!(interpretar_porcentaje("90%"), Some(90.0));
        assert_eq!(interpretar_porcentaje("  90 % "), Some(90.0));
        assert_eq!(interpretar_porcentaje("90,5"), Some(90.5));
        assert_eq!(interpretar_porcentaje("90.5"), Some(90.5));
    }

    #[test]
    fn un_texto_invalido_no_se_interpreta() {
        for malo in ["", "abc", "-5", "0", "%", "9 0", "NaN", "inf"] {
            assert_eq!(interpretar_porcentaje(malo), None, "{malo:?}");
        }
    }

    #[test]
    fn el_porcentaje_se_limita_a_los_extremos() {
        assert_eq!(interpretar_porcentaje("0,2"), Some(PORCENTAJE_MINIMO));
        assert_eq!(interpretar_porcentaje("99999"), Some(PORCENTAJE_MAXIMO));
    }

    #[test]
    fn se_formatea_sin_decimales_cuando_es_entero() {
        assert_eq!(formatear_porcentaje(100.0), "100");
        assert_eq!(formatear_porcentaje(90.0), "90");
        assert_eq!(formatear_porcentaje(90.5), "90.5");
        assert_eq!(formatear_porcentaje(99.99), "100");
    }

    #[test]
    fn la_base_nueva_arranca_en_100_y_reproduce_la_superficie() {
        let b = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        assert_eq!(b.porcentaje, 100.0);
        assert!(b.vale_para("DP-1", 0, &BASE, None));
    }

    #[test]
    fn escalar_respecto_de_la_base_es_idempotente_y_reversible() {
        let mut b = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        b.porcentaje = 50.0;
        let mitad = b.esquinas();
        assert_eq!(mitad[1], [50.0, 25.0]);
        // Poner de nuevo el mismo valor da lo mismo (no acumula).
        b.porcentaje = 50.0;
        assert_eq!(b.esquinas(), mitad);
        // Y volver a 100 devuelve exactamente la superficie de partida.
        b.porcentaje = 100.0;
        assert_eq!(b.esquinas(), BASE);
    }

    #[test]
    fn la_escala_conserva_el_centro_y_las_proporciones() {
        let mut b = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        b.porcentaje = 150.0;
        let e = b.esquinas();
        assert_eq!(centroide(&e), centroide(&BASE));
        // El ancho y el alto crecieron lo mismo.
        assert!(((e[1][0] - e[0][0]) / 200.0 - 1.5).abs() < 1e-5);
        assert!(((e[0][1] - e[3][1]) / 100.0 - 1.5).abs() < 1e-5);
    }

    #[test]
    fn la_base_sigue_valiendo_tras_aplicar_su_propia_escala() {
        let mut b = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        b.porcentaje = 73.0;
        assert!(b.vale_para("DP-1", 0, &b.esquinas(), None));
    }

    #[test]
    fn la_malla_se_escala_junto_con_la_superficie() {
        let mut malla = Malla::nueva(3, 3);
        malla.poner(1, 1, [20.0, -10.0]);
        let mut b = EscalaBase::nueva("DP-1".into(), 0, BASE, Some(&malla));
        b.porcentaje = 50.0;
        let escalada = b.malla_escalada().unwrap();
        assert_eq!(escalada.desplazamiento(1, 1), [10.0, -5.0]);
        // Con la malla escalada y las esquinas escaladas, la base sigue valiendo.
        assert!(b.vale_para("DP-1", 0, &b.esquinas(), Some(&escalada)));
        // Si se mueve un punto de control, deja de valer.
        let mut movida = escalada.clone();
        movida.poner(1, 1, [99.0, 99.0]);
        assert!(!b.vale_para("DP-1", 0, &b.esquinas(), Some(&movida)));
        // Y si la superficie pasó a no tener malla (o al revés).
        assert!(!b.vale_para("DP-1", 0, &b.esquinas(), None));
        let sin_malla = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        assert!(!sin_malla.vale_para("DP-1", 0, &BASE, Some(&malla)));
    }

    #[test]
    fn la_base_deja_de_valer_si_se_movio_una_esquina_o_se_cambio_de_superficie() {
        let mut b = EscalaBase::nueva("DP-1".into(), 0, BASE, None);
        b.porcentaje = 80.0;
        let mut movidas = b.esquinas();
        movidas[2][0] += 10.0;
        assert!(!b.vale_para("DP-1", 0, &movidas, None));
        assert!(!b.vale_para("DP-1", 1, &b.esquinas(), None)); // otra superficie
        assert!(!b.vale_para("HDMI-A-1", 0, &b.esquinas(), None)); // otra salida
    }
}
