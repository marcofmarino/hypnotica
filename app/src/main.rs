use bevy::prelude::*;
use bevy::window::{ExitCondition, WindowPlugin};
use vm_audio::AudioPlugin;
use vm_composicion::ComposicionPlugin;
use vm_control::ControlPlugin;
use vm_output::SalidasPlugin;
use vm_proyecto::ProyectoPlugin;
use vm_ui::UiPlugin;
use vm_video::VideoPlugin;
use vm_warp::WarpPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            // Sin ventana principal: las ventanas las abre `SalidasPlugin`, una por monitor.
            primary_window: None,
            // Sin esto la app se cerraría al arrancar por no tener ventanas.
            exit_condition: ExitCondition::DontExit,
            // Cerrar una ventana no la cierra sola: es un pedido de salir, que el programa
            // confirma (si hay cambios sin guardar) antes de cerrar.
            close_when_requested: false,
            ..default()
        }))
        .add_plugins(ProyectoPlugin)
        .add_plugins(VideoPlugin)
        .add_plugins(AudioPlugin)
        .add_plugins(ControlPlugin)
        .add_plugins(ComposicionPlugin)
        .add_plugins(SalidasPlugin)
        .add_plugins(WarpPlugin)
        .add_plugins(UiPlugin)
        .run();
}
