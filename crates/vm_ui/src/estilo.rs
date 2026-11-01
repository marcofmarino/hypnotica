//! El aspecto de la interfaz: la tipografía (Rubik, con dos pesos) y los íconos (Phosphor).
//!
//! egui no sabe elegir el peso de una fuente variable, así que se incluyen dos copias
//! estáticas de Rubik generadas a partir de la variable: *Regular* (400) para el texto
//! común y *SemiBold* (600) para títulos y énfasis. Rubik es de licencia OFL (ver
//! `fuentes/OFL.txt`); Phosphor es MIT.

use std::sync::Arc;

use bevy_egui::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

use crate::animacion;
use crate::tema::{self, Esquema, Tema};

pub(crate) use crate::tema::{amarillo, azul, naranja, rojo, rosa, texto_suave, verde};

/// Va acercando un valor a su objetivo de a poco (suavizado exponencial): cada frame recorre
/// una fracción de lo que falta, así arranca rápido y frena suave, sin importar cuánto
/// haya que recorrer. `velocidad` es qué tan rápido (≈ 14 llega en un cuarto de segundo).
/// El valor actual se guarda en la memoria de egui bajo `id`.
pub(crate) fn seguir(ctx: &egui::Context, id: egui::Id, objetivo: f32, velocidad: f32) -> f32 {
    let actual = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(objetivo);
    let dt = ctx.input(|i| i.stable_dt).min(0.1);
    let nuevo = actual + (objetivo - actual) * (1.0 - (-velocidad * dt).exp());
    // Cuando ya está a menos de un píxel, se pega al objetivo y deja de pedir frames.
    let nuevo = if (objetivo - nuevo).abs() < 0.05 {
        objetivo
    } else {
        ctx.request_repaint();
        nuevo
    };
    ctx.data_mut(|d| d.insert_temp(id, nuevo));
    nuevo
}

/// Una línea de énfasis que se desliza hasta quedar bajo la pestaña elegida.
pub(crate) fn indicador_deslizante(ui: &egui::Ui, id: &str, pestana: egui::Rect) {
    let ctx = ui.ctx();
    let x0 = seguir(ctx, egui::Id::new((id, "x0")), pestana.left(), 16.0);
    let x1 = seguir(ctx, egui::Id::new((id, "x1")), pestana.right(), 16.0);
    let y = pestana.bottom() + 2.0;
    ui.painter().line_segment(
        [egui::pos2(x0, y), egui::pos2(x1, y)],
        egui::Stroke::new(2.5, tema::tema().primario),
    );
}

/// Un ícono (un carácter de Phosphor) con su color.
pub(crate) fn icono(glifo: &str, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(glifo).color(color)
}

/// Un texto precedido por un ícono de otro color (para botones y pestañas).
pub(crate) fn con_icono(
    ui: &egui::Ui,
    glifo: &str,
    color_del_icono: egui::Color32,
    texto: &str,
) -> egui::text::LayoutJob {
    let fuente = FontId::new(14.0, FontFamily::Proportional);
    let mut trabajo = egui::text::LayoutJob::default();
    trabajo.append(
        glifo,
        0.0,
        egui::text::TextFormat::simple(fuente.clone(), color_del_icono),
    );
    trabajo.append(
        &format!("  {texto}"),
        0.0,
        egui::text::TextFormat::simple(fuente, ui.visuals().text_color()),
    );
    trabajo
}

/// El nombre de la familia con más peso, para títulos y énfasis.
const NEGRITA: &str = "negrita";

/// La familia de letra con más peso (Rubik SemiBold, con los íconos de respaldo).
pub(crate) fn negrita() -> FontFamily {
    FontFamily::Name(NEGRITA.into())
}

/// Un texto con peso (para títulos de sección, nombres que se quieren destacar).
pub(crate) fn texto_negrita(texto: impl Into<String>) -> egui::RichText {
    egui::RichText::new(texto).family(negrita())
}

/// En qué paso va la instalación del estilo. `egui` aplica las fuentes nuevas recién en el
/// pase siguiente a `set_fonts`; si en ese mismo pase se pidiera una familia que todavía no
/// existe («negrita»), el programa se caería. Por eso se hace en dos tiempos y la interfaz
/// no se dibuja hasta que terminan.
#[derive(bevy::prelude::Resource, Default, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Instalacion {
    #[default]
    SinEmpezar,
    FuentesPedidas,
    Lista,
}

/// Da el siguiente paso de la instalación (uno por frame).
pub(crate) fn avanzar(ctx: &egui::Context, paso: &mut Instalacion) {
    match *paso {
        Instalacion::SinEmpezar => {
            instalar_fuentes(ctx);
            *paso = Instalacion::FuentesPedidas;
        }
        Instalacion::FuentesPedidas => {
            instalar_tamanos(ctx);
            *paso = Instalacion::Lista;
        }
        Instalacion::Lista => {}
    }
}

/// Instala las fuentes (se activan en el pase siguiente).
fn instalar_fuentes(ctx: &egui::Context) {
    let mut fuentes = FontDefinitions::default();

    fuentes.font_data.insert(
        "Rubik-Regular".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../fuentes/Rubik-Regular.ttf"
        ))),
    );
    fuentes.font_data.insert(
        "Rubik-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../fuentes/Rubik-SemiBold.ttf"
        ))),
    );

    // Los íconos de Phosphor quedan de respaldo de la familia común: donde el texto no
    // tiene un carácter (un ícono), toma la fuente de íconos.
    egui_phosphor::add_to_fonts(&mut fuentes, egui_phosphor::Variant::Regular);

    // Rubik pasa a ser la primera opción de la familia común (las demás quedan de respaldo).
    let comun = fuentes
        .families
        .entry(FontFamily::Proportional)
        .or_default();
    comun.insert(0, "Rubik-Regular".into());
    let respaldo: Vec<String> = comun.iter().skip(1).cloned().collect();

    // La familia con peso: SemiBold primero y lo mismo de respaldo.
    let mut con_peso = vec!["Rubik-SemiBold".to_string()];
    con_peso.extend(respaldo);
    fuentes.families.insert(negrita(), con_peso);

    ctx.set_fonts(fuentes);
}

/// Los tamaños de texto de cada estilo (ya con las fuentes disponibles).
fn instalar_tamanos(ctx: &egui::Context) {
    ctx.global_style_mut(|estilo| {
        let tamanos = [
            (TextStyle::Heading, FontId::new(19.0, negrita())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ];
        for (nombre, id) in tamanos {
            estilo.text_styles.insert(nombre, id);
        }
        aplicar_colores(estilo, &tema::tema());
    });
}

/// Los colores, bordes y espacios, a partir del tema elegido (ver `tema.rs`): paneles,
/// botones y campos con las capas de superficie del tema, selección y foco con su color
/// principal y esquinas redondeadas.
pub(crate) fn aplicar_colores(estilo: &mut egui::Style, t: &Tema) {
    use egui::{CornerRadius, Stroke};

    let redondeo = CornerRadius::same(4);
    let encima = mezclar(t.contenedor, t.primario, 0.18);
    let apretado = mezclar(t.contenedor, t.fondo, 0.5);

    let v = &mut estilo.visuals;
    v.dark_mode = t.oscuro;
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.campo; // campos de texto, deslizadores, fondo de listas
    v.faint_bg_color = mezclar(t.panel, t.contenedor, 0.5);
    v.selection.bg_fill = t.seleccion;
    v.selection.stroke = Stroke::new(1.0, t.primario);
    v.hyperlink_color = t.primario;
    v.override_text_color = None;

    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, mezclar(t.panel, t.borde, 0.6));
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.texto);
    v.widgets.noninteractive.corner_radius = redondeo;

    v.widgets.inactive.bg_fill = t.contenedor;
    v.widgets.inactive.weak_bg_fill = t.contenedor;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.borde);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.texto);
    v.widgets.inactive.corner_radius = redondeo;

    v.widgets.hovered.bg_fill = encima;
    v.widgets.hovered.weak_bg_fill = encima;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.primario);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, t.texto);
    v.widgets.hovered.corner_radius = redondeo;

    v.widgets.active.bg_fill = apretado;
    v.widgets.active.weak_bg_fill = apretado;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.primario);
    v.widgets.active.fg_stroke = Stroke::new(1.5, t.texto);
    v.widgets.active.corner_radius = redondeo;

    // Sin «expansión»: con el valor por defecto un widget crece 1 px por lado al pasar el mouse y,
    // como el layout usa ese tamaño, EMPUJA a los vecinos (las pestañas y los desplegables
    // «temblaban»). El resaltado se nota igual por el color y el borde.
    v.widgets.noninteractive.expansion = 0.0;
    v.widgets.inactive.expansion = 0.0;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.expansion = 0.0;

    v.widgets.open.bg_fill = encima;
    v.widgets.open.weak_bg_fill = encima;
    v.widgets.open.corner_radius = redondeo;

    estilo.spacing.item_spacing = egui::vec2(8.0, 6.0);
    estilo.spacing.button_padding = egui::vec2(8.0, 4.0);
    estilo.spacing.interact_size.y = 24.0;
    // Los cambios de color de los botones (al pasar el mouse) son una transición corta.
    estilo.animation_time = 0.14;
}

/// Mezcla dos colores: `t = 0` es `a` y `t = 1` es `b`.
fn mezclar(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    a.lerp_to_gamma(b, t)
}

/// El esquema de colores elegido, y lo que hace falta para mantenerlo al día: el tema se
/// vuelve a aplicar cuando se elige otro esquema o, con Caelestia, cuando cambia su archivo.
#[derive(bevy::prelude::Resource)]
pub(crate) struct EsquemaActual {
    pub esquema: Esquema,
    /// El esquema que se aplicó la última vez (para notar que se eligió otro).
    aplicado: Option<Esquema>,
    /// Cuándo se modificó por última vez el archivo de Caelestia que se aplicó.
    modificado: Option<std::time::SystemTime>,
    /// Segundos (del reloj de la app) de la última vez que se miró el archivo de Caelestia.
    ultima_mirada: f64,
    /// Una transición de color en curso: de qué tema, a cuál y cuándo empezó.
    transicion: Option<(Tema, Tema, f64)>,
}

impl Default for EsquemaActual {
    fn default() -> Self {
        EsquemaActual {
            esquema: tema::esquema_guardado(),
            aplicado: None,
            modificado: None,
            ultima_mirada: f64::NEG_INFINITY,
            transicion: None,
        }
    }
}

/// Cuánto dura la transición de color al cambiar de tema, en segundos.
const DURACION_DE_LA_TRANSICION: f64 = 0.45;

/// Cada cuántos segundos se mira si Caelestia cambió su esquema.
const SEGUNDOS_ENTRE_MIRADAS: f64 = 1.0;

/// Aplica el tema si hace falta. `ahora` son los segundos transcurridos de la app.
pub(crate) fn mantener_tema(ctx: &egui::Context, estado: &mut EsquemaActual, ahora: f64) {
    let eligieron_otro = estado.aplicado != Some(estado.esquema);
    let mut cambio_el_archivo = false;
    if estado.esquema == Esquema::Caelestia
        && ahora - estado.ultima_mirada >= SEGUNDOS_ENTRE_MIRADAS
    {
        estado.ultima_mirada = ahora;
        let hoy = tema::ruta_caelestia()
            .and_then(|r| std::fs::metadata(r).ok())
            .and_then(|m| m.modified().ok());
        cambio_el_archivo = hoy != estado.modificado;
        estado.modificado = hoy;
    }
    if eligieron_otro || cambio_el_archivo {
        if eligieron_otro {
            // Solo se recuerda lo que el usuario elige, no el esquema con el que arrancó.
            if estado.aplicado.is_some() {
                tema::guardar_esquema(estado.esquema);
            }
            // Al cambiar de esquema, el próximo archivo de Caelestia se lee de nuevo.
            estado.modificado = None;
        }
        let nuevo = estado.esquema.tema();
        if estado.aplicado.is_none() {
            // El primer tema se aplica de golpe: no hay de dónde venir.
            tema::poner(nuevo);
            ctx.global_style_mut(|estilo| aplicar_colores(estilo, &nuevo));
        } else {
            // Los siguientes pasan de un tema al otro con una transición de color.
            estado.transicion = Some((tema::tema(), nuevo, ahora));
        }
        estado.aplicado = Some(estado.esquema);
    }

    if let Some((desde, hacia, inicio)) = estado.transicion {
        let avance = animacion::progreso(inicio, ahora, DURACION_DE_LA_TRANSICION);
        let actual = tema::entre(&desde, &hacia, animacion::entrada_salida_cubica(avance));
        tema::poner(actual);
        ctx.global_style_mut(|estilo| aplicar_colores(estilo, &actual));
        if avance >= 1.0 {
            estado.transicion = None;
        } else {
            ctx.request_repaint();
        }
    }
}
