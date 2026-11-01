//! MIDI: de los bytes que manda un controlador a una orden del programa. Rust puro: no abre
//! ningún dispositivo (eso lo hace `vm_control`), así se prueba con una tabla de casos.
//!
//! Un controlador manda notas (un botón o una tecla) y *controles* (una perilla o un
//! deslizador, de 0 a 127). Cada uno se **asigna** a un destino con «MIDI learn»: se elige el
//! destino, se mueve el control y la asignación queda guardada en el proyecto.

use serde::{Deserialize, Serialize};

use crate::composicion::TRANSICION_MAXIMA;
use crate::control::OrdenDeControl;

/// Lo que mandó el control: una nota o un controlador continuo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TipoMidi {
    Nota,
    Control,
}

/// Qué control físico es: el tipo, el número (la nota o el controlador, de 0 a 127) y el canal
/// (de 0 a 15; en la interfaz se muestra desde 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntradaMidi {
    pub canal: u8,
    pub tipo: TipoMidi,
    pub numero: u8,
}

impl EntradaMidi {
    /// Una descripción corta para la interfaz: «Nota 36 (canal 1)» o «Control 7 (canal 1)».
    pub fn descripcion(&self) -> String {
        let tipo = match self.tipo {
            TipoMidi::Nota => "Nota",
            TipoMidi::Control => "Control",
        };
        format!("{tipo} {} (canal {})", self.numero, self.canal + 1)
    }
}

/// Lo que se puede manejar con un control. Capas y columnas son índices desde 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DestinoDeControl {
    /// Disparar el clip de una celda.
    Clip {
        capa: usize,
        columna: usize,
    },
    /// Disparar una columna entera.
    Columna(usize),
    Detener {
        capa: usize,
    },
    DetenerTodo,
    AlternarPausa {
        capa: usize,
    },
    /// La opacidad de una capa (de 0 a 1).
    Opacidad {
        capa: usize,
    },
    /// Mostrar u ocultar una capa.
    Visible {
        capa: usize,
    },
    /// La duración del fundido entre clips.
    Transicion,
}

impl DestinoDeControl {
    /// Una descripción corta para la interfaz (capas y columnas desde 1).
    pub fn descripcion(&self) -> String {
        match self {
            DestinoDeControl::Clip { capa, columna } => {
                format!("Clip: capa {}, columna {}", capa + 1, columna + 1)
            }
            DestinoDeControl::Columna(c) => format!("Columna {}", c + 1),
            DestinoDeControl::Detener { capa } => format!("Detener la capa {}", capa + 1),
            DestinoDeControl::DetenerTodo => "Detener todo".into(),
            DestinoDeControl::AlternarPausa { capa } => format!("Pausa de la capa {}", capa + 1),
            DestinoDeControl::Opacidad { capa } => format!("Opacidad de la capa {}", capa + 1),
            DestinoDeControl::Visible { capa } => format!("Mostrar la capa {}", capa + 1),
            DestinoDeControl::Transicion => "Duración del fundido".into(),
        }
    }

    /// La orden que resulta cuando el control vale `valor` (de 0 a 1). Un botón dispara con
    /// cualquier valor mayor que cero (y su «soltar», que vale 0, no hace nada); lo continuo
    /// (opacidad, fundido) sigue al valor; mostrar u ocultar es un interruptor a mitad de
    /// recorrido.
    pub fn orden(&self, valor: f32) -> Option<OrdenDeControl> {
        let pulsado = valor > 0.0;
        match *self {
            DestinoDeControl::Clip { capa, columna } => {
                pulsado.then_some(OrdenDeControl::Clip { capa, columna })
            }
            DestinoDeControl::Columna(c) => pulsado.then_some(OrdenDeControl::Columna(c)),
            DestinoDeControl::Detener { capa } => {
                pulsado.then_some(OrdenDeControl::Detener { capa })
            }
            DestinoDeControl::DetenerTodo => pulsado.then_some(OrdenDeControl::DetenerTodo),
            DestinoDeControl::AlternarPausa { capa } => {
                pulsado.then_some(OrdenDeControl::AlternarPausa { capa })
            }
            DestinoDeControl::Opacidad { capa } => Some(OrdenDeControl::Opacidad {
                capa,
                valor: valor.clamp(0.0, 1.0),
            }),
            DestinoDeControl::Visible { capa } => Some(OrdenDeControl::Visible {
                capa,
                visible: valor >= 0.5,
            }),
            DestinoDeControl::Transicion => Some(OrdenDeControl::Transicion(
                (valor * TRANSICION_MAXIMA).clamp(0.0, TRANSICION_MAXIMA),
            )),
        }
    }

    /// ¿Es un destino continuo (una perilla) y no un botón? Sirve para sugerir qué tipo de
    /// control asignarle.
    pub fn es_continuo(&self) -> bool {
        matches!(
            self,
            DestinoDeControl::Opacidad { .. } | DestinoDeControl::Transicion
        )
    }
}

/// Una asignación: este control maneja este destino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsignacionMidi {
    pub entrada: EntradaMidi,
    pub destino: DestinoDeControl,
}

/// Lee un mensaje MIDI crudo. Devuelve qué control fue y su valor de 0 a 127. Una nota que se
/// suelta (nota apagada, o nota encendida con velocidad 0) vale 0. Todo lo que no sea una nota
/// o un control (cambios de programa, relojes, mensajes de sistema...) se ignora.
pub fn interpretar_midi(bytes: &[u8]) -> Option<(EntradaMidi, u8)> {
    let (&estado, datos) = bytes.split_first()?;
    let (&numero, &valor) = (datos.first()?, datos.get(1).unwrap_or(&0));
    // Los datos van de 0 a 127: un byte con el bit alto es otro mensaje.
    if numero > 127 || valor > 127 {
        return None;
    }
    let canal = estado & 0x0F;
    let (tipo, valor) = match estado & 0xF0 {
        0x90 => (TipoMidi::Nota, valor), // nota encendida (velocidad 0 = apagada)
        0x80 => (TipoMidi::Nota, 0),     // nota apagada
        0xB0 => (TipoMidi::Control, valor),
        _ => return None,
    };
    Some((
        EntradaMidi {
            canal,
            tipo,
            numero,
        },
        valor,
    ))
}

/// Las órdenes que resultan de un mensaje: las de todas las asignaciones de ese control.
pub fn resolver_midi(
    asignaciones: &[AsignacionMidi],
    entrada: EntradaMidi,
    valor: u8,
) -> Vec<OrdenDeControl> {
    let valor = f32::from(valor) / 127.0;
    asignaciones
        .iter()
        .filter(|a| a.entrada == entrada)
        .filter_map(|a| a.destino.orden(valor))
        .collect()
}

/// Agrega una asignación aprendida. Un mismo control maneja **un** destino y un destino lo
/// maneja **un** control: si ya había una asignación del mismo control o del mismo destino, se
/// reemplaza (así volver a aprender corrige en vez de acumular).
pub fn asignar(asignaciones: &mut Vec<AsignacionMidi>, nueva: AsignacionMidi) {
    asignaciones.retain(|a| a.entrada != nueva.entrada && a.destino != nueva.destino);
    asignaciones.push(nueva);
}

#[cfg(test)]
mod tests {
    use super::*;
    use TipoMidi::*;

    fn nota(canal: u8, numero: u8) -> EntradaMidi {
        EntradaMidi {
            canal,
            tipo: Nota,
            numero,
        }
    }

    fn control(canal: u8, numero: u8) -> EntradaMidi {
        EntradaMidi {
            canal,
            tipo: Control,
            numero,
        }
    }

    #[test]
    fn se_leen_las_notas_y_los_controles() {
        // Nota encendida en el canal 1 (índice 0), nota 36, velocidad 100.
        assert_eq!(interpretar_midi(&[0x90, 36, 100]), Some((nota(0, 36), 100)));
        // Canal 3.
        assert_eq!(interpretar_midi(&[0x92, 60, 127]), Some((nota(2, 60), 127)));
        // Control 7 (volumen) en el canal 1 con valor 64.
        assert_eq!(interpretar_midi(&[0xB0, 7, 64]), Some((control(0, 7), 64)));
    }

    #[test]
    fn soltar_una_nota_vale_cero_de_las_dos_formas() {
        // Nota encendida con velocidad 0 es una nota apagada.
        assert_eq!(interpretar_midi(&[0x90, 36, 0]), Some((nota(0, 36), 0)));
        // Y una nota apagada, con la velocidad que traiga.
        assert_eq!(interpretar_midi(&[0x80, 36, 64]), Some((nota(0, 36), 0)));
    }

    #[test]
    fn lo_que_no_es_nota_ni_control_se_ignora() {
        assert_eq!(interpretar_midi(&[0xC0, 5]), None); // cambio de programa
        assert_eq!(interpretar_midi(&[0xF8]), None); // reloj
        assert_eq!(interpretar_midi(&[0xE0, 0, 64]), None); // pitch bend
        assert_eq!(interpretar_midi(&[]), None);
        assert_eq!(interpretar_midi(&[0x90]), None); // sin número
        assert_eq!(interpretar_midi(&[0x90, 200, 10]), None); // un dato con el bit alto
    }

    #[test]
    fn un_boton_dispara_al_apretar_y_no_al_soltar() {
        let d = DestinoDeControl::Clip {
            capa: 1,
            columna: 2,
        };
        assert_eq!(
            d.orden(1.0),
            Some(OrdenDeControl::Clip {
                capa: 1,
                columna: 2
            })
        );
        assert!(d.orden(0.3).is_some());
        assert_eq!(d.orden(0.0), None);
        assert_eq!(DestinoDeControl::DetenerTodo.orden(0.0), None);
    }

    #[test]
    fn una_perilla_sigue_el_valor_de_0_a_1() {
        let o = DestinoDeControl::Opacidad { capa: 0 };
        assert_eq!(
            o.orden(0.0),
            Some(OrdenDeControl::Opacidad {
                capa: 0,
                valor: 0.0
            })
        );
        assert_eq!(
            o.orden(0.5),
            Some(OrdenDeControl::Opacidad {
                capa: 0,
                valor: 0.5
            })
        );
        // Y el fundido va de 0 al máximo.
        assert_eq!(
            DestinoDeControl::Transicion.orden(1.0),
            Some(OrdenDeControl::Transicion(TRANSICION_MAXIMA))
        );
        assert_eq!(
            DestinoDeControl::Transicion.orden(0.0),
            Some(OrdenDeControl::Transicion(0.0))
        );
    }

    #[test]
    fn mostrar_u_ocultar_es_un_interruptor_a_mitad_de_recorrido() {
        let d = DestinoDeControl::Visible { capa: 2 };
        assert_eq!(
            d.orden(0.49),
            Some(OrdenDeControl::Visible {
                capa: 2,
                visible: false
            })
        );
        assert_eq!(
            d.orden(0.5),
            Some(OrdenDeControl::Visible {
                capa: 2,
                visible: true
            })
        );
    }

    #[test]
    fn un_mensaje_se_resuelve_con_las_asignaciones_de_ese_control() {
        let asignaciones = [
            AsignacionMidi {
                entrada: nota(0, 36),
                destino: DestinoDeControl::Clip {
                    capa: 0,
                    columna: 0,
                },
            },
            AsignacionMidi {
                entrada: control(0, 7),
                destino: DestinoDeControl::Opacidad { capa: 1 },
            },
        ];
        // La nota 36 dispara el clip; soltarla no hace nada.
        assert_eq!(
            resolver_midi(&asignaciones, nota(0, 36), 100),
            vec![OrdenDeControl::Clip {
                capa: 0,
                columna: 0
            }]
        );
        assert!(resolver_midi(&asignaciones, nota(0, 36), 0).is_empty());
        // El control 7 en 127 es opacidad total.
        assert_eq!(
            resolver_midi(&asignaciones, control(0, 7), 127),
            vec![OrdenDeControl::Opacidad {
                capa: 1,
                valor: 1.0
            }]
        );
        // Otro canal u otra nota: nada.
        assert!(resolver_midi(&asignaciones, nota(1, 36), 100).is_empty());
        assert!(resolver_midi(&asignaciones, nota(0, 37), 100).is_empty());
    }

    #[test]
    fn volver_a_aprender_reemplaza_en_vez_de_acumular() {
        let mut lista = Vec::new();
        let clip = DestinoDeControl::Clip {
            capa: 0,
            columna: 0,
        };
        asignar(
            &mut lista,
            AsignacionMidi {
                entrada: nota(0, 36),
                destino: clip,
            },
        );
        // El mismo destino con otro control: reemplaza al control anterior.
        asignar(
            &mut lista,
            AsignacionMidi {
                entrada: nota(0, 40),
                destino: clip,
            },
        );
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].entrada, nota(0, 40));
        // El mismo control para otro destino: también reemplaza (un control, un destino).
        asignar(
            &mut lista,
            AsignacionMidi {
                entrada: nota(0, 40),
                destino: DestinoDeControl::DetenerTodo,
            },
        );
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].destino, DestinoDeControl::DetenerTodo);
        // Y uno distinto en ambos se suma.
        asignar(
            &mut lista,
            AsignacionMidi {
                entrada: nota(0, 41),
                destino: clip,
            },
        );
        assert_eq!(lista.len(), 2);
    }

    #[test]
    fn las_descripciones_cuentan_desde_1() {
        assert_eq!(nota(0, 36).descripcion(), "Nota 36 (canal 1)");
        assert_eq!(control(2, 7).descripcion(), "Control 7 (canal 3)");
        assert_eq!(
            DestinoDeControl::Clip {
                capa: 0,
                columna: 1
            }
            .descripcion(),
            "Clip: capa 1, columna 2"
        );
    }

    #[test]
    fn una_asignacion_se_guarda_y_se_lee() {
        let a = AsignacionMidi {
            entrada: control(1, 74),
            destino: DestinoDeControl::Opacidad { capa: 3 },
        };
        let leida: AsignacionMidi = ron::from_str(&ron::to_string(&a).unwrap()).unwrap();
        assert_eq!(leida, a);
    }
}
