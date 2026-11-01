//! Interfaz de la ventana de control, con egui (`bevy_egui`).
//!
//! egui es una librería de interfaz de "modo inmediato": en cada frame se vuelve
//! a describir la pantalla entera con código (`if ui.button("X").clicked() { ... }`)
//! en lugar de crear widgets una vez y guardarlos.
//!
//! Estructura de la ventana (inspirada en Resolume):
//! - Barra superior: guardar y cargar.
//! - Panel izquierdo: monitores y salidas.
//! - Centro, con pestañas: **Composición** (vista previa y, más adelante, la grilla de
//!   capas y clips) y **Salida avanzada** (el editor de mallas).

mod animacion;
#[cfg(test)]
mod banco_visual;
mod composicion;
mod control;
mod conversion;
mod editor;
mod entrada;
mod escala;
mod estilo;
mod explorador;
mod inspector;
mod midi;
mod parametro;
mod proporcion;
#[cfg(test)]
mod pruebas_egui;
mod reparto;
mod seleccion;
mod tema;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::Monitor;
use bevy_egui::egui;
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, EguiUserTextures,
    PrimaryEguiContext,
};
use composicion::{
    CapaElegida, ClipElegido, ContextoComposicion, DialogoArchivo, DialogoCarpeta, Dispositivos,
    Explorador, pestana_composicion, recibir_archivo_elegido, recibir_carpeta_de_reubicacion,
};
use editor::{
    ArrastreEntrada, ContextoEditor, EdicionSalida, Vista, pantalla_de_edicion,
    pantalla_de_proyector,
};
use estilo::{amarillo, azul, con_icono, icono, naranja, rosa, texto_negrita, verde};
use vm_audio::{AplicacionesQueSuenan, SenalesDeAudio};
use vm_composicion::{
    CapaMovida, CapaQuitada, ErroresDeCapas, MensajeComposicion, Miniaturas, OrdenClip,
    ProgresoDeCapas, Reproduccion, TexturaComposicion, TexturasDeCapas,
};
use vm_control::{
    AprendizajeMidi, ConfigMidi, ConfigOsc, DestinoOsc, EstadoMidi, EstadoOsc, PuertosMidi,
};
use vm_core::OrdenComposicion;
use vm_output::VistasPreviasNecesarias;
use vm_output::{Control, PatronesActivos, Salida, UbicacionControl, id_de_monitor};
use vm_proyecto::{
    EstadoProyecto, HistorialProyecto, OrdenProyecto, ProyectoActual, RutaProyecto,
    SalidaPendiente, TecladoOcupado,
};
use vm_warp::{MostrarGuias, Seleccion};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // Apagamos la creación automática del contexto de egui: con varias ventanas
        // queremos que la interfaz esté SOLO en la ventana de control, y no en las salidas.
        app.insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .add_plugins(EguiPlugin::default())
        .init_resource::<estilo::Instalacion>()
        .init_resource::<estilo::EsquemaActual>()
        .insert_resource(Pestana::inicial())
        .init_resource::<EdicionSalida>()
        .init_resource::<ArrastreEntrada>()
        .init_resource::<DialogoArchivo>()
        .init_resource::<CapaElegida>()
        .init_resource::<ClipElegido>()
        .init_resource::<Explorador>()
        .init_resource::<Dispositivos>()
        .init_resource::<DialogoCarpeta>()
        .add_systems(
            Update,
            (
                activar_egui_en_control,
                recibir_archivo_elegido,
                recibir_carpeta_de_reubicacion,
            ),
        )
        .add_systems(
            Update,
            oscilar_la_ventana
                .run_if(|| std::env::var_os("HYPNOTICA_DIAGNOSTICO_REDIMENSION").is_some()),
        )
        // Los sistemas de interfaz corren en su propio schedule, que egui prepara cada frame.
        .add_systems(
            EguiPrimaryContextPass,
            (instalar_estilo, dibujar_interfaz.run_if(estilo_instalado)).chain(),
        );
    }
}

/// Qué pestaña del centro está abierta.
#[derive(Resource, Default, PartialEq, Clone, Copy)]
enum Pestana {
    #[default]
    Composicion,
    Entrada,
    Salida,
    /// Lo que es de cada proyector: patrón de prueba, color y borde suave.
    Proyector,
    /// MIDI y OSC.
    Control,
}

/// Margen interior del panel central (alrededor de las pestañas y de lo que muestran).
const MARGEN_CENTRAL: i8 = 12;

/// El desplegable del tema de colores (arriba a la derecha). Devuelve el esquema elegido.
fn selector_de_esquema(ui: &mut egui::Ui, actual: tema::Esquema) -> tema::Esquema {
    let mut elegido = actual;
    egui::ComboBox::from_id_salt("esquema_de_colores")
        .selected_text(con_icono(
            ui,
            egui_phosphor::regular::PALETTE,
            estilo::naranja(),
            actual.nombre(),
        ))
        .show_ui(ui, |ui| {
            for opcion in tema::Esquema::TODOS {
                crate::inspector::opcion(ui, &mut elegido, opcion, opcion.nombre());
            }
        })
        .response
        .on_hover_text(
            "Colores de la interfaz. «Caelestia (en vivo)» toma los colores de tu \
             escritorio y los sigue cuando cambian.",
        );
    elegido
}

/// La fila de pestañas de arriba (Composición, Entrada, Salida, Proyector, Control), con la línea
/// que se desliza bajo la elegida. Devuelve la pestaña elegida después de dibujarla.
fn barra_de_pestanas(ui: &mut egui::Ui, actual: Pestana) -> Pestana {
    use egui_phosphor::regular as icono_de;
    let pestanas = [
        (
            Pestana::Composicion,
            icono_de::STACK,
            naranja(),
            "Composición",
        ),
        (Pestana::Entrada, icono_de::CROP, azul(), "Entrada"),
        (Pestana::Salida, icono_de::MONITOR_PLAY, verde(), "Salida"),
        (
            Pestana::Proyector,
            icono_de::PROJECTOR_SCREEN,
            amarillo(),
            "Proyector",
        ),
        (
            Pestana::Control,
            icono_de::GAME_CONTROLLER,
            rosa(),
            "Control",
        ),
    ];
    let mut elegida = actual;
    ui.horizontal(|ui| {
        let mut rect_elegida: Option<egui::Rect> = None;
        for (pestana, glifo, color, nombre) in pestanas {
            let texto = con_icono(ui, glifo, color, nombre);
            let respuesta = crate::inspector::elegible(ui, actual == pestana, texto);
            if respuesta.clicked() {
                elegida = pestana;
            }
            if actual == pestana {
                rect_elegida = Some(respuesta.rect);
            }
        }
        // La línea de abajo se desliza de una pestaña a la otra.
        if let Some(rect) = rect_elegida {
            estilo::indicador_deslizante(ui, "indicador_de_pestana", rect);
        }
    });
    elegida
}

impl Pestana {
    /// ¿Esta pestaña muestra la vista previa de una salida? (Si no, esas cámaras no dibujan.)
    fn muestra_vistas_previas(self) -> bool {
        matches!(self, Pestana::Salida | Pestana::Proyector)
    }

    /// La pestaña con la que arranca: `Composición`, o la que pida `HYPNOTICA_PESTANA`
    /// (`composicion`, `entrada`, `salida` o `control`). Sirve para probar la app sin hacer clics.
    fn inicial() -> Pestana {
        match std::env::var("HYPNOTICA_PESTANA").as_deref() {
            Ok("entrada") => Pestana::Entrada,
            Ok("salida") => Pestana::Salida,
            Ok("proyector") => Pestana::Proyector,
            Ok("control") => Pestana::Control,
            _ => Pestana::Composicion,
        }
    }
}

/// Cuando aparece la cámara de la ventana de control, le agregamos `PrimaryEguiContext`:
/// esa marca le dice a `bevy_egui` "dibujá la interfaz en la ventana de esta cámara".
fn activar_egui_en_control(mut commands: Commands, control: Query<Entity, Added<Control>>) {
    for camara in &control {
        commands.entity(camara).insert(PrimaryEguiContext);
    }
}

/// Todo lo que la interfaz necesita del proyecto, agrupado en un solo parámetro.
/// `#[derive(SystemParam)]` permite juntar varios parámetros de sistema en un struct:
/// el sistema recibe `DatosProyecto` y Bevy se encarga de pedir cada campo.
#[derive(SystemParam)]
struct DatosProyecto<'w> {
    proyecto: ResMut<'w, ProyectoActual>,
    estado: Res<'w, EstadoProyecto>,
    ruta: Res<'w, RutaProyecto>,
    ordenes: MessageWriter<'w, OrdenProyecto>,
    historial: Res<'w, HistorialProyecto>,
    salida_pendiente: Res<'w, SalidaPendiente>,
}

/// Los monitores conectados y dónde está la ventana de control.
#[derive(SystemParam)]
struct DatosMonitores<'w, 's> {
    monitores: Query<'w, 's, (Entity, &'static Monitor)>,
    ubicacion: UbicacionControl<'w, 's>,
}

/// Lo que necesita la pestaña de composición, además del proyecto.
#[derive(SystemParam)]
struct DatosComposicion<'w> {
    reproduccion: ResMut<'w, Reproduccion>,
    mensaje: Res<'w, MensajeComposicion>,
    capas: Res<'w, TexturasDeCapas>,
    progreso: Res<'w, ProgresoDeCapas>,
    config_osc: ResMut<'w, ConfigOsc>,
    estado_osc: Res<'w, EstadoOsc>,
    destino_osc: ResMut<'w, DestinoOsc>,
    config_midi: ResMut<'w, ConfigMidi>,
    estado_midi: Res<'w, EstadoMidi>,
    puertos_midi: ResMut<'w, PuertosMidi>,
    aprendizaje_midi: ResMut<'w, AprendizajeMidi>,
    senales_audio: Res<'w, SenalesDeAudio>,
    aplicaciones_audio: ResMut<'w, AplicacionesQueSuenan>,
    errores: Res<'w, ErroresDeCapas>,
    miniaturas: Res<'w, Miniaturas>,
    dialogo: ResMut<'w, DialogoArchivo>,
    ordenes: MessageWriter<'w, OrdenClip>,
    quitadas: MessageWriter<'w, CapaQuitada>,
    movidas: MessageWriter<'w, CapaMovida>,
    elegida: ResMut<'w, CapaElegida>,
    clip: ResMut<'w, ClipElegido>,
    explorador: ResMut<'w, Explorador>,
    dispositivos: ResMut<'w, Dispositivos>,
    dialogo_carpeta: ResMut<'w, DialogoCarpeta>,
}

/// Ajustes de la interfaz y de la visualización, y las salidas abiertas.
#[derive(SystemParam)]
struct Ajustes<'w, 's> {
    pestana: ResMut<'w, Pestana>,
    medicion: Local<'s, MedicionDeInterfaz>,
    previas: ResMut<'w, VistasPreviasNecesarias>,
    guias: ResMut<'w, MostrarGuias>,
    seleccion: ResMut<'w, Seleccion>,
    edicion: ResMut<'w, EdicionSalida>,
    arrastre_entrada: ResMut<'w, ArrastreEntrada>,
    patrones: ResMut<'w, PatronesActivos>,
    esquema: ResMut<'w, estilo::EsquemaActual>,
    teclado: ResMut<'w, TecladoOcupado>,
    salidas: Query<'w, 's, &'static Salida>,
}

/// Instala la tipografía y los íconos al arrancar (ver `estilo::Instalacion`).
fn instalar_estilo(
    mut contexto: Single<&mut EguiContext, With<PrimaryEguiContext>>,
    mut paso: ResMut<estilo::Instalacion>,
    mut esquema: ResMut<estilo::EsquemaActual>,
    tiempo: Res<Time>,
) {
    if *paso != estilo::Instalacion::Lista {
        let mut nuevo = *paso;
        estilo::avanzar(contexto.get_mut(), &mut nuevo);
        *paso = nuevo;
    } else {
        // Con el estilo instalado, se mantiene al día el tema de colores.
        estilo::mantener_tema(contexto.get_mut(), &mut esquema, tiempo.elapsed_secs_f64());
    }
}

/// La interfaz se dibuja recién cuando el estilo está instalado.
fn estilo_instalado(paso: Res<estilo::Instalacion>) -> bool {
    *paso == estilo::Instalacion::Lista
}

/// `HYPNOTICA_DIAGNOSTICO_UI=1` registra cada segundo cuántas veces se dibujó la interfaz, cuántas
/// veces cambió el espacio que queda para el centro entre dos dibujos seguidos (si cambia todo
/// el tiempo, algo oscila) y a qué escala va la pantalla. Sirve para cazar temblores.
fn diagnostico_de_interfaz() -> bool {
    use std::sync::OnceLock;
    static VALOR: OnceLock<bool> = OnceLock::new();
    *VALOR.get_or_init(|| std::env::var_os("HYPNOTICA_DIAGNOSTICO_UI").is_some())
}

/// Tiempo total (en nanosegundos) que llevó dibujar la interfaz desde el último registro.
static NANOS_DE_DIBUJO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Suma al total lo que tarda en dibujarse un cuadro (se suma al terminar la función, también
/// si sale antes).
struct Cronometro(std::time::Instant);

impl Drop for Cronometro {
    fn drop(&mut self) {
        NANOS_DE_DIBUJO.fetch_add(
            self.0.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// `HYPNOTICA_DIAGNOSTICO_REDIMENSION=1` cambia el ancho de la ventana de control cuadro a cuadro
/// (como al arrastrar su borde) para medir, junto con `HYPNOTICA_DIAGNOSTICO_UI`, cuánto cuesta.
fn oscilar_la_ventana(
    control: Query<&Control>,
    mut ventanas: Query<&mut Window>,
    tiempo: Res<Time>,
) {
    let Some(ventana) = control.iter().next().map(|c| c.ventana) else {
        return;
    };
    if let Ok(mut v) = ventanas.get_mut(ventana) {
        let t = tiempo.elapsed_secs();
        let ancho = 1000.0 + 700.0 * ((t * 2.0).sin() * 0.5 + 0.5);
        v.resolution.set(ancho, 760.0);
    }
}

/// Lo que va midiendo el diagnóstico de la interfaz.
#[derive(Default)]
struct MedicionDeInterfaz {
    desde: Option<std::time::Instant>,
    dibujos: u32,
    cambios: u32,
    ultimo_libre: Option<egui::Rect>,
}

fn dibujar_interfaz(
    // `Single` pide exactamente una entidad que cumpla el filtro; si todavía no hay
    // (la ventana de control no abrió), el sistema simplemente no corre.
    mut contexto: Single<&mut EguiContext, With<PrimaryEguiContext>>,
    mut texturas: ResMut<EguiUserTextures>,
    composicion: Option<Res<TexturaComposicion>>,
    mut datos: DatosProyecto,
    monitores: DatosMonitores,
    mut ajustes: Ajustes,
    mut clips: DatosComposicion,
) {
    let _cronometro = Cronometro(std::time::Instant::now());
    let ctx = contexto.get_mut();

    // En esta versión de egui los paneles se dibujan dentro de un `Ui` raíz: uno que
    // cubre toda la ventana, al fondo de las capas.
    let mut raiz = egui::Ui::new(
        ctx.clone(),
        "raiz".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    // La barra de arriba con aire: no queda pegada a la barra de título ni a la línea de abajo.
    let marco_superior = egui::Frame::side_top_panel(&ctx.global_style())
        .inner_margin(egui::Margin::symmetric(MARGEN_CENTRAL, 10));
    egui::Panel::top("barra_superior").frame(marco_superior).show(&mut raiz, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Hypnotica");
            ui.separator();
            use egui_phosphor::regular as icono_de;
            if ui
                .button(con_icono(ui, icono_de::FLOPPY_DISK, azul(), "Guardar"))
                .on_hover_text("Guardar en el archivo actual (Ctrl+S)")
                .clicked()
            {
                datos.ordenes.write(OrdenProyecto::Guardar);
            }
            if ui
                .button(con_icono(
                    ui,
                    icono_de::FLOPPY_DISK_BACK,
                    verde(),
                    "Guardar como…",
                ))
                .on_hover_text("Guardar con otro nombre o en otra carpeta (Ctrl+Shift+S)")
                .clicked()
            {
                datos.ordenes.write(OrdenProyecto::GuardarComo);
            }
            if ui
                .button(con_icono(ui, icono_de::FOLDER_OPEN, naranja(), "Abrir…"))
                .on_hover_text("Abrir otro proyecto (Ctrl+O)")
                .clicked()
            {
                datos.ordenes.write(OrdenProyecto::Cargar);
            }
            ui.separator();
            if ui
                .add_enabled(
                    datos.historial.0.puede_deshacer(),
                    egui::Button::new(icono(egui_phosphor::regular::ARROW_U_UP_LEFT, azul())),
                )
                .on_hover_text("Deshacer (Ctrl+Z)")
                .clicked()
            {
                datos.ordenes.write(OrdenProyecto::Deshacer);
            }
            if ui
                .add_enabled(
                    datos.historial.0.puede_rehacer(),
                    egui::Button::new(icono(egui_phosphor::regular::ARROW_U_UP_RIGHT, azul())),
                )
                .on_hover_text("Rehacer (Ctrl+Shift+Z)")
                .clicked()
            {
                datos.ordenes.write(OrdenProyecto::Rehacer);
            }
            ui.separator();
            let sin_guardar = datos.historial.0.sin_guardar(&datos.proyecto.0);
            let nombre = datos
                .ruta
                .0
                .file_name()
                .map_or_else(|| "proyecto".into(), |n| n.to_string_lossy().into_owned());
            if sin_guardar {
                ui.label(icono("●", naranja()))
                    .on_hover_text("Hay cambios sin guardar");
            }
            ui.label(&nombre)
                .on_hover_text(datos.ruta.0.display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Se lee una copia y solo se escribe si el usuario eligió otro esquema.
                let elegido = selector_de_esquema(ui, ajustes.esquema.esquema);
                if elegido != ajustes.esquema.esquema {
                    ajustes.esquema.esquema = elegido;
                }
            });
        });
        if !datos.estado.mensaje.is_empty() {
            ui.label(&datos.estado.mensaje);
        }
        if datos.estado.autoguardado_disponible {
            ui.horizontal(|ui| {
                ui.label(icono(egui_phosphor::regular::WARNING, estilo::amarillo()));
                ui.label(
                    "Se cerró sin guardar la última vez: hay un autoguardado más nuevo que el proyecto.",
                );
                if ui.button("Recuperarlo").clicked() {
                    datos.ordenes.write(OrdenProyecto::RecuperarAutoguardado);
                }
                if ui.button("Descartarlo").clicked() {
                    datos.ordenes.write(OrdenProyecto::DescartarAutoguardado);
                }
            });
        }
    });

    // Al salir con cambios sin guardar, se pregunta qué hacer.
    if datos.salida_pendiente.0 {
        egui::Window::new("¿Salir sin guardar?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Hay cambios sin guardar en el proyecto.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(con_icono(
                            ui,
                            egui_phosphor::regular::FLOPPY_DISK,
                            azul(),
                            "Guardar y salir",
                        ))
                        .clicked()
                    {
                        datos.ordenes.write(OrdenProyecto::GuardarYSalir);
                    }
                    if ui
                        .button(con_icono(
                            ui,
                            egui_phosphor::regular::SIGN_OUT,
                            estilo::rojo(),
                            "Salir sin guardar",
                        ))
                        .clicked()
                    {
                        datos.ordenes.write(OrdenProyecto::SalirSinGuardar);
                    }
                    if ui.button("Cancelar").clicked() {
                        datos.ordenes.write(OrdenProyecto::CancelarSalida);
                    }
                });
            });
    }

    let marco_inferior = egui::Frame::side_top_panel(&ctx.global_style())
        .inner_margin(egui::Margin::symmetric(MARGEN_CENTRAL, 6));
    egui::Panel::bottom("barra_de_monitores")
        .frame(marco_inferior)
        .resizable(false)
        .show(&mut raiz, |ui| {
            panel_monitores(
                ui,
                &mut datos.proyecto,
                &monitores.monitores,
                &monitores.ubicacion,
            );
        });

    if diagnostico_de_interfaz() {
        let libre = raiz.available_rect_before_wrap();
        ajustes.medicion.dibujos += 1;
        if ajustes
            .medicion
            .ultimo_libre
            .is_some_and(|antes| antes != libre)
        {
            ajustes.medicion.cambios += 1;
            if ajustes.medicion.cambios <= 5 {
                info!(
                    "UI: el espacio del centro cambió de {:?} a {libre:?}",
                    ajustes.medicion.ultimo_libre
                );
            }
        }
        ajustes.medicion.ultimo_libre = Some(libre);
        let desde = *ajustes
            .medicion
            .desde
            .get_or_insert_with(std::time::Instant::now);
        if desde.elapsed() >= std::time::Duration::from_secs(1) {
            let nanos = NANOS_DE_DIBUJO.swap(0, std::sync::atomic::Ordering::Relaxed);
            info!(
                "UI: {} dibujos/s · {:.2} ms por dibujo · {} cambios del centro · escala {:.2} · pasada {}",
                ajustes.medicion.dibujos,
                nanos as f64 / 1e6 / ajustes.medicion.dibujos.max(1) as f64,
                ajustes.medicion.cambios,
                ctx.pixels_per_point(),
                ctx.cumulative_pass_nr()
            );
            ajustes.medicion.desde = Some(std::time::Instant::now());
            ajustes.medicion.dibujos = 0;
            ajustes.medicion.cambios = 0;
        }
    }

    // Las vistas previas de las salidas solo se dibujan mientras se están viendo. Se compara antes
    // de escribir para no marcar el recurso como modificado en cada cuadro.
    let necesarias = ajustes.pestana.muestra_vistas_previas();
    if ajustes.previas.0 != necesarias {
        ajustes.previas.0 = necesarias;
    }

    // Órdenes de disparo pedidas por la grilla en este frame; se envían al terminar de dibujar.
    let mut ordenes_clip: Vec<OrdenComposicion> = Vec::new();
    let mut capas_quitadas: Vec<usize> = Vec::new();
    let mut capas_movidas: Vec<(usize, usize)> = Vec::new();

    // El panel central con más aire que el predeterminado: nada pegado a los bordes de la ventana.
    let marco_central = egui::Frame::central_panel(&ctx.global_style())
        .inner_margin(egui::Margin::same(MARGEN_CENTRAL));
    egui::CentralPanel::default()
        .frame(marco_central)
        .show(&mut raiz, |ui| {
            // Se lee una copia y solo se escribe si el usuario eligió otra pestaña.
            let elegida = barra_de_pestanas(ui, *ajustes.pestana);
            if elegida != *ajustes.pestana {
                *ajustes.pestana = elegida;
            }
            ui.separator();

            match *ajustes.pestana {
                Pestana::Composicion | Pestana::Control => {
                    let mut contexto = ContextoComposicion {
                        capa_elegida: &mut clips.elegida.0,
                        clip_elegido: &mut clips.clip.0,
                        explorador: &mut clips.explorador,
                        dispositivos: &mut clips.dispositivos,
                        progreso: &clips.progreso,
                        config_osc: &mut clips.config_osc,
                        estado_osc: &clips.estado_osc,
                        destino_osc: &mut clips.destino_osc,
                        config_midi: &mut clips.config_midi,
                        estado_midi: &clips.estado_midi,
                        puertos_midi: &mut clips.puertos_midi,
                        aprendizaje_midi: &mut clips.aprendizaje_midi,
                        senales_audio: &clips.senales_audio,
                        aplicaciones_audio: &mut clips.aplicaciones_audio,
                        errores: &clips.errores,
                        dialogo_carpeta: &mut clips.dialogo_carpeta,
                        capas_texturas: &clips.capas,
                        capas_quitadas: &mut capas_quitadas,
                        capas_movidas: &mut capas_movidas,
                        texturas: &mut texturas,
                        textura: composicion.as_deref(),
                        proyecto: &mut datos.proyecto,
                        ruta: &datos.ruta,
                        miniaturas: &clips.miniaturas,
                        reproduccion: &mut clips.reproduccion.0,
                        mensaje: &clips.mensaje.texto,
                        dialogo: &mut clips.dialogo,
                        ordenes: &mut ordenes_clip,
                    };
                    if *ajustes.pestana == Pestana::Control {
                        control::pestana_control(ui, &mut contexto);
                    } else {
                        pestana_composicion(ui, &mut contexto);
                    }
                }
                Pestana::Entrada | Pestana::Salida | Pestana::Proyector => {
                    let vista = if *ajustes.pestana == Pestana::Entrada {
                        Vista::Entrada
                    } else {
                        Vista::Salida
                    };
                    let en_proyector = *ajustes.pestana == Pestana::Proyector;
                    let salidas: Vec<&Salida> = ajustes.salidas.iter().collect();
                    let mut editor = ContextoEditor {
                        texturas: &mut texturas,
                        composicion: composicion.as_deref(),
                        capas: &clips.capas,
                        seleccion: &mut ajustes.seleccion,
                        proyecto: &mut datos.proyecto,
                        guias: &mut ajustes.guias,
                        edicion: &mut ajustes.edicion,
                        arrastre_entrada: &mut ajustes.arrastre_entrada,
                        patrones: &mut ajustes.patrones,
                    };
                    if en_proyector {
                        pantalla_de_proyector(ui, &salidas, &mut editor);
                    } else {
                        pantalla_de_edicion(ui, &salidas, &mut editor, vista);
                    }
                }
            }
        });

    for orden in ordenes_clip {
        clips.ordenes.write(OrdenClip(orden));
    }
    for indice in capas_quitadas {
        clips.quitadas.write(CapaQuitada(indice));
    }
    for (desde, hasta) in capas_movidas {
        clips.movidas.write(CapaMovida { desde, hasta });
    }

    // Se le avisa al resto del programa si la interfaz está usando el teclado (por ejemplo,
    // se está escribiendo en un campo de texto), para que los atajos no reaccionen.
    // Se compara antes de escribir para no marcar el recurso como modificado sin necesidad.
    let quiere_teclado = ctx.egui_wants_keyboard_input();
    if ajustes.teclado.0 != quiere_teclado {
        ajustes.teclado.0 = quiere_teclado;
    }
}

/// Alto de la barra de monitores (más que cualquiera de sus partes, para centrarlas).
const ALTO_BARRA: f32 = 34.0;

/// Los monitores en una barra horizontal (abajo de todo): cada uno con su resolución y la
/// casilla de su salida. Ocupa poco alto, así no molesta al editar.
fn panel_monitores(
    ui: &mut egui::Ui,
    proyecto: &mut ResMut<ProyectoActual>,
    monitores: &Query<(Entity, &Monitor)>,
    ubicacion: &UbicacionControl,
) {
    // De izquierda a derecha, como están en el escritorio.
    let mut lista: Vec<_> = monitores.iter().collect();
    lista.sort_by_key(|(_, m)| (m.physical_position.x, m.physical_position.y));
    let un_solo_monitor = lista.len() == 1;
    let monitor_control = ubicacion.monitor();

    // Todo en una fila de alto fijo y centrado: etiquetas, botones y recuadros comparten el
    // mismo eje (con alturas mezcladas, egui los dejaba a distintas alturas).
    ui.horizontal(|ui| {
        ui.set_min_height(ALTO_BARRA);
        ui.label(icono(egui_phosphor::regular::MONITOR, estilo::azul()));
        ui.label(texto_negrita("Monitores"));
        for (texto, valor) in [("Todos", true), ("Ninguno", false)] {
            if ui
                .add_sized([64.0, inspector::ALTO_FILA], egui::Button::new(texto))
                .on_hover_text("Activar o desactivar la salida de todos los monitores")
                .clicked()
            {
                // Solo se tocan las salidas de los monitores conectados.
                for (_, monitor) in &lista {
                    let id = id_de_monitor(monitor);
                    if let Some(cfg) = proyecto.0.salidas.iter_mut().find(|s| s.monitor == id) {
                        cfg.activa = valor;
                    }
                }
            }
        }
        ui.separator();

        for (entidad, monitor) in lista {
            let id = id_de_monitor(monitor);
            let es_control = monitor_control == Some(entidad) && !un_solo_monitor;

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::symmetric(8, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(inspector::ALTO_FILA);
                        ui.label(texto_negrita(&id));
                        let hz = monitor
                            .refresh_rate_millihertz
                            .map(|mhz| format!(" @ {:.0} Hz", mhz as f32 / 1000.0))
                            .unwrap_or_default();
                        ui.weak(format!(
                            "{}×{}{hz}",
                            monitor.physical_width, monitor.physical_height
                        ));
                        if es_control {
                            ui.label(icono(egui_phosphor::regular::APP_WINDOW, estilo::naranja()))
                                .on_hover_text("Acá está la ventana de control");
                        }

                        // Se lee una copia y solo se escribe en el proyecto si el usuario
                        // cambió la casilla: así el proyecto no se marca como modificado en cada frame.
                        let Some(activa) = proyecto
                            .0
                            .salidas
                            .iter()
                            .find(|s| s.monitor == id)
                            .map(|s| s.activa)
                        else {
                            ui.label("(preparando…)");
                            return;
                        };
                        let mut nueva = activa;
                        let texto = if un_solo_monitor {
                            "Salida (en ventana)"
                        } else {
                            "Salida (fullscreen)"
                        };
                        if ui.checkbox(&mut nueva, texto).changed()
                            && let Some(cfg) =
                                proyecto.0.salidas.iter_mut().find(|s| s.monitor == id)
                        {
                            cfg.activa = nueva;
                        }
                        if activa && es_control {
                            ui.label(icono(egui_phosphor::regular::WARNING, estilo::amarillo()))
                                .on_hover_text("La salida tapa la ventana de control");
                        }
                    });
                });
        }
    });
}

#[cfg(test)]
mod pruebas_de_pestanas {
    use super::*;

    #[test]
    fn solo_salida_y_proyector_necesitan_las_vistas_previas() {
        assert!(Pestana::Salida.muestra_vistas_previas());
        assert!(Pestana::Proyector.muestra_vistas_previas());
        assert!(!Pestana::Composicion.muestra_vistas_previas());
        assert!(!Pestana::Entrada.muestra_vistas_previas());
        assert!(!Pestana::Control.muestra_vistas_previas());
    }
}
