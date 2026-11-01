//! La pestaña «Control»: todo lo que viene de afuera. A la izquierda el controlador MIDI y sus
//! asignaciones; a la derecha OSC (recibir órdenes y mandar el estado a un celular).

use crate::composicion::ContextoComposicion;
use crate::estilo::{rojo, texto_negrita, verde};
use bevy_egui::egui;

/// La pantalla de control, en dos columnas con scroll propio.
pub(crate) fn pestana_control(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    ui.columns(2, |columnas| {
        egui::ScrollArea::vertical()
            .id_salt("control_midi")
            .auto_shrink([false, false])
            .show(&mut columnas[0], |ui| {
                ui.label(texto_negrita("MIDI"));
                ui.add_space(6.0);
                crate::midi::pestana_midi(ui, ctx);
            });
        egui::ScrollArea::vertical()
            .id_salt("control_osc")
            .auto_shrink([false, false])
            .show(&mut columnas[1], |ui| {
                ui.label(texto_negrita("OSC"));
                ui.add_space(6.0);
                selector_de_osc(ui, ctx);
            });
    });
}

/// El control por OSC: un interruptor, el puerto y lo que está pasando. Mientras está
/// prendido escucha en toda la red local.
fn selector_de_osc(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let actual = **ctx.config_osc;
    let mut nuevo = actual;
    ui.horizontal(|ui| {
        ui.checkbox(&mut nuevo.activo, "Control por OSC")
            .on_hover_text("Recibir órdenes desde un celular u otro programa. Mientras está prendido escucha en toda la red local.");
        ui.label("puerto");
        // `speed(0.0)`: el valor se escribe, no se arrastra (cada cambio reinicia el servidor).
        ui.add(
            egui::DragValue::new(&mut nuevo.puerto)
                .range(1024..=65535)
                .speed(0.0),
        );
    });
    // Solo se escribe si cambió: así se guarda una vez y el servidor no se reinicia de más.
    if nuevo != actual {
        **ctx.config_osc = nuevo;
    }
    campo_de_destino_osc(ui, ctx);
    if let Some(motivo) = &ctx.estado_osc.error {
        ui.add(egui::Label::new(egui::RichText::new(motivo).color(rojo())).truncate());
    } else if ctx.estado_osc.escuchando.is_some() {
        let ultimo = ctx
            .estado_osc
            .ultimo
            .as_deref()
            .unwrap_or("esperando mensajes");
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{} mensajes · {ultimo}", ctx.estado_osc.recibidos))
                    .color(verde()),
            )
            .truncate(),
        );
    }
}

/// A dónde se manda el estado (`ip:puerto`), para que el celular refleje lo que pasa. Se
/// confirma al apretar Enter o al salir del campo; algo que no sea `ip:puerto` se descarta.
fn campo_de_destino_osc(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    let id = egui::Id::new("destino_osc_texto");
    let actual = ctx.destino_osc.direccion;
    let mut texto = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| actual.map(|a| a.to_string()).unwrap_or_default());
    let valido = texto.trim().is_empty() || texto.trim().parse::<std::net::SocketAddr>().is_ok();
    let respuesta = ui
        .horizontal(|ui| {
            ui.label("Enviar estado a:");
            let mut campo = egui::TextEdit::singleline(&mut texto)
                .hint_text("192.168.1.20:9000")
                .desired_width(150.0);
            if !valido {
                campo = campo.text_color(rojo());
            }
            ui.add(campo).on_hover_text(
                "Dirección del celular o programa que recibe el estado (capa activa, \
                     opacidad…). Vacío: no se envía nada.",
            )
        })
        .inner;
    if respuesta.lost_focus() && valido {
        let nueva = texto.trim().parse().ok();
        if nueva != actual {
            ctx.destino_osc.direccion = nueva;
        }
    }
    // Mientras se escribe se recuerda el texto; al salir del campo, vuelve a lo guardado.
    if respuesta.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, texto));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
}
