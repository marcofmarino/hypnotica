//! Modulación: que una **señal** (el sonido, y más adelante un mensaje de OSC o de MIDI) mueva
//! un **parámetro** (hoy la opacidad de una capa), con un deslizador de cuánto reacciona. Es la
//! base común de la reacción al audio (`docs/05`) y del control externo.
//!
//! Cada modulación lleva **su propia fuente de sonido**: dos parámetros pueden escuchar cosas
//! distintas (una capa el escritorio y otra un clip, por ejemplo).
//!
//! Rust puro: las señales llegan ya medidas, de 0 a 1.

use serde::{Deserialize, Serialize};

use crate::audio::Senales;

/// De dónde sale el sonido que se analiza.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FuenteDeAudio {
    #[default]
    Ninguna,
    /// Todo lo que suena en el escritorio.
    Escritorio,
    /// La entrada de sonido por defecto del sistema (el micrófono).
    Microfono,
    /// Lo que emite una aplicación, por su nombre (`application.name` de PipeWire).
    Aplicacion(String),
    /// El sonido del clip que suena en una capa (el índice desde 0). Se toma de adentro del
    /// reproductor, antes del volumen: sirve aunque el clip esté silenciado. Sigue al **índice**
    /// de la capa, no a la capa: si se reordenan, hay que volver a elegirla.
    Capa(usize),
}

impl FuenteDeAudio {
    /// Cómo se escribe en un texto: `ninguna`, `escritorio`, `microfono`, `aplicacion:Nombre` o
    /// `capa:N` (desde 1, como en la interfaz).
    pub fn a_texto(&self) -> String {
        match self {
            FuenteDeAudio::Ninguna => "ninguna".into(),
            FuenteDeAudio::Escritorio => "escritorio".into(),
            FuenteDeAudio::Microfono => "microfono".into(),
            FuenteDeAudio::Aplicacion(nombre) => format!("aplicacion:{nombre}"),
            FuenteDeAudio::Capa(i) => format!("capa:{}", i + 1),
        }
    }

    /// Lo inverso de `a_texto`. Un texto que no se entiende vale «ninguna».
    pub fn desde_texto(texto: &str) -> Self {
        let texto = texto.trim();
        match texto {
            "escritorio" => FuenteDeAudio::Escritorio,
            "microfono" => FuenteDeAudio::Microfono,
            _ => match texto.split_once(':') {
                Some(("aplicacion", nombre)) if !nombre.trim().is_empty() => {
                    FuenteDeAudio::Aplicacion(nombre.trim().to_string())
                }
                Some(("capa", n)) => n
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .map_or(FuenteDeAudio::Ninguna, FuenteDeAudio::Capa),
                _ => FuenteDeAudio::Ninguna,
            },
        }
    }

    /// El nombre para mostrar en la interfaz.
    pub fn nombre(&self) -> String {
        match self {
            FuenteDeAudio::Ninguna => "Ninguna".into(),
            FuenteDeAudio::Escritorio => "Escritorio".into(),
            FuenteDeAudio::Microfono => "Micrófono".into(),
            FuenteDeAudio::Aplicacion(nombre) => nombre.clone(),
            FuenteDeAudio::Capa(i) => format!("Capa {}", i + 1),
        }
    }
}

/// Qué señal mueve el parámetro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OrigenDeSenal {
    /// El volumen general.
    #[default]
    Nivel,
    Graves,
    Medios,
    Agudos,
}

impl OrigenDeSenal {
    pub const TODOS: [OrigenDeSenal; 4] = [
        OrigenDeSenal::Nivel,
        OrigenDeSenal::Graves,
        OrigenDeSenal::Medios,
        OrigenDeSenal::Agudos,
    ];

    pub fn nombre(&self) -> &'static str {
        match self {
            OrigenDeSenal::Nivel => "Nivel",
            OrigenDeSenal::Graves => "Graves",
            OrigenDeSenal::Medios => "Medios",
            OrigenDeSenal::Agudos => "Agudos",
        }
    }

    /// El valor de esta señal (de 0 a 1) en las `senales` dadas.
    pub fn valor(&self, senales: &Senales) -> f32 {
        match self {
            OrigenDeSenal::Nivel => senales.nivel,
            OrigenDeSenal::Graves => senales.graves,
            OrigenDeSenal::Medios => senales.medios,
            OrigenDeSenal::Agudos => senales.agudos,
        }
    }
}

/// Los proyectos de antes (formato 21) no guardaban la fuente: se asume el escritorio.
fn fuente_por_defecto() -> FuenteDeAudio {
    FuenteDeAudio::Escritorio
}

/// De dónde sale el sonido, qué señal se toma y cuánto mueve un parámetro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Modulacion {
    /// El sonido que se escucha. Cada modulación elige el suyo.
    #[serde(default = "fuente_por_defecto")]
    pub fuente: FuenteDeAudio,
    pub senal: OrigenDeSenal,
    /// Cuánto suma la señal al valor del parámetro cuando está a fondo, de −1 a 1. Con
    /// valores negativos, la señal lo **baja** (por ejemplo, un fondo que se apaga con los
    /// graves).
    pub cantidad: f32,
}

impl Default for Modulacion {
    fn default() -> Self {
        Modulacion {
            fuente: fuente_por_defecto(),
            senal: OrigenDeSenal::Nivel,
            cantidad: 1.0,
        }
    }
}

impl Modulacion {
    /// Un archivo editado a mano puede traer cualquier número: se deja entre −1 y 1.
    pub fn normalizar(&mut self) {
        self.cantidad = if self.cantidad.is_finite() {
            self.cantidad.clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}

/// El valor de un parámetro (de 0 a 1) después de modular: `base + cantidad · señal`, acotado.
///
/// `base` es lo que se eligió en el deslizador del parámetro. Con la señal en 0 (silencio)
/// el parámetro vale su base; con la señal a fondo, base + cantidad. Así una opacidad base de
/// 0,2 con cantidad 0,8 «respira» entre 0,2 y 1.
pub fn aplicar_modulacion(base: f32, modulacion: &Modulacion, senales: &Senales) -> f32 {
    (base + modulacion.cantidad * modulacion.senal.valor(senales)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn senales() -> Senales {
        Senales {
            nivel: 0.5,
            graves: 1.0,
            medios: 0.25,
            agudos: 0.0,
        }
    }

    #[test]
    fn cada_origen_lee_su_senal() {
        let s = senales();
        assert_eq!(OrigenDeSenal::Nivel.valor(&s), 0.5);
        assert_eq!(OrigenDeSenal::Graves.valor(&s), 1.0);
        assert_eq!(OrigenDeSenal::Medios.valor(&s), 0.25);
        assert_eq!(OrigenDeSenal::Agudos.valor(&s), 0.0);
    }

    #[test]
    fn con_silencio_el_parametro_vale_su_base_y_a_fondo_base_mas_cantidad() {
        let m = Modulacion {
            senal: OrigenDeSenal::Graves,
            cantidad: 0.8,
            ..Modulacion::default()
        };
        let silencio = Senales::default();
        assert_eq!(aplicar_modulacion(0.2, &m, &silencio), 0.2);
        let a_fondo = Senales {
            graves: 1.0,
            ..Senales::default()
        };
        assert!((aplicar_modulacion(0.2, &m, &a_fondo) - 1.0).abs() < 1e-6);
        // A mitad de la señal, a mitad del camino.
        let mitad = Senales {
            graves: 0.5,
            ..Senales::default()
        };
        assert!((aplicar_modulacion(0.2, &m, &mitad) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn el_resultado_nunca_sale_de_0_a_1() {
        let sube = Modulacion {
            senal: OrigenDeSenal::Graves,
            cantidad: 1.0,
            ..Modulacion::default()
        };
        let baja = Modulacion {
            senal: OrigenDeSenal::Graves,
            cantidad: -1.0,
            ..Modulacion::default()
        };
        let s = senales();
        // 0,9 + 1,0 se acota a 1; 0,2 − 1,0 se acota a 0.
        assert_eq!(aplicar_modulacion(0.9, &sube, &s), 1.0);
        assert_eq!(aplicar_modulacion(0.2, &baja, &s), 0.0);
    }

    #[test]
    fn una_cantidad_negativa_baja_el_parametro() {
        let m = Modulacion {
            senal: OrigenDeSenal::Medios,
            cantidad: -0.4,
            ..Modulacion::default()
        };
        // 1,0 − 0,4 · 0,25 = 0,9
        assert!((aplicar_modulacion(1.0, &m, &senales()) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn una_cantidad_invalida_se_repara() {
        for (entrada, esperado) in [(5.0, 1.0), (-5.0, -1.0), (f32::NAN, 0.0), (0.3, 0.3)] {
            let mut m = Modulacion {
                senal: OrigenDeSenal::Nivel,
                cantidad: entrada,
                ..Modulacion::default()
            };
            m.normalizar();
            assert_eq!(m.cantidad, esperado);
        }
    }

    #[test]
    fn una_modulacion_se_guarda_y_se_lee() {
        let m = Modulacion {
            senal: OrigenDeSenal::Agudos,
            cantidad: -0.25,
            ..Modulacion::default()
        };
        let leida: Modulacion = ron::from_str(&ron::to_string(&m).unwrap()).unwrap();
        assert_eq!(leida, m);
    }

    #[test]
    fn la_fuente_se_guarda_y_se_lee_en_texto() {
        for fuente in [
            FuenteDeAudio::Ninguna,
            FuenteDeAudio::Escritorio,
            FuenteDeAudio::Microfono,
            FuenteDeAudio::Aplicacion("Firefox".into()),
            // Un nombre con dos puntos no se confunde con el separador.
            FuenteDeAudio::Aplicacion("Mi app: versión 2".into()),
            FuenteDeAudio::Capa(0),
            FuenteDeAudio::Capa(7),
        ] {
            assert_eq!(FuenteDeAudio::desde_texto(&fuente.a_texto()), fuente);
        }
    }

    #[test]
    fn un_texto_ilegible_vale_ninguna() {
        for texto in [
            "",
            "   ",
            "cualquier cosa",
            "aplicacion:",
            "aplicacion:   ",
            "capa:",
            "capa:0", // se cuenta desde 1
            "capa:x",
        ] {
            assert_eq!(FuenteDeAudio::desde_texto(texto), FuenteDeAudio::Ninguna);
        }
        assert_eq!(
            FuenteDeAudio::desde_texto("  escritorio\n"),
            FuenteDeAudio::Escritorio
        );
    }

    #[test]
    fn una_modulacion_de_antes_sin_fuente_escucha_el_escritorio() {
        let m: Modulacion = ron::from_str("(senal: Graves, cantidad: 0.5)").unwrap();
        assert_eq!(m.fuente, FuenteDeAudio::Escritorio);
        let con_fuente = Modulacion {
            fuente: FuenteDeAudio::Aplicacion("Firefox".into()),
            ..m
        };
        let leida: Modulacion = ron::from_str(&ron::to_string(&con_fuente).unwrap()).unwrap();
        assert_eq!(leida, con_fuente);
    }
}
