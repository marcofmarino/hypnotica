//! La fila de un parámetro modulable, al estilo del inspector de Godot:
//!
//! `Opacidad  [──────●───]  60 %  [🔊]`
//!
//! El botón del sonido despliega (o esconde) las opciones de la modulación de ese parámetro:
//! activarla, de dónde sale el sonido, qué parte del sonido se toma y cuánto mueve el valor.
//! Cada parámetro tiene SU modulación y SU fuente, así que dos capas pueden escuchar cosas
//! distintas. Cualquier parámetro nuevo que se pueda modular usa esta misma fila.

use crate::estilo::{azul, icono, rojo, verde};
use crate::inspector::{
    ALTO_FILA, boton_cuadrado, boton_cuadrado_elegible, deslizador, desplegable, fila,
};
use bevy_egui::egui;
use egui_phosphor::regular as icono_de;
use vm_audio::{AplicacionesQueSuenan, SenalesDeAudio};
use vm_core::{FuenteDeAudio, Modulacion, OrigenDeSenal};

/// Lo que la fila necesita saber del sonido, además del parámetro.
pub(crate) struct ContextoDeAudio<'a> {
    /// Las aplicaciones que suenan ahora (para elegir una como fuente).
    pub(crate) aplicaciones: &'a mut AplicacionesQueSuenan,
    /// Lo que se oye de cada fuente (para el medidor y los errores).
    pub(crate) senales: &'a SenalesDeAudio,
    /// Los nombres de las capas (para elegir el clip de una como fuente).
    pub(crate) nombres_de_capas: &'a [String],
}

/// Una fila de un parámetro de 0 a 100 %: `Opacidad [──●──] 60 % [🔊]`. Devuelve `true` si algo
/// cambió (el valor o la modulación).
///
/// - `id`: identifica la fila (guarda si está desplegada).
pub(crate) fn fila_de_parametro(
    ui: &mut egui::Ui,
    id: egui::Id,
    etiqueta: &str,
    porcentaje: &mut f32,
    modulacion: &mut Option<Modulacion>,
    audio: &mut ContextoDeAudio,
) -> bool {
    let mut cambio = false;
    let id_abierta = id.with("abierta");
    let mut abierta = ui.data(|d| d.get_temp::<bool>(id_abierta)).unwrap_or(false);

    fila(ui, etiqueta, |ui| {
        // El deslizador deja lugar a la derecha para el botón del sonido.
        let reservado = ALTO_FILA + ui.spacing().item_spacing.x;
        cambio |= deslizador(ui, porcentaje, 0.0..=100.0, " %", 0, reservado).changed();
        // El botón del sonido: de color si el parámetro reacciona, y marcado si está desplegado.
        let reacciona = modulacion.is_some();
        let color = if reacciona { verde() } else { azul() };
        let ayuda = if reacciona {
            "Reacciona al sonido. Tocá para ver o esconder las opciones."
        } else {
            "Hacer que reaccione al sonido"
        };
        if boton_cuadrado_elegible(ui, abierta, icono(icono_de::SPEAKER_HIGH, color))
            .on_hover_text(ayuda)
            .clicked()
        {
            abierta = !abierta;
        }
    });

    if abierta {
        // Las opciones del sonido, en un recuadro que ocupa todo el ancho.
        egui::Frame::NONE
            .fill(
                ui.visuals()
                    .widgets
                    .noninteractive
                    .weak_bg_fill
                    .gamma_multiply(0.6),
            )
            .corner_radius(4.0)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                cambio |= opciones_de_sonido(ui, id, porcentaje, modulacion, audio);
            });
        ui.add_space(2.0);
    }
    ui.data_mut(|d| d.insert_temp(id_abierta, abierta));
    cambio
}

/// Activa la modulación (con sus valores por defecto) o la quita. Con la base al tope, sumar no
/// se vería: al activarla se baja a 20 %.
fn activar_o_quitar(activa: bool, porcentaje: &mut f32, modulacion: &mut Option<Modulacion>) {
    if activa {
        *modulacion = Some(Modulacion::default());
        if *porcentaje > 90.0 {
            *porcentaje = 20.0;
        }
    } else {
        *modulacion = None;
    }
}

/// Lo que se muestra al desplegar: activar, fuente, señal, cantidad y un medidor.
fn opciones_de_sonido(
    ui: &mut egui::Ui,
    id: egui::Id,
    porcentaje: &mut f32,
    modulacion: &mut Option<Modulacion>,
    audio: &mut ContextoDeAudio,
) -> bool {
    let mut cambio = false;

    let mut activa = modulacion.is_some();
    if ui
        .checkbox(&mut activa, "Reaccionar al sonido")
        .on_hover_text(
            "El valor de la barra es el que tiene sin sonido; el sonido le suma hasta la \
             cantidad elegida.",
        )
        .changed()
    {
        cambio = true;
        activar_o_quitar(activa, porcentaje, modulacion);
    }
    let Some(m) = modulacion.as_mut() else {
        return cambio;
    };

    // La primera vez que se necesita, la lista de aplicaciones se llena sola.
    if !audio.aplicaciones.buscadas {
        audio.aplicaciones.actualizar();
    }

    fila(ui, "Fuente", |ui| {
        // El desplegable deja lugar al botón de volver a buscar.
        let ancho_boton = ALTO_FILA;
        let ancho = ui.available_width() - ancho_boton - ui.spacing().item_spacing.x;
        ui.allocate_ui(egui::vec2(ancho, ALTO_FILA), |ui| {
            cambio |= selector_de_fuente(ui, id, &mut m.fuente, audio);
        });
        if boton_cuadrado(ui, icono(icono_de::ARROWS_CLOCKWISE, azul()))
            .on_hover_text("Volver a buscar las aplicaciones que están sonando")
            .clicked()
        {
            audio.aplicaciones.actualizar();
        }
    });

    fila(ui, "Señal", |ui| {
        desplegable(ui, id.with("senal"), m.senal.nombre(), |ui| {
            for opcion in OrigenDeSenal::TODOS {
                cambio |=
                    crate::inspector::opcion(ui, &mut m.senal, opcion, opcion.nombre()).changed();
            }
        })
        .on_hover_text("Qué parte del sonido mueve el valor");
    });

    fila(ui, "Cantidad", |ui| {
        let mut cantidad = m.cantidad * 100.0;
        if deslizador(ui, &mut cantidad, -100.0..=100.0, " %", 0, 0.0)
            .on_hover_text("Cuánto suma el sonido a fondo. Negativo: lo baja.")
            .changed()
        {
            m.cantidad = cantidad / 100.0;
            cambio = true;
        }
    });

    // El medidor de lo que llega por esa fuente.
    fila(ui, "Entra", |ui| {
        let valor = audio
            .senales
            .de(&m.fuente)
            .map_or(0.0, |e| m.senal.valor(&e.senales));
        ui.add(egui::ProgressBar::new(valor).desired_width(ui.available_width()));
    });

    if let Some(motivo) = audio.senales.de(&m.fuente).and_then(|e| e.error.as_ref()) {
        ui.add(egui::Label::new(egui::RichText::new(motivo).color(rojo())).truncate());
    }
    cambio
}

/// El desplegable de la fuente de sonido. Devuelve `true` si se eligió otra.
fn selector_de_fuente(
    ui: &mut egui::Ui,
    id: egui::Id,
    fuente: &mut FuenteDeAudio,
    audio: &mut ContextoDeAudio,
) -> bool {
    let actual = fuente.clone();
    let texto = match &actual {
        FuenteDeAudio::Capa(i) => audio
            .nombres_de_capas
            .get(*i)
            .map_or_else(|| actual.nombre(), |n| format!("Clip de «{n}»")),
        otra => otra.nombre(),
    };
    desplegable(ui, id.with("fuente"), &texto, |ui| {
        crate::inspector::opcion(ui, fuente, FuenteDeAudio::Escritorio, "Escritorio");
        crate::inspector::opcion(ui, fuente, FuenteDeAudio::Microfono, "Micrófono");
        ui.separator();
        // El sonido de un clip puesto en la grilla (aunque esté silenciado).
        for (i, nombre) in audio.nombres_de_capas.iter().enumerate() {
            crate::inspector::opcion(
                ui,
                fuente,
                FuenteDeAudio::Capa(i),
                format!("Clip de «{nombre}»"),
            );
        }
        ui.separator();
        // Si la elegida es una aplicación que no suena ahora, igual se ve.
        if let FuenteDeAudio::Aplicacion(nombre) = &actual
            && !audio.aplicaciones.lista.iter().any(|a| &a.nombre == nombre)
        {
            crate::inspector::opcion(ui, fuente, actual.clone(), format!("{nombre} (no suena)"));
        }
        for app in &audio.aplicaciones.lista {
            crate::inspector::opcion(
                ui,
                fuente,
                FuenteDeAudio::Aplicacion(app.nombre.clone()),
                &app.nombre,
            );
        }
    })
    .on_hover_text("De dónde se escucha el sonido de este parámetro");
    *fuente != actual
}

#[cfg(test)]
mod tests {
    use super::*;
    use vm_core::Modulacion;

    #[test]
    fn al_activar_con_la_base_al_tope_se_baja_a_20() {
        let (mut p, mut m) = (100.0, None);
        activar_o_quitar(true, &mut p, &mut m);
        assert!(m.is_some());
        assert_eq!(p, 20.0);
        // Una base chica se respeta.
        let (mut p, mut m) = (35.0, None);
        activar_o_quitar(true, &mut p, &mut m);
        assert_eq!(p, 35.0);
        // Desactivar quita la modulación y no toca el valor.
        activar_o_quitar(false, &mut p, &mut m);
        assert!(m.is_none());
        assert_eq!(p, 35.0);
    }

    /// Dibuja la fila en cada estado (cerrada, abierta sin y con modulación) sin ventana: no
    /// tiene que fallar ni decir que algo cambió sin que nadie toque nada.
    #[test]
    fn la_fila_se_dibuja_en_todos_sus_estados_sin_cambiar_nada() {
        let ctx = egui::Context::default();
        let mut aplicaciones = AplicacionesQueSuenan::default();
        aplicaciones.buscadas = true; // no lanzar `pw-dump` en una prueba
        let senales = SenalesDeAudio::default();
        let nombres = vec!["Fondo".to_string(), "Luces".to_string()];
        let id = egui::Id::new("fila_de_prueba");
        let casos = [
            (false, None),
            (true, None),
            (true, Some(Modulacion::default())),
            (
                true,
                Some(Modulacion {
                    fuente: FuenteDeAudio::Capa(1),
                    ..Modulacion::default()
                }),
            ),
        ];
        for (abierta, modulacion) in casos {
            ctx.data_mut(|d| d.insert_temp(id.with("abierta"), abierta));
            let (mut p, mut m) = (50.0, modulacion.clone());
            let mut cambio = true;
            let mut salida = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let mut audio = ContextoDeAudio {
                        aplicaciones: &mut aplicaciones,
                        senales: &senales,
                        nombres_de_capas: &nombres,
                    };
                    cambio = fila_de_parametro(ui, id, "Opacidad", &mut p, &mut m, &mut audio);
                },
            );
            // Sin GPU no hay a dónde subir las texturas que egui pide: se descartan a propósito.
            salida.textures_delta.clear();
            assert!(!cambio, "no se tocó nada: {abierta} {modulacion:?}");
            assert_eq!(p, 50.0);
            assert_eq!(m, modulacion);
        }
    }
}
