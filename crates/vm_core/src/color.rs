//! Calibración de color de una salida (un proyector): ganancia por canal, brillo, contraste
//! y gamma. Rust puro. Sirve para que dos proyectores distintos muestren lo mismo.
//!
//! Se aplica a cada color **en espacio sRGB** (el de los valores que se ven y se miden, no
//! el lineal): primero la ganancia de cada canal, después contraste y brillo y al final el
//! gamma. El shader (`superficie.wesl`) hace exactamente la misma cuenta.

use serde::{Deserialize, Serialize};

/// Límites de cada ajuste (la interfaz y `normalizar` los respetan).
pub const GANANCIA_MINIMA: f32 = 0.0;
pub const GANANCIA_MAXIMA: f32 = 2.0;
pub const BRILLO_MINIMO: f32 = -0.5;
pub const BRILLO_MAXIMO: f32 = 0.5;
pub const CONTRASTE_MINIMO: f32 = 0.0;
pub const CONTRASTE_MAXIMO: f32 = 2.0;
pub const GAMMA_MINIMO: f32 = 0.25;
pub const GAMMA_MAXIMO: f32 = 4.0;

/// La corrección de color de una salida. El valor por defecto no cambia nada.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorreccionColor {
    /// Cuánto se multiplica cada canal (1 = sin cambio).
    pub rojo: f32,
    pub verde: f32,
    pub azul: f32,
    /// Se suma a todos los canales (0 = sin cambio): sube o baja el nivel de negro y de todo.
    pub brillo: f32,
    /// Separa o junta los tonos alrededor del gris medio (1 = sin cambio).
    pub contraste: f32,
    /// Curva de los tonos medios (1 = sin cambio; más de 1 los aclara).
    pub gamma: f32,
}

impl Default for CorreccionColor {
    fn default() -> Self {
        CorreccionColor {
            rojo: 1.0,
            verde: 1.0,
            azul: 1.0,
            brillo: 0.0,
            contraste: 1.0,
            gamma: 1.0,
        }
    }
}

/// Un valor, o `neutro` si no es un número; siempre dentro de `[minimo, maximo]`.
fn limitar(valor: f32, minimo: f32, maximo: f32, neutro: f32) -> f32 {
    if valor.is_finite() {
        valor.clamp(minimo, maximo)
    } else {
        neutro
    }
}

impl CorreccionColor {
    /// Deja todos los ajustes dentro de sus límites (un archivo editado a mano puede traer
    /// cualquier cosa); lo que no sea un número vuelve a su valor neutro.
    pub fn normalizar(&mut self) {
        self.rojo = limitar(self.rojo, GANANCIA_MINIMA, GANANCIA_MAXIMA, 1.0);
        self.verde = limitar(self.verde, GANANCIA_MINIMA, GANANCIA_MAXIMA, 1.0);
        self.azul = limitar(self.azul, GANANCIA_MINIMA, GANANCIA_MAXIMA, 1.0);
        self.brillo = limitar(self.brillo, BRILLO_MINIMO, BRILLO_MAXIMO, 0.0);
        self.contraste = limitar(self.contraste, CONTRASTE_MINIMO, CONTRASTE_MAXIMO, 1.0);
        self.gamma = limitar(self.gamma, GAMMA_MINIMO, GAMMA_MAXIMO, 1.0);
    }

    /// ¿No cambia nada?
    pub fn es_neutra(&self) -> bool {
        *self == CorreccionColor::default()
    }

    /// La corrección aplicada a un color sRGB (cada canal de 0 a 1). Es la misma cuenta que
    /// hace el shader, y sirve para comprobarlo.
    pub fn aplicar(&self, srgb: [f32; 3]) -> [f32; 3] {
        let ganancias = [self.rojo, self.verde, self.azul];
        let mut salida = [0.0; 3];
        for i in 0..3 {
            let con_ganancia = srgb[i] * ganancias[i];
            let con_contraste = (con_ganancia - 0.5) * self.contraste + 0.5 + self.brillo;
            salida[i] = con_contraste.clamp(0.0, 1.0).powf(1.0 / self.gamma);
        }
        salida
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn casi(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn la_correccion_por_defecto_no_cambia_ningun_color() {
        let c = CorreccionColor::default();
        assert!(c.es_neutra());
        for color in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.2, 0.5, 0.9]] {
            assert!(casi(c.aplicar(color), color), "{color:?}");
        }
    }

    #[test]
    fn la_ganancia_multiplica_cada_canal_por_separado() {
        let c = CorreccionColor {
            rojo: 0.5,
            azul: 2.0,
            ..Default::default()
        };
        assert!(casi(c.aplicar([0.8, 0.8, 0.3]), [0.4, 0.8, 0.6]));
        // Se frena en el blanco.
        assert!(casi(c.aplicar([1.0, 1.0, 0.9]), [0.5, 1.0, 1.0]));
    }

    #[test]
    fn el_brillo_sube_o_baja_todo_parejo() {
        let c = CorreccionColor {
            brillo: 0.1,
            ..Default::default()
        };
        assert!(casi(c.aplicar([0.0, 0.5, 0.95]), [0.1, 0.6, 1.0]));
        let c = CorreccionColor {
            brillo: -0.2,
            ..Default::default()
        };
        assert!(casi(c.aplicar([0.1, 0.5, 1.0]), [0.0, 0.3, 0.8]));
    }

    #[test]
    fn el_contraste_gira_los_tonos_alrededor_del_gris_medio() {
        let c = CorreccionColor {
            contraste: 2.0,
            ..Default::default()
        };
        assert!(casi(c.aplicar([0.5, 0.5, 0.5]), [0.5, 0.5, 0.5])); // el medio no se mueve
        assert!(casi(c.aplicar([0.4, 0.6, 0.25]), [0.3, 0.7, 0.0]));
        let c = CorreccionColor {
            contraste: 0.0,
            ..Default::default()
        };
        assert!(casi(c.aplicar([0.0, 1.0, 0.3]), [0.5, 0.5, 0.5])); // todo gris
    }

    #[test]
    fn el_gamma_mueve_los_medios_sin_tocar_el_negro_ni_el_blanco() {
        let c = CorreccionColor {
            gamma: 2.0,
            ..Default::default()
        };
        let r = c.aplicar([0.0, 0.25, 1.0]);
        assert!(casi(r, [0.0, 0.5, 1.0]), "{r:?}"); // 0,25^(1/2) = 0,5
    }

    #[test]
    fn normalizar_limita_y_arregla_lo_que_no_es_numero() {
        let mut c = CorreccionColor {
            rojo: 9.0,
            verde: f32::NAN,
            azul: -3.0,
            brillo: 4.0,
            contraste: f32::INFINITY,
            gamma: 0.0,
        };
        c.normalizar();
        assert_eq!(c.rojo, GANANCIA_MAXIMA);
        assert_eq!(c.verde, 1.0);
        assert_eq!(c.azul, GANANCIA_MINIMA);
        assert_eq!(c.brillo, BRILLO_MAXIMO);
        assert_eq!(c.contraste, 1.0);
        assert_eq!(c.gamma, GAMMA_MINIMO);
    }

    #[test]
    fn un_archivo_sin_correccion_se_lee_como_neutra() {
        let c: CorreccionColor = ron::from_str("(brillo: 0.1)").unwrap();
        assert_eq!(c.brillo, 0.1);
        assert_eq!(c.rojo, 1.0);
        assert_eq!(c.gamma, 1.0);
    }
}
