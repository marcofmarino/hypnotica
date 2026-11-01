//! La pestaña MIDI: elegir el controlador y asignar sus botones y perillas («MIDI learn»).
//!
//! Se elige QUÉ se quiere controlar, se aprieta «Aprender» y se mueve el control del
//! aparato: el primer mensaje que llega queda asignado. Las asignaciones se guardan en el
//! proyecto; el controlador elegido, aparte (es del equipo).

use crate::composicion::ContextoComposicion;
use crate::estilo::{rojo, verde};
use bevy_egui::egui;
use vm_control::{AprendizajeMidi, ConfigMidi};
use vm_core::DestinoDeControl;

/// Los tipos de destino que se pueden elegir, en el orden en que se muestran.
const TIPOS: [&str; 8] = [
    "Disparar un clip",
    "Disparar una columna",
    "Detener una capa",
    "Detener todo",
    "Pausa de una capa",
    "Opacidad de una capa",
    "Mostrar u ocultar una capa",
    "Duración del fundido",
];

/// Arma el destino elegido (capa y columna llegan desde 0).
fn destino_de(tipo: usize, capa: usize, columna: usize) -> DestinoDeControl {
    match tipo {
        0 => DestinoDeControl::Clip { capa, columna },
        1 => DestinoDeControl::Columna(columna),
        2 => DestinoDeControl::Detener { capa },
        3 => DestinoDeControl::DetenerTodo,
        4 => DestinoDeControl::AlternarPausa { capa },
        5 => DestinoDeControl::Opacidad { capa },
        6 => DestinoDeControl::Visible { capa },
        _ => DestinoDeControl::Transicion,
    }
}

/// ¿El tipo necesita elegir capa? ¿Y columna?
fn usa_capa(tipo: usize) -> bool {
    matches!(tipo, 0 | 2 | 4 | 5 | 6)
}
fn usa_columna(tipo: usize) -> bool {
    matches!(tipo, 0 | 1)
}

pub(crate) fn pestana_midi(ui: &mut egui::Ui, ctx: &mut ContextoComposicion) {
    // La primera vez se busca solo; después, con el botón.
    if !ctx.puertos_midi.buscados {
        ctx.puertos_midi.actualizar();
    }

    // --- Controlador ---
    let actual = ctx.config_midi.puerto.clone();
    let mut elegido = actual.clone();
    ui.horizontal(|ui| {
        ui.label("Controlador:");
        egui::ComboBox::from_id_salt("puerto_midi")
            .selected_text(elegido.as_deref().unwrap_or("Ninguno"))
            .width(180.0)
            .show_ui(ui, |ui| {
                crate::inspector::opcion(ui, &mut elegido, None, "Ninguno");
                for p in &ctx.puertos_midi.lista {
                    crate::inspector::opcion(ui, &mut elegido, Some(p.clone()), p);
                }
            });
        if ui
            .small_button("↻")
            .on_hover_text("Buscar controladores conectados")
            .clicked()
        {
            ctx.puertos_midi.actualizar();
        }
    });
    // Solo se escribe si cambió: cada cambio reconecta y se guarda en disco.
    if elegido != actual {
        **ctx.config_midi = ConfigMidi { puerto: elegido };
    }
    if let Some(motivo) = &ctx.estado_midi.error {
        ui.add(egui::Label::new(egui::RichText::new(motivo).color(rojo())).truncate());
    } else if ctx.estado_midi.conectado.is_some() {
        let ultimo = ctx.estado_midi.ultimo.as_deref().unwrap_or("esperando");
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{} mensajes · {ultimo}", ctx.estado_midi.recibidos))
                    .color(verde()),
            )
            .truncate(),
        );
    }
    ui.add_space(6.0);

    // --- Qué se quiere controlar ---
    let id = egui::Id::new("destino_midi");
    let (mut tipo, mut capa, mut columna) = ui
        .data(|d| d.get_temp::<(usize, usize, usize)>(id))
        .unwrap_or((0, 0, 0));
    let capas = ctx.proyecto.0.composicion.capas.len().max(1);
    let columnas = ctx.proyecto.0.composicion.columnas.len().max(1);
    capa = capa.min(capas - 1);
    columna = columna.min(columnas - 1);

    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("tipo_de_destino_midi")
            .selected_text(TIPOS[tipo])
            .width(170.0)
            .show_ui(ui, |ui| {
                for (i, nombre) in TIPOS.iter().enumerate() {
                    crate::inspector::opcion(ui, &mut tipo, i, *nombre);
                }
            });
        if usa_capa(tipo) {
            ui.label("capa");
            let mut n = capa + 1;
            ui.add(egui::DragValue::new(&mut n).range(1..=capas).speed(0.1));
            capa = n - 1;
        }
        if usa_columna(tipo) {
            ui.label("columna");
            let mut n = columna + 1;
            ui.add(egui::DragValue::new(&mut n).range(1..=columnas).speed(0.1));
            columna = n - 1;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (tipo, capa, columna)));
    let destino = destino_de(tipo, capa, columna);

    let aprendiendo = ctx.aprendizaje_midi.destino == Some(destino);
    ui.horizontal(|ui| {
        let texto = if aprendiendo {
            "Mové un control del aparato… (cancelar)"
        } else {
            "Aprender"
        };
        if crate::inspector::elegible(ui, aprendiendo, texto)
            .on_hover_text(
                "Apretá un botón o mové una perilla del controlador: queda asignado a lo \
                 elegido. Si ese control o ese destino ya tenían asignación, se reemplaza.",
            )
            .clicked()
        {
            let AprendizajeMidi { destino: d } = &mut **ctx.aprendizaje_midi;
            *d = if aprendiendo { None } else { Some(destino) };
        }
        if destino.es_continuo() {
            ui.weak("conviene una perilla");
        }
    });
    ui.add_space(6.0);

    // --- Lo asignado ---
    let mut quitar = None;
    ui.label(format!(
        "Asignaciones ({})",
        ctx.proyecto.0.asignaciones_midi.len()
    ));
    egui::ScrollArea::vertical()
        .id_salt("asignaciones_midi")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, a) in ctx.proyecto.0.asignaciones_midi.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("✕").on_hover_text("Quitar").clicked() {
                        quitar = Some(i);
                    }
                    ui.add(
                        egui::Label::new(format!(
                            "{} → {}",
                            a.entrada.descripcion(),
                            a.destino.descripcion()
                        ))
                        .truncate(),
                    );
                });
            }
        });
    // Se escribe en el proyecto solo si se pidió quitar una.
    if let Some(i) = quitar {
        ctx.proyecto.0.asignaciones_midi.remove(i);
    }
}
