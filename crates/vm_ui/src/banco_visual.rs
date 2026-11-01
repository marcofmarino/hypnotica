//! Banco de pruebas VISUAL: dibuja las pantallas reales de la interfaz a una imagen (sin
//! ventana, con `egui_kittest`) para poder mirarlas, y mide dos cosas que a ojo se notan mal:
//!
//! - **Que nada se salga de su panel** (lo que se corta o se solapa).
//! - **Que el dibujo no tiemble**: dos cuadros seguidos, con el mouse quieto, tienen que ser
//!   idénticos píxel por píxel.
//!
//! Las imágenes quedan en `target/capturas_ui/`. Son pruebas `#[ignore]` porque necesitan una
//! GPU: `cargo test -p vm_ui banco_visual -- --ignored`.

#![allow(clippy::field_reassign_with_default)]
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use bevy_egui::{EguiUserTextures, egui};
use egui_kittest::Harness;
use vm_audio::{AplicacionesQueSuenan, SenalesDeAudio};
use vm_composicion::{ErroresDeCapas, Miniaturas, ProgresoDeCapas, TexturasDeCapas};
use vm_control::{
    AprendizajeMidi, ConfigMidi, ConfigOsc, DestinoOsc, EstadoMidi, EstadoOsc, PuertosMidi,
};
use vm_core::{ConfigClip, EstadoReproduccion, OrdenComposicion};
use vm_core::{ConfigSalida, Proyecto};
use vm_output::{PatronesActivos, Salida};
use vm_proyecto::ProyectoActual;
use vm_proyecto::RutaProyecto;
use vm_warp::{MostrarGuias, Seleccion};

use crate::composicion::{
    ContextoComposicion, DialogoArchivo, DialogoCarpeta, Dispositivos, Explorador,
    pestana_composicion,
};
use crate::editor::{
    ArrastreEntrada, ContextoEditor, EdicionSalida, ModoEdicion, Vista, pantalla_de_edicion,
    pantalla_de_proyector,
};
use crate::estilo::{self, EsquemaActual, Instalacion};
use crate::tema::Esquema;

/// Dónde se guardan las imágenes.
fn carpeta() -> String {
    let c = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/capturas_ui").to_string();
    std::fs::create_dir_all(&c).unwrap();
    c
}

/// El panel central con el mismo marco que la aplicación.
fn panel_central(ui: &mut egui::Ui, contenido: impl FnOnce(&mut egui::Ui)) {
    let marco = egui::Frame::central_panel(&ui.ctx().global_style())
        .inner_margin(egui::Margin::same(crate::MARGEN_CENTRAL));
    egui::CentralPanel::default()
        .frame(marco)
        .show(ui, contenido);
}

/// Una salida de mentira (sin ventana ni cámara): solo lo que la interfaz lee.
fn salida_de_prueba() -> Salida {
    Salida {
        id: "HDMI-1".into(),
        monitor: Entity::PLACEHOLDER,
        ventana: Entity::PLACEHOLDER,
        capa: 1,
        capa_guias: 1001,
        previa: Handle::default(),
        camara_previa: Entity::PLACEHOLDER,
        tamano: Vec2::new(1920.0, 1080.0),
    }
}

/// Un proyecto con una salida, una superficie y algunas máscaras y huecos para llenar las listas.
fn proyecto_de_prueba() -> Proyecto {
    let mut proyecto = Proyecto::default();
    let mut salida = ConfigSalida::nueva("HDMI-1".into(), 1920.0, 1080.0);
    salida.agregar_mascara(1920.0, 1080.0, 0.3);
    salida.agregar_mascara(1920.0, 1080.0, 0.2);
    salida.agregar_mascara(1920.0, 1080.0, 0.1);
    salida.superficies[0].agregar_hueco();
    // Tres superficies: para ver el orden de la lista (la de más arriba, primero).
    salida.agregar_superficie(1920.0, 1080.0);
    salida.agregar_superficie(1920.0, 1080.0);
    proyecto.salidas.push(salida);
    proyecto
}

/// Dibuja `contenido` con el estilo real de Hypnotica (tipografía, íconos y tema Tierra), deja
/// que el dibujo se asiente, guarda la imagen y comprueba que no tiembla.
fn capturar(nombre: &str, ancho: f32, alto: f32, mut contenido: impl FnMut(&mut egui::Ui)) {
    // Igual que la aplicación: las fuentes se instalan en dos pasos (uno por cuadro) y la
    // interfaz no se dibuja hasta terminar; después se aplica el tema.
    let mut paso = Instalacion::default();
    let mut esquema = EsquemaActual::default();
    esquema.esquema = Esquema::Tierra;
    // `HYPNOTICA_ESCALA` prueba con la escala de pantalla (1.25, 1.5…): con escalas fraccionarias
    // el redondeo a píxeles puede hacer que algo oscile.
    let escala: f32 = std::env::var("HYPNOTICA_ESCALA")
        .ok()
        .and_then(|e| e.parse().ok())
        .unwrap_or(1.0);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(ancho, alto))
        .with_pixels_per_point(escala)
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut esquema, ahora);
            contenido(ui);
        });
    // Cuadros de sobra para que todo se asiente (animaciones, tamaños de dos pasadas…).
    for _ in 0..40 {
        harness.step();
    }
    let a = harness.render().expect("no se pudo dibujar");
    // Con la interfaz asentada, cada cuadro tiene que ser UNA pasada de layout: si egui repite
    // el dibujo (porque algo cambió de tamaño a mitad de cuadro) y lo sigue haciendo, algo
    // oscila entre dos medidas.
    let antes = harness.ctx.cumulative_pass_nr();
    harness.step();
    let pasadas = harness.ctx.cumulative_pass_nr() - antes;
    assert!(
        pasadas == 1,
        "{nombre}: egui necesitó {pasadas} pasadas de layout en un cuadro ya asentado"
    );
    let b = harness.render().expect("no se pudo dibujar");
    a.save(format!("{}/{nombre}.png", carpeta())).unwrap();
    // Nada puede quedar fuera de la ventana (ni a la izquierda ni a la derecha): lo que se sale
    // se corta o se solapa con el vecino.
    let usado = harness.ctx.globally_used_rect();
    assert!(
        usado.min.x >= -0.5 && usado.max.x <= ancho + 0.5,
        "{nombre}: algo se sale de la ventana: usa {usado:?} y la ventana mide {ancho}"
    );
    let distintos = a.pixels().zip(b.pixels()).filter(|(x, y)| x != y).count();
    assert!(
        distintos == 0,
        "{nombre}: la imagen TIEMBLA, {distintos} píxeles cambian entre dos cuadros seguidos"
    );
}

/// Todo lo que necesita la pantalla de Salida, para dibujarla cuadro a cuadro.
struct EscenaDeSalida {
    vista: Vista,
    world: World,
    estado: SystemState<ResMut<'static, ProyectoActual>>,
    salida: Salida,
    seleccion: Seleccion,
    edicion: EdicionSalida,
    arrastre: ArrastreEntrada,
    guias: MostrarGuias,
    patrones: PatronesActivos,
    texturas: EguiUserTextures,
    capas: TexturasDeCapas,
}

impl EscenaDeSalida {
    fn nueva(modo: ModoEdicion) -> Self {
        let mut world = World::new();
        world.insert_resource(ProyectoActual(proyecto_de_prueba()));
        let estado: SystemState<ResMut<'static, ProyectoActual>> = SystemState::new(&mut world);
        let mut edicion = EdicionSalida::default();
        edicion.modo = modo;
        EscenaDeSalida {
            vista: Vista::Salida,
            world,
            estado,
            salida: salida_de_prueba(),
            seleccion: Seleccion {
                id: "HDMI-1".into(),
                superficie: 0,
                esquina: 0,
            },
            edicion,
            arrastre: ArrastreEntrada::default(),
            guias: MostrarGuias::default(),
            patrones: PatronesActivos::default(),
            texturas: EguiUserTextures::default(),
            capas: TexturasDeCapas::default(),
        }
    }

    /// Dibuja un cuadro con el mismo esqueleto que `dibujar_interfaz`: una barra arriba, la de
    /// monitores abajo y la pantalla adentro del panel central.
    fn dibujar(&mut self, ui: &mut egui::Ui) {
        let mut proyecto = self.estado.get_mut(&mut self.world).unwrap();
        let salidas = [&self.salida];
        let mut editor = ContextoEditor {
            texturas: &mut self.texturas,
            composicion: None,
            capas: &self.capas,
            seleccion: &mut self.seleccion,
            proyecto: &mut proyecto,
            guias: &mut self.guias,
            edicion: &mut self.edicion,
            arrastre_entrada: &mut self.arrastre,
            patrones: &mut self.patrones,
        };
        egui::Panel::top("barra_superior")
            .frame(
                egui::Frame::side_top_panel(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::symmetric(crate::MARGEN_CENTRAL, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    ui.label("Hypnotica");
                });
            });
        egui::Panel::bottom("barra_de_monitores")
            .frame(
                egui::Frame::side_top_panel(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::symmetric(crate::MARGEN_CENTRAL, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    ui.label("Monitores");
                });
            });
        panel_central(ui, |ui| {
            pantalla_de_edicion(ui, &salidas, &mut editor, self.vista);
        });
    }
}

/// La pantalla de Salida con el modo elegido, a un tamaño de ventana dado.
fn capturar_salida(nombre: &str, ancho: f32, alto: f32, modo: ModoEdicion) {
    let mut escena = EscenaDeSalida::nueva(modo);
    capturar(nombre, ancho, alto, |ui| escena.dibujar(ui));
}

/// Al achicar la ventana, el PRIMER cuadro con el tamaño nuevo tiene que verse igual que una
/// ventana que ya estaba de ese tamaño. Si no, durante el redimensionado la interfaz se ve rota.
#[test]
#[ignore = "necesita una GPU"]
fn al_cambiar_el_tamano_no_queda_un_cuadro_roto() {
    let mut escena = EscenaDeSalida::nueva(ModoEdicion::Esquinas);
    let mut paso = Instalacion::default();
    let mut esquema = EsquemaActual::default();
    esquema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 700.0))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut esquema, ahora);
            escena.dibujar(ui);
        });
    for _ in 0..40 {
        harness.step();
    }
    let mut fallos = Vec::new();
    for ancho in [1100.0, 1700.0, 900.0, 1404.0] {
        harness.set_size(egui::vec2(ancho, 700.0));
        // UN solo cuadro con el tamaño nuevo.
        harness.step();
        let primero = harness.render().unwrap();
        for _ in 0..40 {
            harness.step();
        }
        let asentado = harness.render().unwrap();
        let distintos = primero
            .pixels()
            .zip(asentado.pixels())
            .filter(|(x, y)| x != y)
            .count();
        primero
            .save(format!("{}/redimension_{ancho}_primero.png", carpeta()))
            .unwrap();
        if distintos > 0 {
            fallos.push(format!("{ancho} px: {distintos} píxeles distintos"));
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

#[test]
#[ignore = "necesita una GPU"]
fn salida_en_modo_mascara() {
    capturar_salida("salida_mascara", 1404.0, 700.0, ModoEdicion::Mascara);
}

#[test]
#[ignore = "necesita una GPU"]
fn salida_en_modo_esquinas() {
    capturar_salida("salida_esquinas", 1404.0, 700.0, ModoEdicion::Esquinas);
}

#[test]
#[ignore = "necesita una GPU"]
fn salida_en_modo_malla() {
    capturar_salida("salida_malla", 1404.0, 700.0, ModoEdicion::Malla);
}

#[test]
#[ignore = "necesita una GPU"]
fn salida_con_la_ventana_chica() {
    capturar_salida("salida_chica", 1000.0, 650.0, ModoEdicion::Esquinas);
}

#[test]
#[ignore = "necesita una GPU"]
fn pestana_proyector() {
    let mut world = World::new();
    world.insert_resource(ProyectoActual(proyecto_de_prueba()));
    let mut estado: SystemState<ResMut<ProyectoActual>> = SystemState::new(&mut world);
    let salida = salida_de_prueba();
    let mut seleccion = Seleccion {
        id: "HDMI-1".into(),
        superficie: 0,
        esquina: 0,
    };
    let mut edicion = EdicionSalida::default();
    let mut arrastre = ArrastreEntrada::default();
    let mut guias = MostrarGuias::default();
    let mut patrones = PatronesActivos::default();
    let mut texturas = EguiUserTextures::default();
    let capas = TexturasDeCapas::default();
    capturar("proyector", 1404.0, 700.0, |ui| {
        let mut proyecto = estado.get_mut(&mut world).unwrap();
        let salidas = [&salida];
        let mut editor = ContextoEditor {
            texturas: &mut texturas,
            composicion: None,
            capas: &capas,
            seleccion: &mut seleccion,
            proyecto: &mut proyecto,
            guias: &mut guias,
            edicion: &mut edicion,
            arrastre_entrada: &mut arrastre,
            patrones: &mut patrones,
        };
        pantalla_de_proyector(ui, &salidas, &mut editor);
    });
}

/// Recorre muchas alturas de ventana: una oscilación (por ejemplo, una barra de desplazamiento
/// que aparece y desaparece según el alto) solo se da en alguna de ellas.
#[test]
#[ignore = "necesita una GPU"]
fn salida_a_muchas_alturas() {
    let mut fallos = Vec::new();
    for alto in (420..=800).step_by(20) {
        for modo in [
            ModoEdicion::Esquinas,
            ModoEdicion::Mascara,
            ModoEdicion::Malla,
        ] {
            let nombre = format!("alto{alto}");
            let r =
                std::panic::catch_unwind(|| capturar_salida(&nombre, 1404.0, alto as f32, modo));
            if let Err(e) = r {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                fallos.push(format!("{alto} px: {msg}"));
            }
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// Recorre muchos anchos de ventana, de chica a grande.
#[test]
#[ignore = "necesita una GPU"]
fn salida_a_muchos_anchos() {
    let mut fallos = Vec::new();
    for ancho in [900, 1000, 1100, 1200, 1280, 1366, 1404, 1500, 1600, 1920] {
        for modo in [
            ModoEdicion::Esquinas,
            ModoEdicion::Mascara,
            ModoEdicion::Malla,
        ] {
            let nombre = format!("ancho{ancho}");
            let r =
                std::panic::catch_unwind(|| capturar_salida(&nombre, ancho as f32, 700.0, modo));
            if let Err(e) = r {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                fallos.push(format!("{ancho} px: {msg}"));
            }
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// Las pestañas y el desplegable del tema (lo que mencionaste que «tiembla»), con el mouse
/// encima de cada uno: el resaltado se anima, y los cuadros siguientes no tienen que cambiar de
/// tamaño ni de lugar.
#[test]
#[ignore = "necesita una GPU"]
fn pestanas_y_tema_no_tiemblan_con_el_mouse_encima() {
    use crate::{Pestana, barra_de_pestanas, selector_de_esquema};
    use egui_kittest::kittest::Queryable;
    let mut pestana = Pestana::Composicion;
    let mut esquema = Esquema::Tierra;
    let mut paso = Instalacion::default();
    let mut estado_de_tema = EsquemaActual::default();
    estado_de_tema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1404.0, 200.0))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut estado_de_tema, ahora);
            egui::Panel::top("barra").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Hypnotica");
                    ui.separator();
                    let _ = ui.button("Guardar");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        esquema = selector_de_esquema(ui, esquema);
                    });
                });
            });
            panel_central(ui, |ui| {
                pestana = barra_de_pestanas(ui, pestana);
            });
        });
    for _ in 0..30 {
        harness.step();
    }
    let mut fallos = Vec::new();
    for etiqueta in [
        "Composición",
        "Entrada",
        "Salida",
        "Proyector",
        "Control",
        "tema",
    ] {
        if etiqueta == "tema" {
            harness.get_by_role(egui::accesskit::Role::ComboBox).hover();
        } else {
            harness.get_by_label_contains(etiqueta).hover();
        }
        for _ in 0..60 {
            harness.step();
        }
        let a = harness.render().unwrap();
        harness.step();
        let b = harness.render().unwrap();
        let distintos = a.pixels().zip(b.pixels()).filter(|(x, y)| x != y).count();
        a.save(format!("{}/hover_{etiqueta}.png", carpeta()))
            .unwrap();
        if distintos > 0 {
            fallos.push(format!(
                "{etiqueta}: {distintos} píxeles cambian entre cuadros"
            ));
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// La pantalla de Composición completa (grilla de clips, vista previa, propiedades, clip y
/// archivos) con el mismo esqueleto que la app.
fn capturar_composicion(nombre: &str, ancho: f32, alto: f32) {
    escena_de_composicion(nombre, ancho, alto, AccionDelBanco::Capturar);
}

/// Con `medir_redimension`, en vez de capturar mide el costo de redimensionar la ventana cuadro a
/// cuadro (tiempo y pasadas de layout).
fn escena_de_composicion(nombre: &str, ancho: f32, alto: f32, accion: AccionDelBanco) {
    type Recursos = (
        ResMut<'static, ProyectoActual>,
        ResMut<'static, ConfigOsc>,
        ResMut<'static, DestinoOsc>,
        ResMut<'static, ConfigMidi>,
        ResMut<'static, AprendizajeMidi>,
        ResMut<'static, PuertosMidi>,
    );
    let mut world = World::new();
    let mut proyecto = proyecto_de_prueba();
    proyecto.composicion.agregar_capa();
    proyecto.composicion.poner_clip(
        0,
        0,
        ConfigClip::de_camara("/dev/video0".into(), "Captura".into()),
    );
    proyecto
        .composicion
        .poner_clip(1, 0, ConfigClip::de_video("/no/existe/video.mp4".into()));
    proyecto
        .composicion
        .poner_clip(1, 2, ConfigClip::de_imagen("/no/existe/foto.png".into()));
    world.insert_resource(ProyectoActual(proyecto));
    world.insert_resource(ConfigOsc::default());
    world.insert_resource(DestinoOsc::default());
    world.insert_resource(ConfigMidi::default());
    world.insert_resource(AprendizajeMidi::default());
    world.insert_resource(PuertosMidi::default());
    let mut estado: SystemState<Recursos> = SystemState::new(&mut world);

    let mut capa_elegida = 0usize;
    // Un clip de video elegido: así se ven «Bucle» y «Silencio», que antes chocaban de id.
    let mut clip_elegido: Option<(usize, usize)> = Some((1, 0));
    let mut explorador = Explorador::default();
    let mut dispositivos = Dispositivos::default();
    let progreso = ProgresoDeCapas::default();
    let estado_osc = EstadoOsc::default();
    let estado_midi = EstadoMidi::default();
    let senales = SenalesDeAudio::default();
    let mut aplicaciones = AplicacionesQueSuenan::default();
    aplicaciones.buscadas = true;
    let errores = ErroresDeCapas::default();
    let mut dialogo_carpeta = DialogoCarpeta::default();
    let capas_texturas = TexturasDeCapas::default();
    let mut quitadas = Vec::new();
    let mut movidas = Vec::new();
    let mut texturas = EguiUserTextures::default();
    let ruta = RutaProyecto(std::env::temp_dir().join("hypnotica_banco/proyecto.ron"));
    let miniaturas = Miniaturas::default();
    let mut reproduccion = EstadoReproduccion::default();
    let mut dialogo = DialogoArchivo::default();
    let mut ordenes: Vec<OrdenComposicion> = Vec::new();

    let dibujar = |ui: &mut egui::Ui| {
        let (mut proyecto, mut osc, mut destino, mut midi, mut aprendizaje, mut puertos) =
            estado.get_mut(&mut world).unwrap();
        let mut contexto = ContextoComposicion {
            capa_elegida: &mut capa_elegida,
            clip_elegido: &mut clip_elegido,
            explorador: &mut explorador,
            dispositivos: &mut dispositivos,
            progreso: &progreso,
            config_osc: &mut osc,
            estado_osc: &estado_osc,
            destino_osc: &mut destino,
            config_midi: &mut midi,
            estado_midi: &estado_midi,
            puertos_midi: &mut puertos,
            aprendizaje_midi: &mut aprendizaje,
            senales_audio: &senales,
            aplicaciones_audio: &mut aplicaciones,
            errores: &errores,
            dialogo_carpeta: &mut dialogo_carpeta,
            capas_texturas: &capas_texturas,
            capas_quitadas: &mut quitadas,
            capas_movidas: &mut movidas,
            texturas: &mut texturas,
            textura: None,
            proyecto: &mut proyecto,
            ruta: &ruta,
            miniaturas: &miniaturas,
            reproduccion: &mut reproduccion,
            mensaje: "",
            dialogo: &mut dialogo,
            ordenes: &mut ordenes,
        };
        egui::Panel::top("barra_superior")
            .frame(
                egui::Frame::side_top_panel(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::symmetric(crate::MARGEN_CENTRAL, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    ui.label("Hypnotica");
                });
            });
        egui::Panel::bottom("barra_de_monitores")
            .frame(
                egui::Frame::side_top_panel(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::symmetric(crate::MARGEN_CENTRAL, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    ui.label("Monitores");
                });
            });
        panel_central(ui, |ui| {
            pestana_composicion(ui, &mut contexto);
        });
    };
    match accion {
        AccionDelBanco::Capturar => capturar(nombre, ancho, alto, dibujar),
        AccionDelBanco::MedirRedimension => medir_el_costo_de_redimensionar(ancho, alto, dibujar),
        AccionDelBanco::ArrastrarDivisores => probar_los_divisores(ancho, alto, dibujar),
        AccionDelBanco::PrimerCuadroTrasRedimensionar => {
            probar_el_primer_cuadro_tras_redimensionar(alto, dibujar)
        }
    }
}

/// Qué hace el banco con la escena de Composición.
#[derive(Clone, Copy)]
enum AccionDelBanco {
    Capturar,
    MedirRedimension,
    ArrastrarDivisores,
    PrimerCuadroTrasRedimensionar,
}

/// Arma el harness con el estilo real (las fuentes se instalan dentro del dibujo, como en la app).
fn armar<'a>(ancho: f32, alto: f32, mut contenido: impl FnMut(&mut egui::Ui) + 'a) -> Harness<'a> {
    let mut paso = Instalacion::default();
    let mut esquema = EsquemaActual::default();
    esquema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(ancho, alto))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut esquema, ahora);
            contenido(ui);
        });
    for _ in 0..30 {
        harness.step();
    }
    harness
}

/// Arrastra cada divisor con el mouse y comprueba que la columna de la derecha se corre lo que
/// se arrastró (y que las demás no se mueven).
fn probar_los_divisores(ancho: f32, alto: f32, dibujar: impl FnMut(&mut egui::Ui)) {
    use egui_kittest::kittest::Queryable;
    let mut harness = armar(ancho, alto, dibujar);
    let x_de = |h: &Harness<'_>, etiqueta: &str| h.get_by_label(etiqueta).rect().min.x;
    // «Visible» está en la columna de propiedades; «Volumen» en la del clip.
    let propiedades_antes = x_de(&harness, "Visible");
    let clip_antes = x_de(&harness, "Volumen");
    // El divisor entre previsualizaciones y propiedades queda MARGEN a la izquierda del contenido.
    let divisor = egui::pos2(
        propiedades_antes - crate::composicion::MARGEN_DE_COLUMNA,
        alto * 0.85,
    );
    harness.hover_at(divisor);
    harness.step();
    harness.drag_at(divisor);
    harness.step();
    for paso in 1..=4 {
        harness.hover_at(divisor + egui::vec2(15.0 * paso as f32, 0.0));
        harness.step();
    }
    harness.drop_at(divisor + egui::vec2(60.0, 0.0));
    for _ in 0..5 {
        harness.step();
    }
    let propiedades_despues = x_de(&harness, "Visible");
    let clip_despues = x_de(&harness, "Volumen");
    assert!(
        (propiedades_despues - propiedades_antes - 60.0).abs() < 2.0,
        "el divisor se arrastró 60 px y las propiedades se corrieron {}",
        propiedades_despues - propiedades_antes
    );
    // La columna del clip se queda donde estaba (solo cambia el ancho de las dos vecinas).
    assert!(
        (clip_despues - clip_antes).abs() < 2.0,
        "la columna del clip se movió {}",
        clip_despues - clip_antes
    );
}

/// El PRIMER cuadro con el tamaño nuevo de la ventana tiene que verse igual que una ventana que
/// ya estaba de ese tamaño: si no, al arrastrar el borde con el mouse la interfaz va un cuadro
/// atrasada y se siente «pesada».
fn probar_el_primer_cuadro_tras_redimensionar(alto: f32, dibujar: impl FnMut(&mut egui::Ui)) {
    let mut harness = armar(1500.0, alto, dibujar);
    let mut fallos = Vec::new();
    for ancho in [1100.0, 1700.0, 950.0, 1404.0] {
        harness.set_size(egui::vec2(ancho, alto));
        harness.step();
        let primero = harness.render().unwrap();
        for _ in 0..40 {
            harness.step();
        }
        let asentado = harness.render().unwrap();
        let distintos = primero
            .pixels()
            .zip(asentado.pixels())
            .filter(|(x, y)| x != y)
            .count();
        if distintos > 0 {
            fallos.push(format!("{ancho} px: {distintos} píxeles distintos"));
            primero
                .save(format!("{}/redim_primero_{ancho}_a.png", carpeta()))
                .unwrap();
            asentado
                .save(format!("{}/redim_primero_{ancho}_b.png", carpeta()))
                .unwrap();
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// La pantalla de Entrada (el recorte de la composición que toma la superficie).
#[test]
#[ignore = "necesita una GPU"]
fn pantalla_de_entrada() {
    let mut escena = EscenaDeSalida::nueva(ModoEdicion::Esquinas);
    escena.vista = Vista::Entrada;
    capturar("entrada", 1404.0, 700.0, |ui| escena.dibujar(ui));
}

/// Lo que se ve en `temblor-1.mp4`: al pasar el mouse sobre una pestaña, esa pestaña crece y
/// empuja a las demás. Ningún widget puede cambiar de lugar ni de tamaño por el hover: se mide
/// el rectángulo de cada pestaña sin mouse y con el mouse sobre cada una.
#[test]
#[ignore = "necesita una GPU"]
fn el_hover_no_mueve_a_las_pestanas_vecinas() {
    use crate::{Pestana, barra_de_pestanas};
    use egui_kittest::kittest::Queryable;
    let nombres = ["Composición", "Entrada", "Salida", "Proyector", "Control"];
    let mut pestana = Pestana::Composicion;
    let mut paso = Instalacion::default();
    let mut estado_de_tema = EsquemaActual::default();
    estado_de_tema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 120.0))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut estado_de_tema, ahora);
            panel_central(ui, |ui| {
                pestana = barra_de_pestanas(ui, pestana);
            });
        });
    for _ in 0..30 {
        harness.step();
    }
    let rects = |h: &Harness<'_, ()>| -> Vec<egui::Rect> {
        nombres
            .iter()
            .map(|n| h.get_by_label_contains(n).rect())
            .collect()
    };
    let en_reposo = rects(&harness);
    let mut fallos = Vec::new();
    for (i, n) in nombres.iter().enumerate() {
        harness.get_by_label_contains(n).hover();
        for _ in 0..20 {
            harness.step();
        }
        let con_hover = rects(&harness);
        for (j, (a, b)) in en_reposo.iter().zip(&con_hover).enumerate() {
            let mueve = (a.min - b.min).abs().max_elem() > 0.01;
            let cambia = (a.size() - b.size()).abs().max_elem() > 0.01;
            if mueve || cambia {
                fallos.push(format!(
                    "con el mouse sobre «{n}», «{}» pasó de {a:?} a {b:?}",
                    nombres[j]
                ));
            }
        }
        let _ = i;
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// Lo que se ve en `temblor-2.mp4`: con el desplegable abierto, pasar el mouse sobre una opción
/// no puede mover ni agrandar a las demás (ni al menú entero).
#[test]
#[ignore = "necesita una GPU"]
fn el_hover_no_mueve_las_opciones_de_un_desplegable() {
    use crate::selector_de_esquema;
    use egui_kittest::kittest::Queryable;
    let opciones = ["Tierra", "Gruvbox", "Catppuccin", "Nord", "Caelestia"];
    let mut esquema = Esquema::Tierra;
    let mut paso = Instalacion::default();
    let mut estado_de_tema = EsquemaActual::default();
    estado_de_tema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(500.0, 300.0))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut estado_de_tema, ahora);
            panel_central(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    esquema = selector_de_esquema(ui, esquema);
                });
            });
        });
    for _ in 0..30 {
        harness.step();
    }
    // El botón del desplegable también tiene que quedarse quieto al pasarle el mouse.
    let boton_antes = harness.get_by_role(egui::accesskit::Role::ComboBox).rect();
    harness.get_by_role(egui::accesskit::Role::ComboBox).hover();
    for _ in 0..20 {
        harness.step();
    }
    let boton_despues = harness.get_by_role(egui::accesskit::Role::ComboBox).rect();
    assert_eq!(
        boton_antes, boton_despues,
        "el botón del desplegable crece con el hover"
    );

    harness.get_by_role(egui::accesskit::Role::ComboBox).click();
    for _ in 0..30 {
        harness.step();
    }
    let rects = |h: &Harness<'_, ()>| -> Vec<egui::Rect> {
        opciones
            .iter()
            .map(|n| h.get_by_label_contains(n).rect())
            .collect()
    };
    let en_reposo = rects(&harness);
    let mut fallos = Vec::new();
    for n in opciones {
        harness.get_by_label_contains(n).hover();
        for _ in 0..20 {
            harness.step();
        }
        for (j, (a, b)) in en_reposo.iter().zip(rects(&harness)).enumerate() {
            if (a.min - b.min).abs().max_elem() > 0.01
                || (a.size() - b.size()).abs().max_elem() > 0.01
            {
                fallos.push(format!(
                    "con el mouse sobre «{n}», «{}» pasó de {a:?} a {b:?}",
                    opciones[j]
                ));
            }
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

/// La barra de superficie en una ventana angosta: los grupos tienen que bajar enteros.
#[test]
#[ignore = "necesita una GPU"]
fn salida_angosta() {
    capturar_salida("salida_angosta", 860.0, 650.0, ModoEdicion::Esquinas);
}

#[test]
#[ignore = "necesita una GPU"]
fn pantalla_de_composicion() {
    // El tamaño de la ventana de las proporciones que eligió Marco (`proporciones.png`).
    capturar_composicion("composicion", 1520.0, 929.0);
}

/// Una ventana grande: el espacio de sobra tiene que aprovecharse.
#[test]
#[ignore = "necesita una GPU"]
fn pantalla_de_composicion_grande() {
    capturar_composicion("composicion_grande", 1920.0, 1080.0);
}

/// Redimensiona la ventana cuadro a cuadro (como al arrastrar el borde) y mide cuánto tarda cada
/// cuadro y cuántas pasadas de layout hace egui: si repite el dibujo (por algo que cambia de
/// tamaño y no se asienta) el costo se duplica y la interfaz «se arrastra».
fn medir_el_costo_de_redimensionar(
    ancho: f32,
    alto: f32,
    mut contenido: impl FnMut(&mut egui::Ui),
) {
    let mut paso = Instalacion::default();
    let mut esquema = EsquemaActual::default();
    esquema.esquema = Esquema::Tierra;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(ancho, alto))
        .wgpu()
        .build_ui(move |ui| {
            if paso != Instalacion::Lista {
                estilo::avanzar(ui.ctx(), &mut paso);
                ui.ctx().request_repaint();
                return;
            }
            let ahora = ui.input(|i| i.time);
            estilo::mantener_tema(ui.ctx(), &mut esquema, ahora);
            contenido(ui);
        });
    for _ in 0..30 {
        harness.step();
    }
    let cuadros = 120;
    let pasadas_antes = harness.ctx.cumulative_pass_nr();
    let inicio = std::time::Instant::now();
    for i in 0..cuadros {
        // El ancho oscila entre 900 y 1900 px, como al arrastrar el borde de la ventana.
        let nuevo = 900.0 + 1000.0 * ((i as f32 * 0.15).sin() * 0.5 + 0.5);
        harness.set_size(egui::vec2(nuevo, alto));
        harness.step();
    }
    let por_cuadro = inicio.elapsed().as_secs_f64() * 1000.0 / cuadros as f64;
    let pasadas = (harness.ctx.cumulative_pass_nr() - pasadas_antes) as f64 / cuadros as f64;
    eprintln!("REDIMENSION: {por_cuadro:.2} ms por cuadro, {pasadas:.2} pasadas por cuadro");
    assert!(
        pasadas <= 1.05,
        "al redimensionar, egui repite el dibujo: {pasadas:.2} pasadas por cuadro"
    );
}

#[test]
#[ignore = "necesita una GPU"]
fn redimensionar_composicion_no_repite_el_dibujo() {
    escena_de_composicion(
        "redim_composicion",
        1400.0,
        800.0,
        AccionDelBanco::MedirRedimension,
    );
}

/// Los divisores de la zona de abajo se arrastran con el mouse.
#[test]
#[ignore = "necesita una GPU"]
fn los_divisores_de_composicion_se_arrastran() {
    escena_de_composicion(
        "divisores",
        1404.0,
        782.0,
        AccionDelBanco::ArrastrarDivisores,
    );
}

/// Redimensionar la ventana no deja un cuadro roto ni atrasado.
#[test]
#[ignore = "necesita una GPU"]
fn al_redimensionar_composicion_el_primer_cuadro_ya_es_el_bueno() {
    escena_de_composicion(
        "redim_primero",
        1500.0,
        782.0,
        AccionDelBanco::PrimerCuadroTrasRedimensionar,
    );
}
