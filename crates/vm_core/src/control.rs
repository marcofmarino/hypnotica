//! Traducción de los mensajes de un controlador externo (hoy OSC; más adelante MIDI) a
//! órdenes del programa. Rust puro: no sabe nada de redes ni de Bevy, así se prueba con una
//! tabla de casos.
//!
//! Las direcciones tienen dos familias:
//!
//! - **Propias**, en español: `/hypnotica/capa/2/clip/3`, `/hypnotica/columna/1`,
//!   `/hypnotica/capa/2/opacidad 0.5`...
//! - **Compatibles con Resolume**, para reusar plantillas de TouchOSC y similares:
//!   `/composition/layers/2/clips/3/connect 1` y `/composition/layers/2/video/opacity 0.5`
//!   (verificadas en la documentación y los foros de Resolume), y
//!   `/composition/columns/1/connect` (**no verificada**, de memoria).
//!
//! Capas, clips y columnas se cuentan **desde 1**, como en la interfaz («Capa 1» es la fila de
//! arriba). Adentro del programa son índices desde 0.

use crate::composicion::TRANSICION_MAXIMA;

/// Un argumento de un mensaje, de los tipos de OSC que interesan.
#[derive(Debug, Clone, PartialEq)]
pub enum Argumento {
    Entero(i32),
    Real(f32),
    Texto(String),
    Booleano(bool),
}

impl Argumento {
    /// El valor como número, si lo tiene.
    fn como_numero(&self) -> Option<f32> {
        match self {
            Argumento::Entero(n) => Some(*n as f32),
            Argumento::Real(x) => Some(*x),
            Argumento::Booleano(b) => Some(f32::from(u8::from(*b))),
            Argumento::Texto(_) => None,
        }
    }

    /// ¿Es una pulsación (distinto de cero)? Un botón manda 1 al apretarse y a veces 0 al
    /// soltarse: el 0 no tiene que disparar nada.
    fn es_pulsacion(&self) -> bool {
        self.como_numero().is_none_or(|n| n != 0.0)
    }
}

/// Lo que puede pedir un controlador externo.
#[derive(Debug, Clone, PartialEq)]
pub enum OrdenDeControl {
    /// Disparar el clip de una celda.
    Clip {
        capa: usize,
        columna: usize,
    },
    /// Disparar, en cada capa, el clip de esa columna.
    Columna(usize),
    /// Detener una capa.
    Detener {
        capa: usize,
    },
    DetenerTodo,
    /// Pausar una capa, o seguir si estaba en pausa.
    AlternarPausa {
        capa: usize,
    },
    /// La opacidad de una capa, de 0 a 1.
    Opacidad {
        capa: usize,
        valor: f32,
    },
    Visible {
        capa: usize,
        visible: bool,
    },
    /// La duración del fundido entre clips, en segundos.
    Transicion(f32),
}

/// Interpreta un mensaje. `Ok(None)` es un mensaje que se entiende pero no hace nada (un botón
/// que se soltó); `Err` explica por qué no se pudo interpretar.
pub fn interpretar(
    direccion: &str,
    argumentos: &[Argumento],
) -> Result<Option<OrdenDeControl>, String> {
    let partes: Vec<&str> = direccion.split('/').filter(|p| !p.is_empty()).collect();
    // Un botón dispara con cualquier valor distinto de cero, o sin valor.
    let pulsado = argumentos.first().is_none_or(Argumento::es_pulsacion);
    // Un número que viene como valor (opacidad, segundos...).
    let numero = || {
        argumentos
            .first()
            .and_then(Argumento::como_numero)
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{direccion}: falta un número"))
    };

    let orden = match partes.as_slice() {
        // --- Propias ---
        ["hypnotica", "capa", n, "clip", m] => pulsar(pulsado, || {
            Ok(OrdenDeControl::Clip {
                capa: indice(n, direccion)?,
                columna: indice(m, direccion)?,
            })
        })?,
        ["hypnotica", "columna", m] => pulsar(pulsado, || {
            Ok(OrdenDeControl::Columna(indice(m, direccion)?))
        })?,
        ["hypnotica", "capa", n, "detener"] => pulsar(pulsado, || {
            Ok(OrdenDeControl::Detener {
                capa: indice(n, direccion)?,
            })
        })?,
        ["hypnotica", "detener"] => pulsar(pulsado, || Ok(OrdenDeControl::DetenerTodo))?,
        ["hypnotica", "capa", n, "pausa"] => pulsar(pulsado, || {
            Ok(OrdenDeControl::AlternarPausa {
                capa: indice(n, direccion)?,
            })
        })?,
        ["hypnotica", "capa", n, "opacidad"] => Some(OrdenDeControl::Opacidad {
            capa: indice(n, direccion)?,
            valor: numero()?.clamp(0.0, 1.0),
        }),
        ["hypnotica", "capa", n, "visible"] => Some(OrdenDeControl::Visible {
            capa: indice(n, direccion)?,
            visible: numero()? != 0.0,
        }),
        ["hypnotica", "transicion"] => Some(OrdenDeControl::Transicion(
            numero()?.clamp(0.0, TRANSICION_MAXIMA),
        )),

        // --- Compatibles con Resolume ---
        ["composition", "layers", n, "clips", m, "connect"] => pulsar(pulsado, || {
            Ok(OrdenDeControl::Clip {
                capa: indice(n, direccion)?,
                columna: indice(m, direccion)?,
            })
        })?,
        ["composition", "columns", m, "connect"] => pulsar(pulsado, || {
            Ok(OrdenDeControl::Columna(indice(m, direccion)?))
        })?,
        ["composition", "layers", n, "video", "opacity"] => Some(OrdenDeControl::Opacidad {
            capa: indice(n, direccion)?,
            valor: numero()?.clamp(0.0, 1.0),
        }),

        _ => return Err(format!("dirección desconocida: {direccion}")),
    };
    Ok(orden)
}

/// Si fue una pulsación, arma la orden; si fue un «soltar» (0), no hace nada.
fn pulsar(
    pulsado: bool,
    orden: impl FnOnce() -> Result<OrdenDeControl, String>,
) -> Result<Option<OrdenDeControl>, String> {
    if pulsado { orden().map(Some) } else { Ok(None) }
}

/// Un número de la dirección (desde 1) a un índice (desde 0).
fn indice(texto: &str, direccion: &str) -> Result<usize, String> {
    texto
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .ok_or_else(|| format!("{direccion}: «{texto}» no es un número desde 1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Argumento::*;
    use OrdenDeControl as O;

    fn ok(direccion: &str, args: &[Argumento]) -> Option<OrdenDeControl> {
        interpretar(direccion, args).unwrap_or_else(|e| panic!("{direccion}: {e}"))
    }

    #[test]
    fn disparar_un_clip_con_las_dos_familias_de_direcciones() {
        let esperado = Some(O::Clip {
            capa: 1,
            columna: 2,
        });
        // Propia, sin valor y con valor.
        assert_eq!(ok("/hypnotica/capa/2/clip/3", &[]), esperado);
        assert_eq!(ok("/hypnotica/capa/2/clip/3", &[Entero(1)]), esperado);
        // Resolume: manda 1 al apretar.
        assert_eq!(
            ok("/composition/layers/2/clips/3/connect", &[Entero(1)]),
            esperado
        );
    }

    #[test]
    fn soltar_un_boton_no_dispara_nada() {
        for args in [[Entero(0)], [Real(0.0)], [Booleano(false)]] {
            assert_eq!(ok("/composition/layers/1/clips/1/connect", &args), None);
            assert_eq!(ok("/hypnotica/columna/1", &args), None);
        }
        // Y cualquier valor distinto de cero es una pulsación.
        assert!(ok("/hypnotica/columna/1", &[Real(0.7)]).is_some());
    }

    #[test]
    fn columnas_detener_y_pausa() {
        assert_eq!(ok("/hypnotica/columna/4", &[]), Some(O::Columna(3)));
        assert_eq!(
            ok("/composition/columns/2/connect", &[Entero(1)]),
            Some(O::Columna(1))
        );
        assert_eq!(
            ok("/hypnotica/capa/3/detener", &[]),
            Some(O::Detener { capa: 2 })
        );
        assert_eq!(ok("/hypnotica/detener", &[]), Some(O::DetenerTodo));
        assert_eq!(
            ok("/hypnotica/capa/1/pausa", &[Entero(1)]),
            Some(O::AlternarPausa { capa: 0 })
        );
    }

    #[test]
    fn la_opacidad_se_limita_de_0_a_1() {
        let opacidad = |direccion: &str, v: f32| ok(direccion, &[Real(v)]);
        assert_eq!(
            opacidad("/hypnotica/capa/2/opacidad", 0.25),
            Some(O::Opacidad {
                capa: 1,
                valor: 0.25
            })
        );
        assert_eq!(
            opacidad("/composition/layers/2/video/opacity", 1.7),
            Some(O::Opacidad {
                capa: 1,
                valor: 1.0
            })
        );
        assert_eq!(
            opacidad("/hypnotica/capa/1/opacidad", -3.0),
            Some(O::Opacidad {
                capa: 0,
                valor: 0.0
            })
        );
        // Un entero también sirve.
        assert_eq!(
            ok("/hypnotica/capa/1/opacidad", &[Entero(1)]),
            Some(O::Opacidad {
                capa: 0,
                valor: 1.0
            })
        );
    }

    #[test]
    fn visible_y_transicion() {
        assert_eq!(
            ok("/hypnotica/capa/1/visible", &[Entero(0)]),
            Some(O::Visible {
                capa: 0,
                visible: false
            })
        );
        assert_eq!(
            ok("/hypnotica/capa/1/visible", &[Booleano(true)]),
            Some(O::Visible {
                capa: 0,
                visible: true
            })
        );
        assert_eq!(
            ok("/hypnotica/transicion", &[Real(2.5)]),
            Some(O::Transicion(2.5))
        );
        // Se limita al máximo del proyecto.
        assert_eq!(
            ok("/hypnotica/transicion", &[Real(99.0)]),
            Some(O::Transicion(TRANSICION_MAXIMA))
        );
    }

    #[test]
    fn lo_que_no_se_entiende_da_un_error_claro() {
        // Dirección desconocida.
        assert!(
            interpretar("/otra/cosa", &[])
                .unwrap_err()
                .contains("desconocida")
        );
        // Cero no es una capa: se cuenta desde 1.
        assert!(
            interpretar("/hypnotica/capa/0/clip/1", &[])
                .unwrap_err()
                .contains("desde 1")
        );
        // No es un número.
        assert!(interpretar("/hypnotica/capa/x/detener", &[]).is_err());
        // Falta el valor, o no es un número.
        assert!(
            interpretar("/hypnotica/capa/1/opacidad", &[])
                .unwrap_err()
                .contains("falta")
        );
        assert!(interpretar("/hypnotica/capa/1/opacidad", &[Texto("mucha".into())]).is_err());
        assert!(interpretar("/hypnotica/capa/1/opacidad", &[Real(f32::NAN)]).is_err());
    }

    #[test]
    fn las_barras_de_mas_y_el_final_no_importan() {
        assert_eq!(ok("//hypnotica//columna/1/", &[]), Some(O::Columna(0)));
    }
}
