//! Pestaña "Salida avanzada": el editor de mallas, con dos vistas como en Resolume.
//!
//! - **Entrada** (izquierda): la composición, con el rectángulo que muestra la superficie.
//! - **Salida** (derecha): lo que sale por el proyector, con las esquinas de la superficie.
//!
//! Todo modifica el proyecto; las mallas y el shader se actualizan solos a partir de él.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_egui::{EguiTextureHandle, EguiUserTextures};
use vm_composicion::{TexturaComposicion, TexturasDeCapas, textura_de_origen};
use vm_core::{
    BRILLO_MAXIMO, BRILLO_MINIMO, BordeSuave, CONTRASTE_MAXIMO, CONTRASTE_MINIMO, ConfigSuperficie,
    CorreccionColor, ENTRADA_COMPLETA, Entrada, GAMMA_MAXIMO, GAMMA_MINIMO, GANANCIA_MAXIMA,
    GANANCIA_MINIMA, Malla, OrigenSuperficie, PUNTOS_MAXIMOS, PUNTOS_MINIMOS, PatronPrueba, Punto,
    contiene, es_simple, insertar_en_lado, lado_mas_cercano, normalizar_entrada,
    rectangulo_unitario, trasladar,
};
use vm_output::{PatronActivo, PatronesActivos, Salida};
use vm_proyecto::ProyectoActual;
use vm_warp::deformacion;
use vm_warp::homografia::{Homografia, es_convexo};
use vm_warp::{MostrarGuias, Seleccion};

use crate::conversion::{pantalla_a_salida, salida_a_pantalla};
use crate::entrada;
use crate::escala::{EscalaBase, formatear_porcentaje, interpretar_porcentaje};
use crate::estilo::{
    amarillo, azul, con_icono, icono, naranja, rojo, rosa, texto_negrita, texto_suave, verde,
};
use crate::inspector::{
    ALTO_FILA, boton, botones_iguales, campo_numerico, campo_numerico_con, deslizador,
    deslizador_con, desplegable, fila, fila_elegible, lista_de_elementos, seccion,
    seccion_con_ayuda,
};
use crate::inspector::{Flujo, ancho_de_texto, fila_con_etiqueta};
use crate::proporcion::{self, proporcion_de_entrada, tamano_de_entrada};
use crate::seleccion::{self, Asa};

/// Qué se edita en la vista de la salida.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ModoEdicion {
    /// Las cuatro esquinas de la superficie (y, arrastrando adentro, la superficie entera).
    #[default]
    Esquinas,
    /// Los puntos de la forma que recorta la superficie.
    Forma,
    /// Los puntos de control de la malla (*mesh warp*) que curva la superficie.
    Malla,
    /// Las máscaras de la salida: polígonos negros que tapan lo que no debe recibir luz.
    Mascara,
    /// Los huecos de la superficie: zonas que no se dibujan y que se deforman con ella.
    Hueco,
}

/// Qué se está arrastrando con el mouse sobre la vista de la salida.
#[derive(Clone, Copy)]
pub(crate) enum ArrastreSalida {
    /// La superficie entera; `ultimo` es la posición anterior del puntero, en coordenadas
    /// de la salida.
    Superficie { ultimo: [f32; 2] },
    /// Los puntos elegidos (`EdicionSalida::elegidos`); `ultimo` es la posición anterior
    /// del puntero, en coordenadas de la salida. Mueve uno solo o varios a la vez.
    Grupo { ultimo: [f32; 2] },
    /// Un marco de selección dibujado arrastrando sobre espacio vacío, en coordenadas de la
    /// salida.
    Marco { inicio: [f32; 2], actual: [f32; 2] },
}

/// El estado del editor de la salida: qué se edita, qué punto de la forma está elegido y
/// qué se está arrastrando.
#[derive(Resource, Default)]
pub(crate) struct EdicionSalida {
    pub(crate) modo: ModoEdicion,
    pub(crate) punto_forma: usize,
    pub(crate) arrastre: Option<ArrastreSalida>,
    /// El punto de la forma sobre el que se abrió el menú de clic derecho, mientras esté abierto.
    pub(crate) punto_menu: Option<usize>,
    /// El punto de control de la malla elegido.
    pub(crate) nodo_malla: Option<(usize, usize)>,
    /// La máscara de la salida que se está editando.
    pub(crate) mascara: usize,
    /// El hueco de la superficie que se está editando.
    pub(crate) hueco: usize,
    /// Los puntos elegidos (según el modo: esquinas, puntos de la forma o de la malla).
    /// Arrastrar uno de ellos mueve todo el grupo.
    pub(crate) elegidos: Vec<Asa>,
    /// La salida y la superficie a las que pertenece `elegidos`; si cambian, se limpia.
    pub(crate) elegidos_de: Option<(String, usize)>,
    /// El tamaño base y el porcentaje de la escala de la superficie elegida.
    pub(crate) escala: Option<EscalaBase>,
    /// El texto del campo de escala. Se aplica al confirmar, no mientras se escribe.
    pub(crate) texto_escala: String,
}

/// Qué se está haciendo con el mouse sobre el rectángulo de entrada.
#[derive(Clone, Copy)]
pub(crate) enum ModoEntrada {
    /// Se arrastra el rectángulo entero; `ultimo` es la posición anterior del puntero.
    Mover { ultimo: [f32; 2] },
    /// Se arrastran las esquinas elegidas (`ArrastreEntrada::elegidas`): una sola se redimensiona,
    /// varias se mueven juntas.
    Grupo { ultimo: [f32; 2] },
    /// Un marco de selección dibujado sobre espacio vacío, en coordenadas de la composición.
    Marco { inicio: [f32; 2], actual: [f32; 2] },
}

/// El estado del editor de la entrada: qué se arrastra y qué esquinas están elegidas.
#[derive(Resource, Default)]
pub(crate) struct ArrastreEntrada {
    pub(crate) modo: Option<ModoEntrada>,
    /// Índices de las esquinas elegidas, según `entrada::esquinas`.
    pub(crate) elegidas: Vec<usize>,
    /// La superficie a la que pertenece `elegidas`; si cambia, se limpia.
    pub(crate) elegidas_de: Option<(String, usize)>,
}

/// Margen (en píxeles) entre el borde del lienzo y la imagen: deja lugar para que los
/// puntos de las esquinas se vean enteros y se puedan agarrar.
const MARGEN_DEL_LIENZO: f32 = 18.0;

/// El lienzo de una vista: ocupa **todo** el espacio que queda, siente clics y arrastres
/// (también fuera de la imagen: sirve para dibujar un marco de selección desde el vacío) y
/// dibuja la imagen centrada, con su proporción. Devuelve la respuesta y el rectángulo donde
/// quedó la imagen.
fn lienzo(
    ui: &mut egui::Ui,
    textura: egui::TextureId,
    relacion: f32,
) -> (egui::Response, egui::Rect) {
    let disponible = ui.available_size().min(ui.max_rect().size());
    // Un mínimo chico: con la ventana muy angosta el lienzo no se sale de su lugar.
    let tamano = disponible.max(egui::vec2(40.0, 40.0));
    let (area, respuesta) = ui.allocate_exact_size(tamano, egui::Sense::click_and_drag());
    let libre = area.shrink(MARGEN_DEL_LIENZO);
    let ancho = libre.width().min(libre.height() * relacion).max(1.0);
    let rect_imagen =
        egui::Rect::from_center_size(area.center(), egui::vec2(ancho, ancho / relacion));
    ui.painter_at(area).image(
        textura,
        rect_imagen,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    (respuesta, rect_imagen)
}

/// Alto de la línea que hay entre el título y la imagen de cada vista (Entrada y Salida):
/// igual en las dos, para que las imágenes queden alineadas.
const ALTO_CABECERA: f32 = 26.0;

/// Dibuja el marco de selección entre dos puntos de pantalla.
fn dibujar_marco(pintor: &egui::Painter, a: egui::Pos2, b: egui::Pos2) {
    let marco = egui::Rect::from_two_pos(a, b);
    pintor.rect_filled(marco, 0.0, amarillo().gamma_multiply(0.12));
    pintor.rect_stroke(
        marco,
        0.0,
        egui::Stroke::new(1.5, amarillo()),
        egui::StrokeKind::Inside,
    );
}

/// Distancia en píxeles de pantalla a la que el mouse "agarra" una esquina.
const RADIO_AGARRE: f32 = 32.0;

/// Lo que necesitan los editores de la pestaña, agrupado para no pasar una decena de parámetros.
///
/// `proyecto` es el `ResMut` completo (no un `&mut ProyectoActual`): convertir un `ResMut`
/// en `&mut` marca el recurso como modificado en ese mismo instante. Pasando el `ResMut`,
/// el proyecto solo se marca cuando de verdad se escribe en él.
pub(crate) struct ContextoEditor<'a, 'w> {
    pub(crate) texturas: &'a mut EguiUserTextures,
    pub(crate) composicion: Option<&'a TexturaComposicion>,
    pub(crate) capas: &'a TexturasDeCapas,
    pub(crate) seleccion: &'a mut Seleccion,
    pub(crate) proyecto: &'a mut ResMut<'w, ProyectoActual>,
    pub(crate) guias: &'a mut MostrarGuias,
    pub(crate) edicion: &'a mut EdicionSalida,
    pub(crate) arrastre_entrada: &'a mut ArrastreEntrada,
    pub(crate) patrones: &'a mut PatronesActivos,
}

/// La superficie `indice` de la salida del monitor `id`.
fn superficie<'a>(
    proyecto: &'a ProyectoActual,
    id: &str,
    indice: usize,
) -> Option<&'a ConfigSuperficie> {
    proyecto
        .0
        .salidas
        .iter()
        .find(|s| s.monitor == id)?
        .superficies
        .get(indice)
}

fn superficie_mut<'a>(
    proyecto: &'a mut ProyectoActual,
    id: &str,
    indice: usize,
) -> Option<&'a mut ConfigSuperficie> {
    proyecto
        .0
        .salidas
        .iter_mut()
        .find(|s| s.monitor == id)?
        .superficies
        .get_mut(indice)
}

/// Índice de la esquina más cercana al puntero, si alguna está a menos de `RADIO_AGARRE`.
fn esquina_cercana(
    esquinas: &[[f32; 2]; 4],
    puntero: egui::Pos2,
    rect_min: [f32; 2],
    rect_tam: [f32; 2],
    salida_tam: [f32; 2],
) -> Option<usize> {
    esquinas
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let p = salida_a_pantalla(*e, rect_min, rect_tam, salida_tam);
            (i, (p[0] - puntero.x).hypot(p[1] - puntero.y))
        })
        .filter(|(_, distancia)| *distancia < RADIO_AGARRE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Cuál de las dos pantallas de edición se muestra.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Vista {
    /// Qué parte de la composición toma cada superficie.
    Entrada,
    /// Dónde y cómo cae cada superficie en el proyector.
    Salida,
}

/// Ancho del árbol de superficies (izquierda) y del panel de propiedades (derecha) cuando hay
/// lugar. Dependen solo del ancho de la ventana (nunca de lo que haya adentro), así que nada
/// cambia de tamaño al elegir otro modo o superficie; si la ventana se achica, se achican
/// ellos también para dejarle lugar al lienzo.
const ANCHO_ARBOL: f32 = 230.0;
const ANCHO_PROPIEDADES: f32 = 320.0;

/// Margen interior de los paneles y de la vista: nada queda pegado a un borde o a un divisor.
const MARGEN_DE_PANEL: i8 = 10;

/// Los anchos de los dos paneles laterales para una ventana de `disponible` píxeles.
fn anchos_de_paneles(disponible: f32) -> (f32, f32) {
    (
        (disponible * 0.2).clamp(140.0, ANCHO_ARBOL),
        (disponible * 0.3).clamp(220.0, ANCHO_PROPIEDADES),
    )
}

/// Pantalla de edición de la entrada o de la salida, con la disposición de Resolume:
/// un árbol de salidas y superficies a la izquierda, el lienzo grande al centro y las
/// propiedades de la superficie a la derecha.
///
/// Todo modifica el proyecto; las mallas y el shader se actualizan solos a partir de él.
pub(crate) fn pantalla_de_edicion(
    ui: &mut egui::Ui,
    salidas: &[&Salida],
    editor: &mut ContextoEditor,
    vista: Vista,
) {
    if salidas.is_empty() {
        ui.add_space(8.0);
        ui.label("No hay salidas activas. Tildá un monitor en el panel de la izquierda.");
        return;
    }

    // Salida elegida (en orden alfabético, igual que la tecla Tab). Se decide antes de
    // dibujar nada porque los paneles necesitan saberla.
    let mut ordenadas = salidas.to_vec();
    ordenadas.sort_by(|a, b| a.id.cmp(&b.id));
    if !ordenadas.iter().any(|s| s.id == editor.seleccion.id) {
        editor.seleccion.id = ordenadas[0].id.clone();
    }
    let Some(salida) = ordenadas.iter().find(|s| s.id == editor.seleccion.id) else {
        return;
    };
    // Si la superficie elegida ya no existe (se borró), se elige la última.
    let cantidad = editor
        .proyecto
        .0
        .salidas
        .iter()
        .find(|c| c.monitor == salida.id)
        .map_or(0, |c| c.superficies.len());
    if cantidad > 0 && editor.seleccion.superficie >= cantidad {
        editor.seleccion.superficie = cantidad - 1;
    }

    // Los paneles se dibujan antes que el centro para que egui les reserve su lugar.
    let (ancho_arbol, ancho_propiedades) = anchos_de_paneles(ui.available_width());
    egui::Panel::left("arbol_de_superficies")
        .resizable(false)
        .exact_size(ancho_arbol)
        .show(ui, |ui| {
            // El contenido no se pega al borde ni al divisor.
            egui::Frame::NONE
                .inner_margin(MARGEN_DE_PANEL)
                .show(ui, |ui| arbol_de_superficies(ui, editor, &ordenadas));
        });

    egui::Panel::right("propiedades_de_superficie")
        .resizable(false)
        .exact_size(ancho_propiedades)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(MARGEN_DE_PANEL)
                        .show(ui, |ui| {
                            // El ancho de lo que hay adentro no depende del contenido.
                            ui.set_width(ui.available_width());
                            match vista {
                                Vista::Salida => {
                                    // Lo que depende del modo elegido (cambia al elegir otro).
                                    herramientas_de_superficie(
                                        ui,
                                        editor,
                                        salida,
                                        Parte::Modo,
                                        None,
                                    );
                                }
                                Vista::Entrada => {
                                    datos_de_superficie(ui, editor, salida);
                                    herramientas_de_superficie(
                                        ui,
                                        editor,
                                        salida,
                                        Parte::Entrada,
                                        None,
                                    );
                                }
                            }
                        });
                });
        });

    // Un margen alrededor de la vista: el título no queda pegado al divisor.
    egui::Frame::NONE
        .inner_margin(MARGEN_DE_PANEL)
        .show(ui, |ui| {
            // Lo que se pase de su región se corta ahí, en vez de pisar a los paneles vecinos.
            let region = ui.max_rect().intersect(ui.clip_rect());
            ui.set_clip_rect(region);
            match vista {
                Vista::Entrada => editor_entrada(ui, editor, &salida.id),
                Vista::Salida => editor_salida(ui, editor, salida),
            }
        });
}

/// La pestaña «Proyector»: lo que es de cada proyector y NO cambia con lo que se selecciona en la
/// vista de salida (patrón de prueba, guías, calibración de color y el borde suave de las
/// superficies). Antes estaba en el inspector, mezclado con lo que sí cambia.
pub(crate) fn pantalla_de_proyector(
    ui: &mut egui::Ui,
    salidas: &[&Salida],
    editor: &mut ContextoEditor,
) {
    if salidas.is_empty() {
        ui.add_space(8.0);
        ui.label("No hay salidas activas. Tildá un monitor en la barra de abajo.");
        return;
    }
    let mut ordenadas = salidas.to_vec();
    ordenadas.sort_by(|a, b| a.id.cmp(&b.id));
    if !ordenadas.iter().any(|s| s.id == editor.seleccion.id) {
        editor.seleccion.id = ordenadas[0].id.clone();
    }
    let id = editor.seleccion.id.clone();

    // A la derecha, lo que sale por el proyector EN VIVO: cada opción que se toca a la izquierda
    // (patrón, color, borde suave) se ve ahí al instante.
    let ancho_vista = (ui.available_width() * 0.5).clamp(320.0, 820.0);
    egui::Panel::right("vista_del_proyector")
        .resizable(false)
        .exact_size(ancho_vista)
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(MARGEN_DE_PANEL)
                .show(ui, |ui| {
                    seccion(ui, &format!("Así sale por {id}"));
                    if let Some(salida) = ordenadas.iter().find(|s| s.id == id) {
                        let relacion = (salida.tamano.x / salida.tamano.y).max(0.1);
                        let ancho = ui.available_width();
                        let tamano = egui::vec2(ancho, ancho / relacion);
                        let textura = editor
                            .texturas
                            .add_image(EguiTextureHandle::Weak(salida.previa.id()));
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            textura, tamano,
                        )));
                    }
                });
        });

    egui::Frame::NONE
        .inner_margin(MARGEN_DE_PANEL)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Un ancho cómodo para leer: en una pantalla ancha, los deslizadores no se
                    // estiran de punta a punta.
                    ui.set_max_width(ANCHO_PAGINA);
                    fila(ui, "Proyector", |ui| {
                        let mut elegido = id.clone();
                        desplegable(ui, "proyector_elegido", &id, |ui| {
                            for s in &ordenadas {
                                crate::inspector::opcion(ui, &mut elegido, s.id.clone(), &s.id);
                            }
                        });
                        if elegido != id {
                            editor.seleccion.id = elegido;
                            editor.seleccion.superficie = 0;
                        }
                    });
                    let id = editor.seleccion.id.clone();
                    proyector_y_patrones(ui, editor, &id);
                    controles_de_color(ui, editor, &id);

                    // El borde suave es de una superficie: se elige cuál.
                    let nombres: Vec<String> = editor
                        .proyecto
                        .0
                        .salidas
                        .iter()
                        .find(|c| c.monitor == id)
                        .map(|c| {
                            c.superficies
                                .iter()
                                .enumerate()
                                .map(|(i, s)| s.nombre_visible(i))
                                .collect()
                        })
                        .unwrap_or_default();
                    if nombres.is_empty() {
                        return;
                    }
                    editor.seleccion.superficie =
                        editor.seleccion.superficie.min(nombres.len() - 1);
                    fila(ui, "Superficie", |ui| {
                        let actual = editor.seleccion.superficie;
                        desplegable(ui, "superficie_del_borde", &nombres[actual], |ui| {
                            for (i, n) in nombres.iter().enumerate().rev() {
                                if crate::inspector::elegible(ui, i == actual, n).clicked() {
                                    editor.seleccion.superficie = i;
                                }
                            }
                        });
                    });
                    controles_de_borde(ui, editor, &id);
                });
        });
}

/// Ancho máximo del contenido de la pestaña «Proyector».
const ANCHO_PAGINA: f32 = 560.0;

/// Lo que es del proyector entero (no de una superficie): el patrón de prueba y las guías.
fn proyector_y_patrones(ui: &mut egui::Ui, editor: &mut ContextoEditor, id: &str) {
    seccion(ui, "Patrón de prueba y guías");
    selector_de_patron(ui, editor, id);
    ui.checkbox(
        &mut editor.guias.en_salidas,
        "Mostrar guías de las esquinas en los proyectores",
    )
    .on_hover_text("Lo que se muestre en las salidas se proyecta. Ocultalas antes del evento.");
}

/// El borde suave (*edge blending*) de la superficie elegida: un deslizador por lado y la
/// potencia de la curva. El resultado se ve enseguida en la vista previa.
fn controles_de_borde(ui: &mut egui::Ui, editor: &mut ContextoEditor, id: &str) {
    let indice = editor.seleccion.superficie;
    let Some((mut borde, esquinas)) =
        superficie(editor.proyecto, id, indice).map(|s| (s.borde, s.esquinas))
    else {
        return;
    };
    let (ancho_px, alto_px) = proporcion::medidas_de_superficie(&esquinas);
    let mut cambio = false;

    seccion_con_ayuda(
        ui,
        "Borde suave (edge blending)",
        "Apaga los lados de la superficie para que, al solaparse con otro proyector, la unión no se note: uno se apaga y el otro se enciende con la rampa espejada, y la luz que suman es constante. Si sabés cuántos píxeles se solapan, medilo en píxeles.",
    );
    {
        {
            // Unidad: % de la superficie o píxeles del proyector.
            let id_unidad = egui::Id::new("borde_en_pixeles");
            let mut en_pixeles = ui.data(|d| d.get_temp::<bool>(id_unidad)).unwrap_or(false);
            fila(ui, "Unidad", |ui| {
                ui.radio_value(&mut en_pixeles, false, "%");
                ui.radio_value(&mut en_pixeles, true, "píxeles");
            });
            ui.data_mut(|d| d.insert_temp(id_unidad, en_pixeles));
            // El ancho de la rampa es una fracción de la medida de la superficie **en la
            // dirección del degradé**: los bordes izquierdo y derecho se miden contra el
            // ANCHO, y los de arriba y abajo contra el ALTO.
            let lados = [
                ("Izquierda", &mut borde.izquierda, ancho_px),
                ("Derecha", &mut borde.derecha, ancho_px),
                ("Arriba", &mut borde.arriba, alto_px),
                ("Abajo", &mut borde.abajo, alto_px),
            ];
            for (nombre, valor, largo) in lados {
                fila(ui, nombre, |ui| {
                    if en_pixeles && largo > 0.0 {
                        let mut pixeles = *valor * largo;
                        if deslizador(ui, &mut pixeles, 0.0..=largo * 0.5, " px", 0, 0.0).changed()
                        {
                            *valor = pixeles / largo;
                            cambio = true;
                        }
                    } else {
                        let mut porcentaje = *valor * 100.0;
                        if deslizador(ui, &mut porcentaje, 0.0..=50.0, " %", 1, 0.0).changed() {
                            *valor = porcentaje / 100.0;
                            cambio = true;
                        }
                    }
                });
            }
            fila(ui, "Potencia", |ui| {
                cambio |= deslizador_con(
                    ui,
                    &mut borde.potencia,
                    vm_core::POTENCIA_MINIMA..=vm_core::POTENCIA_MAXIMA,
                    0.0,
                    |s| s.logarithmic(true).fixed_decimals(2),
                )
                .on_hover_text(
                    "Ajusta la curva: 1 es la neutra; más de 1 oscurece el borde, menos lo aclara",
                )
                .changed();
            });
            if boton(ui, "Sin borde").clicked() {
                borde = BordeSuave::default();
                cambio = true;
            }
        }
    }

    // Solo se escribe en el proyecto si algo cambió.
    if cambio && let Some(sup) = superficie_mut(editor.proyecto, id, indice) {
        borde.normalizar();
        sup.borde = borde;
    }
}

/// El patrón de prueba de una salida: se proyecta encima de todo para alinear y calibrar.
/// No se guarda en el proyecto (ver `vm_output::PatronesActivos`).
fn selector_de_patron(ui: &mut egui::Ui, editor: &mut ContextoEditor, id: &str) {
    let actual = editor.patrones.0.get(id).copied();
    let mut elegido = actual.map(|a| a.patron);
    let mut en_superficie = actual.is_some_and(|a| a.en_superficie);
    fila(ui, "Patrón", |ui| {
        desplegable(
            ui,
            "patron_de_prueba",
            elegido.map_or("Ninguno", |p| p.nombre()),
            |ui| {
                crate::inspector::opcion(ui, &mut elegido, None, "Ninguno");
                for patron in PatronPrueba::TODOS {
                    crate::inspector::opcion(ui, &mut elegido, Some(patron), patron.nombre());
                }
            },
        )
        .on_hover_text(
            "Se proyecta en esta salida para alinear y calibrar. No se guarda con el proyecto.",
        );
    });
    if elegido.is_some() {
        ui.horizontal(|ui| {
            ui.radio_value(&mut en_superficie, false, "En pantalla")
                .on_hover_text(
                    "Ocupa todo el proyector sin deformarse: sirve para ver hasta dónde llega la \
                     imagen, la resolución y los colores.",
                );
            ui.radio_value(&mut en_superficie, true, "En la superficie")
                .on_hover_text(
                    "Pasa por las superficies como si fuera la imagen: se deforma con las esquinas \
                     y la malla, y sirve para alinear con la pared (las líneas deben quedar rectas).",
                );
        });
        ui.label(icono("●", rojo()))
            .on_hover_text("Hay un patrón proyectándose");
    }
    let nuevo = elegido.map(|patron| PatronActivo {
        patron,
        en_superficie,
    });
    if nuevo != actual {
        match nuevo {
            Some(activo) => {
                editor.patrones.0.insert(id.to_string(), activo);
            }
            None => {
                editor.patrones.0.remove(id);
            }
        }
    }
}

/// La calibración de color de la salida (el proyector): ganancia por canal, brillo,
/// contraste y gamma. Vale para todas sus superficies y se ve enseguida, también en los
/// patrones de prueba.
fn controles_de_color(ui: &mut egui::Ui, editor: &mut ContextoEditor, id: &str) {
    let Some(mut color) = editor
        .proyecto
        .0
        .salidas
        .iter()
        .find(|s| s.monitor == id)
        .map(|s| s.color)
    else {
        return;
    };
    let mut cambio = false;

    seccion_con_ayuda(
        ui,
        "Calibración de color",
        "Para que dos proyectores muestren lo mismo. Proyectá un patrón de prueba (Blanco, Degradado de grises, Barras) y ajustá hasta que se vea igual que en el otro. Se aplica a todas las superficies de esta salida; con todo en su valor neutro la imagen no cambia.",
    );
    {
        {
            let deslizadores: [(&str, &mut f32, std::ops::RangeInclusive<f32>, bool, &str); 6] = [
                (
                    "Rojo",
                    &mut color.rojo,
                    GANANCIA_MINIMA..=GANANCIA_MAXIMA,
                    false,
                    "Cuánto rojo sale (1 = sin cambio)",
                ),
                (
                    "Verde",
                    &mut color.verde,
                    GANANCIA_MINIMA..=GANANCIA_MAXIMA,
                    false,
                    "Cuánto verde sale (1 = sin cambio)",
                ),
                (
                    "Azul",
                    &mut color.azul,
                    GANANCIA_MINIMA..=GANANCIA_MAXIMA,
                    false,
                    "Cuánto azul sale (1 = sin cambio)",
                ),
                (
                    "Brillo",
                    &mut color.brillo,
                    BRILLO_MINIMO..=BRILLO_MAXIMO,
                    false,
                    "Se suma a todo (0 = sin cambio): sube o baja también el negro",
                ),
                (
                    "Contraste",
                    &mut color.contraste,
                    CONTRASTE_MINIMO..=CONTRASTE_MAXIMO,
                    false,
                    "Separa o junta los tonos alrededor del gris medio (1 = sin cambio)",
                ),
                (
                    "Gamma",
                    &mut color.gamma,
                    GAMMA_MINIMO..=GAMMA_MAXIMO,
                    true,
                    "Curva de los tonos medios (1 = sin cambio; más de 1 los aclara)",
                ),
            ];
            for (nombre, valor, rango, logaritmico, ayuda) in deslizadores {
                fila(ui, nombre, |ui| {
                    cambio |= deslizador_con(ui, valor, rango, 0.0, |s| {
                        s.logarithmic(logaritmico).fixed_decimals(2)
                    })
                    .on_hover_text(ayuda)
                    .changed();
                });
            }
            if ui
                .add_enabled_ui(!color.es_neutra(), |ui| boton(ui, "Reiniciar color"))
                .inner
                .clicked()
            {
                color = CorreccionColor::default();
                cambio = true;
            }
        }
    }

    // Solo se escribe en el proyecto si algo cambió.
    if cambio
        && let Some(salida) = editor
            .proyecto
            .0
            .salidas
            .iter_mut()
            .find(|s| s.monitor == id)
    {
        color.normalizar();
        salida.color = color;
    }
}

/// Lo que el usuario pidió hacer con las superficies de una salida.
enum AccionSuperficie {
    Agregar,
    Duplicar,
    Borrar,
    /// Mover en la lista: +1 la dibuja más arriba (adelante), −1 más abajo (atrás).
    Mover(isize),
    /// Copiar la superficie elegida a otra salida (con ese id), reescalada a su resolución.
    CopiarA(String),
}

/// El árbol de la izquierda: cada salida activa con sus superficies (se elige una con un
/// clic; el ojo la oculta o la muestra) y, abajo, las acciones sobre la elegida: agregar,
/// duplicar, borrar y cambiar el orden.
///
/// Las superficies de una salida se dibujan en orden: **las últimas quedan encima**.
fn arbol_de_superficies(ui: &mut egui::Ui, editor: &mut ContextoEditor, salidas: &[&Salida]) {
    use egui_phosphor::regular as icono_de;

    // Se copia lo que hace falta del proyecto para no tenerlo prestado mientras se dibuja.
    struct FilaSalida {
        id: String,
        tamano: Vec2Texto,
        superficies: Vec<(String, bool)>,
    }
    struct Vec2Texto(String);
    let filas: Vec<FilaSalida> = salidas
        .iter()
        .filter_map(|s| {
            let config = editor
                .proyecto
                .0
                .salidas
                .iter()
                .find(|c| c.monitor == s.id)?;
            Some(FilaSalida {
                id: s.id.clone(),
                tamano: Vec2Texto(format!("{} x {}", s.tamano.x as u32, s.tamano.y as u32)),
                superficies: config
                    .superficies
                    .iter()
                    .enumerate()
                    .map(|(i, sup)| (sup.nombre_visible(i), sup.visible))
                    .collect(),
            })
        })
        .collect();

    let mut elegida: Option<(String, usize)> = None;
    let mut alternar_visible: Option<(String, usize)> = None;
    // Una superficie renombrada en el árbol: (salida, posición, texto nuevo).
    let mut renombrada: Option<(String, usize, String)> = None;
    let mut accion: Option<AccionSuperficie> = None;

    ui.label(texto_negrita("Salidas y superficies"));
    ui.add_space(2.0);
    // Las acciones valen para la superficie elegida.
    let cantidad = filas
        .iter()
        .find(|f| f.id == editor.seleccion.id)
        .map_or(0, |f| f.superficies.len());
    let actual = editor.seleccion.superficie;
    ui.horizontal(|ui| {
        if ui
            .button(icono(icono_de::PLUS, verde()))
            .on_hover_text("Agregar una superficie a esta salida")
            .clicked()
        {
            accion = Some(AccionSuperficie::Agregar);
        }
        if ui
            .button(icono(icono_de::COPY, azul()))
            .on_hover_text("Duplicar la superficie elegida")
            .clicked()
        {
            accion = Some(AccionSuperficie::Duplicar);
        }
        if filas.len() > 1 {
            ui.menu_button(icono(icono_de::EXPORT, naranja()), |ui| {
                ui.label("Copiar la superficie a:");
                for otra in filas.iter().filter(|f| f.id != editor.seleccion.id) {
                    if ui.button(&otra.id).clicked() {
                        accion = Some(AccionSuperficie::CopiarA(otra.id.clone()));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(
                "Copiar la superficie elegida a otra salida (se reescala a su resolución)",
            );
        }
        if ui
            .add_enabled(
                cantidad > 1,
                egui::Button::new(icono(icono_de::TRASH, rojo())),
            )
            .on_hover_text("Borrar la superficie elegida (siempre queda al menos una)")
            .clicked()
        {
            accion = Some(AccionSuperficie::Borrar);
        }
        if ui
            .add_enabled(
                actual > 0,
                egui::Button::new(icono(icono_de::ARROW_DOWN, naranja())),
            )
            .on_hover_text("Dibujarla debajo de la anterior")
            .clicked()
        {
            accion = Some(AccionSuperficie::Mover(-1));
        }
        if ui
            .add_enabled(
                actual + 1 < cantidad,
                egui::Button::new(icono(icono_de::ARROW_UP, naranja())),
            )
            .on_hover_text("Dibujarla encima de la siguiente")
            .clicked()
        {
            accion = Some(AccionSuperficie::Mover(1));
        }
    });

    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("arbol_de_superficies")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for fila in &filas {
                ui.horizontal(|ui| {
                    ui.label(icono(icono_de::MONITOR, azul()));
                    ui.label(texto_negrita(&fila.id));
                    ui.weak(&fila.tamano.0);
                });
                // La que se dibuja ENCIMA de todas va primero en la lista (como las capas): así el botón
                // «subir» la sube en la lista y «bajar» la baja.
                for (i, (nombre, visible)) in fila.superficies.iter().enumerate().rev() {
                    let es_la_elegida =
                        editor.seleccion.id == fila.id && editor.seleccion.superficie == i;
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let ojo = if *visible {
                            icono(icono_de::EYE, verde())
                        } else {
                            icono(icono_de::EYE_SLASH, texto_suave())
                        };
                        if ui
                            .add(egui::Button::new(ojo).frame(false))
                            .on_hover_text(if *visible { "Ocultar" } else { "Mostrar" })
                            .clicked()
                        {
                            alternar_visible = Some((fila.id.clone(), i));
                        }
                        // Doble clic: se renombra en el lugar (Enter confirma, Escape cancela).
                        let id_ren = egui::Id::new(("renombrar_superficie", &fila.id, i));
                        let renombrando = ui.data(|d| d.get_temp::<bool>(id_ren)).unwrap_or(false);
                        if renombrando {
                            let pedir_foco = ui
                                .data_mut(|d| d.remove_temp::<bool>(id_ren.with("foco")))
                                .unwrap_or(false);
                            match campo_de_nombre(
                                ui,
                                id_ren.with("texto"),
                                nombre,
                                140.0,
                                pedir_foco,
                            ) {
                                EdicionDeNombre::Sigue => {}
                                EdicionDeNombre::Confirmado(texto) => {
                                    ui.data_mut(|d| d.insert_temp(id_ren, false));
                                    renombrada = Some((fila.id.clone(), i, texto));
                                }
                                EdicionDeNombre::Cancelado => {
                                    ui.data_mut(|d| d.insert_temp(id_ren, false));
                                }
                            }
                        } else {
                            let texto = format!("{} {}", icono_de::POLYGON, nombre);
                            let respuesta = crate::inspector::elegible(ui, es_la_elegida, texto)
                                .on_hover_text("Doble clic para renombrar");
                            if respuesta.clicked() {
                                elegida = Some((fila.id.clone(), i));
                            }
                            if respuesta.double_clicked() {
                                elegida = Some((fila.id.clone(), i));
                                ui.data_mut(|d| {
                                    d.insert_temp(id_ren, true);
                                    d.insert_temp(id_ren.with("foco"), true);
                                });
                            }
                        }
                    });
                }
                ui.add_space(6.0);
            }
        });

    // --- Se aplica lo pedido ---
    if let Some((id, i, texto)) = renombrada
        && !texto.trim().is_empty()
        && let Some(sup) = superficie_mut(editor.proyecto, &id, i)
        && sup.nombre != texto.trim()
    {
        sup.nombre = texto.trim().to_string();
    }
    if let Some((id, i)) = elegida {
        editor.seleccion.id = id;
        editor.seleccion.superficie = i;
    }
    if let Some((id, i)) = alternar_visible
        && let Some(sup) = superficie_mut(editor.proyecto, &id, i)
    {
        sup.visible = !sup.visible;
    }
    if let Some(AccionSuperficie::CopiarA(destino)) = &accion
        && let Some(origen) = salidas.iter().find(|s| s.id == editor.seleccion.id)
        && let Some(destino) = salidas.iter().find(|s| s.id == *destino)
    {
        // Las posiciones se reescalan a la resolución del otro proyector.
        let escala = [
            destino.tamano.x / origen.tamano.x,
            destino.tamano.y / origen.tamano.y,
        ];
        if let Some(nueva) =
            editor
                .proyecto
                .0
                .copiar_superficie(&origen.id, actual, &destino.id, escala)
        {
            editor.seleccion.id = destino.id.clone();
            editor.seleccion.superficie = nueva;
        }
        accion = None;
    }
    if let Some(accion) = accion
        && let Some(salida) = salidas.iter().find(|s| s.id == editor.seleccion.id)
        && let Some(config) = editor
            .proyecto
            .0
            .salidas
            .iter_mut()
            .find(|s| s.monitor == salida.id)
    {
        let nueva = match accion {
            AccionSuperficie::Agregar => {
                Some(config.agregar_superficie(salida.tamano.x, salida.tamano.y))
            }
            AccionSuperficie::Duplicar => config.duplicar_superficie(actual),
            AccionSuperficie::Borrar => config
                .borrar_superficie(actual)
                .then(|| actual.min(config.superficies.len() - 1)),
            AccionSuperficie::Mover(delta) => config.mover_superficie(actual, delta),
            AccionSuperficie::CopiarA(_) => None,
        };
        if let Some(indice) = nueva {
            editor.seleccion.superficie = indice;
        }
    }
}

/// Cómo terminó una edición de nombre.
enum EdicionDeNombre {
    /// Sigue escribiéndose (o ni empezó).
    Sigue,
    /// Se confirmó (Enter o clic afuera) con este texto.
    Confirmado(String),
    /// Se canceló (Escape).
    Cancelado,
}

/// Un campo de texto para renombrar que **no escribe en el proyecto mientras se tipea**: el
/// texto vive aparte hasta confirmar. Así se puede borrar todo y volver a escribir sin que el
/// nombre «salte» a otro valor, y un nombre vacío nunca llega al proyecto.
///
/// `id` identifica este campo (con él se guarda el texto en la memoria de egui);
/// `pedir_foco` lo enfoca en el primer frame (para el renombrado en el lugar).
fn campo_de_nombre(
    ui: &mut egui::Ui,
    id: egui::Id,
    actual: &str,
    ancho: f32,
    pedir_foco: bool,
) -> EdicionDeNombre {
    let mut texto: String = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| actual.to_string());
    let respuesta = ui.add(egui::TextEdit::singleline(&mut texto).desired_width(ancho));
    if pedir_foco {
        respuesta.request_focus();
    }
    if respuesta.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, texto.clone()));
        return EdicionDeNombre::Sigue;
    }
    if respuesta.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(id));
        let cancelado = ui.input(|i| i.key_pressed(egui::Key::Escape));
        return if cancelado {
            EdicionDeNombre::Cancelado
        } else {
            EdicionDeNombre::Confirmado(texto)
        };
    }
    // Sin foco y sin haberlo tenido (por ejemplo, se pidió el foco y no llegó): se da por
    // cancelado, para no quedar en modo edición para siempre.
    if !pedir_foco && ui.data(|d| d.get_temp::<String>(id)).is_some() {
        ui.data_mut(|d| d.remove_temp::<String>(id));
        return EdicionDeNombre::Cancelado;
    }
    EdicionDeNombre::Sigue
}

/// Nombre, visibilidad y origen de la superficie elegida (arriba del panel de propiedades).
fn datos_de_superficie(ui: &mut egui::Ui, editor: &mut ContextoEditor, salida: &Salida) {
    let actual = editor.seleccion.superficie;
    let Some((nombre, mut visible, mut origen)) = superficie(editor.proyecto, &salida.id, actual)
        .map(|s| (s.nombre_visible(actual), s.visible, s.origen))
    else {
        return;
    };
    // Las opciones de origen: la composición y cada capa del proyecto.
    let comp = &editor.proyecto.0.composicion;
    let mut opciones: Vec<(OrigenSuperficie, String)> =
        vec![(OrigenSuperficie::Composicion, "Composición".to_string())];
    opciones.extend((0..comp.capas.len()).map(|i| {
        let o = OrigenSuperficie::Capa(i);
        (o, o.nombre(comp))
    }));
    // Si el origen actual no está entre las opciones (apunta a una capa que ya no existe),
    // igual se muestra, para que se vea el problema.
    let nombre_origen = origen.nombre(comp);
    let mut nombre_nuevo: Option<String> = None;
    let (mut cambio_visible, mut cambio_origen) = (false, false);

    fila(ui, "Nombre", |ui| {
        let id = egui::Id::new(("nombre_de_superficie", &salida.id, actual));
        if let EdicionDeNombre::Confirmado(texto) =
            campo_de_nombre(ui, id, &nombre, ui.available_width(), false)
        {
            nombre_nuevo = Some(texto);
        }
    });
    fila(ui, "Visible", |ui| {
        cambio_visible = ui.checkbox(&mut visible, "").changed();
    });
    fila(ui, "Origen", |ui| {
        desplegable(ui, "origen_de_superficie", &nombre_origen, |ui| {
            for (valor, texto) in &opciones {
                if crate::inspector::opcion(ui, &mut origen, *valor, texto).changed() {
                    cambio_origen = true;
                }
            }
        })
        .on_hover_text("De dónde toma la imagen: la composición mezclada, o una capa sola");
    });

    // Solo se escribe en el proyecto si hubo un cambio.
    let nombre_nuevo = nombre_nuevo
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && *n != nombre);
    if (nombre_nuevo.is_some() || cambio_visible || cambio_origen)
        && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, actual)
    {
        if cambio_origen {
            sup.origen = origen;
        }
        if let Some(n) = nombre_nuevo {
            sup.nombre = n;
        }
        if cambio_visible {
            sup.visible = visible;
        }
    }
}

/// Editor de la entrada: la composición con un rectángulo que se arrastra y se redimensiona.
fn editor_entrada(ui: &mut egui::Ui, editor: &mut ContextoEditor, id: &str) {
    let Some((mut entrada, origen)) =
        superficie(editor.proyecto, id, editor.seleccion.superficie).map(|s| (s.entrada, s.origen))
    else {
        return;
    };
    ui.label(texto_negrita(format!(
        "Entrada ({})",
        origen.nombre(&editor.proyecto.0.composicion)
    )));
    ui.horizontal(|ui| {
        ui.set_min_height(ALTO_CABECERA);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Arrastrá el rectángulo o sus esquinas.")
                    .color(ui.visuals().weak_text_color()),
            )
            .truncate(),
        )
        .on_hover_text(
            "Arrastrá el rectángulo o sus esquinas. Con un marco dibujado sobre el espacio \
             vacío elegís varias esquinas (Shift suma).",
        );
    });
    let Some(composicion) = editor.composicion else {
        ui.label("(la composición todavía no existe)");
        return;
    };
    // La imagen que se muestra es la del origen elegido: la composición o una capa.
    let fuente = textura_de_origen(origen, composicion, editor.capas);

    // La imagen ocupa el espacio disponible manteniendo su proporción.
    let relacion = fuente.ancho as f32 / fuente.alto as f32;
    let id_textura = editor
        .texturas
        .add_image(EguiTextureHandle::Weak(fuente.imagen.id()));
    let (respuesta, rect_imagen) = lienzo(ui, id_textura, relacion);
    let rect_min = [rect_imagen.min.x, rect_imagen.min.y];
    let rect_tam = [rect_imagen.width(), rect_imagen.height()];

    // Pantalla <-> coordenadas de la composición (0..1, con la `y` hacia abajo).
    let a_uv = |p: egui::Pos2| {
        [
            (p.x - rect_min[0]) / rect_tam[0],
            (p.y - rect_min[1]) / rect_tam[1],
        ]
    };
    let a_pantalla = |uv: [f32; 2]| {
        egui::pos2(
            rect_min[0] + uv[0] * rect_tam[0],
            rect_min[1] + uv[1] * rect_tam[1],
        )
    };

    // --- Interacción con el mouse ---
    let mut modificado = false;
    let sumar = ui.input(|i| i.modifiers.shift);
    // La selección es de una superficie: al cambiar de superficie se limpia.
    let de_esta = Some((id.to_string(), editor.seleccion.superficie));
    if editor.arrastre_entrada.elegidas_de != de_esta {
        editor.arrastre_entrada.elegidas.clear();
        editor.arrastre_entrada.elegidas_de = de_esta;
    }
    let esquina_bajo = |origen: egui::Pos2, entrada: Entrada| {
        entrada::esquinas(entrada)
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let p = a_pantalla(*e);
                (i, (p.x - origen.x).hypot(p.y - origen.y))
            })
            .filter(|(_, d)| *d < RADIO_AGARRE)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };

    // Al empezar a arrastrar: sobre una esquina se mueve ella (o todo el grupo elegido);
    // dentro del rectángulo se mueve entero; sobre espacio vacío se dibuja un marco.
    if respuesta.drag_started()
        && let Some(origen) = ui.input(|i| i.pointer.press_origin())
    {
        let uv = a_uv(origen);
        editor.arrastre_entrada.modo = match esquina_bajo(origen, entrada) {
            Some(i) => {
                let actuales: Vec<Asa> = editor
                    .arrastre_entrada
                    .elegidas
                    .iter()
                    .map(|e| Asa::Esquina(*e))
                    .collect();
                editor.arrastre_entrada.elegidas =
                    seleccion::al_agarrar(&actuales, Asa::Esquina(i), sumar)
                        .iter()
                        .filter_map(|a| {
                            if let Asa::Esquina(e) = a {
                                Some(*e)
                            } else {
                                None
                            }
                        })
                        .collect();
                Some(ModoEntrada::Grupo { ultimo: uv })
            }
            None if entrada::contiene(entrada, uv) => Some(ModoEntrada::Mover { ultimo: uv }),
            None => Some(ModoEntrada::Marco {
                inicio: uv,
                actual: uv,
            }),
        };
    }
    if respuesta.dragged()
        && let Some(modo) = editor.arrastre_entrada.modo
        && let Some(pos) = respuesta.interact_pointer_pos()
    {
        let uv = a_uv(pos);
        match modo {
            ModoEntrada::Grupo { ultimo } => {
                let elegidas = editor.arrastre_entrada.elegidas.clone();
                if let [una] = elegidas[..] {
                    // Una sola esquina: sigue al puntero, con la opuesta fija.
                    entrada = entrada::mover_esquina(entrada, una, uv);
                } else {
                    let delta = [uv[0] - ultimo[0], uv[1] - ultimo[1]];
                    entrada = entrada::mover_esquinas_elegidas(entrada, &elegidas, delta);
                }
                editor.arrastre_entrada.modo = Some(ModoEntrada::Grupo { ultimo: uv });
                modificado = true;
            }
            ModoEntrada::Mover { ultimo } => {
                let delta = [uv[0] - ultimo[0], uv[1] - ultimo[1]];
                entrada = entrada::mover_rectangulo(entrada, delta);
                editor.arrastre_entrada.modo = Some(ModoEntrada::Mover { ultimo: uv });
                modificado = true;
            }
            ModoEntrada::Marco { inicio, .. } => {
                editor.arrastre_entrada.modo = Some(ModoEntrada::Marco { inicio, actual: uv });
            }
        }
    }
    if respuesta.drag_stopped() {
        // Al soltar un marco, quedan elegidas las esquinas que cayeron adentro.
        if let Some(ModoEntrada::Marco { inicio, actual }) = editor.arrastre_entrada.modo {
            let marco = seleccion::rectangulo(inicio, actual);
            let adentro: Vec<Asa> = entrada::esquinas(entrada)
                .iter()
                .enumerate()
                .filter(|(_, e)| seleccion::dentro(marco, **e))
                .map(|(i, _)| Asa::Esquina(i))
                .collect();
            let actuales: Vec<Asa> = editor
                .arrastre_entrada
                .elegidas
                .iter()
                .map(|e| Asa::Esquina(*e))
                .collect();
            editor.arrastre_entrada.elegidas =
                seleccion::al_soltar_marco(&actuales, &adentro, sumar)
                    .iter()
                    .filter_map(|a| {
                        if let Asa::Esquina(e) = a {
                            Some(*e)
                        } else {
                            None
                        }
                    })
                    .collect();
        }
        editor.arrastre_entrada.modo = None;
    }
    // Un clic sobre una esquina la elige sola (con Shift, la suma o la quita); en vacío limpia.
    if respuesta.clicked()
        && let Some(pos) = respuesta.interact_pointer_pos()
    {
        let actuales: Vec<Asa> = editor
            .arrastre_entrada
            .elegidas
            .iter()
            .map(|e| Asa::Esquina(*e))
            .collect();
        let sobre = esquina_bajo(pos, entrada).map(Asa::Esquina);
        editor.arrastre_entrada.elegidas = seleccion::al_hacer_clic(&actuales, sobre, sumar)
            .iter()
            .filter_map(|a| {
                if let Asa::Esquina(e) = a {
                    Some(*e)
                } else {
                    None
                }
            })
            .collect();
    }

    // --- Dibujo encima de la composición ---
    let pintor = ui.painter_at(respuesta.rect);
    let puntos: Vec<egui::Pos2> = entrada::esquinas(entrada)
        .iter()
        .map(|e| a_pantalla(*e))
        .collect();
    pintor.add(egui::Shape::convex_polygon(
        puntos.clone(),
        naranja().gamma_multiply(0.16),
        egui::Stroke::new(2.0, naranja()),
    ));
    for (i, punto) in puntos.iter().enumerate() {
        pintor.circle_filled(*punto, 6.0, naranja());
        pintor.circle_stroke(*punto, 7.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
        if editor.arrastre_entrada.elegidas.contains(&i) {
            pintor.circle_stroke(*punto, 11.0, egui::Stroke::new(2.5, amarillo()));
        }
    }
    if let Some(ModoEntrada::Marco { inicio, actual }) = editor.arrastre_entrada.modo {
        dibujar_marco(&pintor, a_pantalla(inicio), a_pantalla(actual));
    }

    // Solo se escribe en el proyecto si algo cambió: así no se marca como modificado en cada frame.
    if modificado
        && let Some(sup) = superficie_mut(editor.proyecto, id, editor.seleccion.superficie)
    {
        sup.entrada = entrada;
    }
}

/// Distancia en píxeles de pantalla a la que un doble clic "agarra" un lado de la forma
/// para agregarle un punto.
const DISTANCIA_LADO: f32 = 30.0;

/// Mueve las cuatro esquinas la misma distancia. Una traslación conserva la convexidad.
fn trasladar_esquinas(esquinas: [[f32; 2]; 4], delta: [f32; 2]) -> [[f32; 2]; 4] {
    let t = trasladar(&esquinas, delta);
    [t[0], t[1], t[2], t[3]]
}

/// El punto de la forma más cercano al puntero, si alguno está a menos de `RADIO_AGARRE`.
/// Los puntos de la forma están en coordenadas de la superficie (`u`, `v`); se pasan por la
/// homografía para saber dónde caen en pantalla.
fn punto_forma_cercano(
    forma: &[Punto],
    h: &Homografia,
    malla: Option<&Malla>,
    puntero: egui::Pos2,
    rect_min: [f32; 2],
    rect_tam: [f32; 2],
    salida_tam: [f32; 2],
) -> Option<usize> {
    forma
        .iter()
        .enumerate()
        .map(|(i, uv)| {
            let en_salida = deformacion::proyectar(h, malla, uv[0], uv[1]);
            let p = salida_a_pantalla(en_salida, rect_min, rect_tam, salida_tam);
            (i, (p[0] - puntero.x).hypot(p[1] - puntero.y))
        })
        .filter(|(_, distancia)| *distancia < RADIO_AGARRE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Los puntos que se pueden elegir en el modo actual, con su posición en la salida: las
/// esquinas, los puntos de la forma o los de la malla. Las esquinas de la malla son las de
/// la superficie (el corner pin), así que en el modo Malla también se pueden arrastrar: se
/// ofrecen como `Asa::Esquina`.
fn asas_en_salida(
    modo: ModoEdicion,
    esquinas: &[[f32; 2]; 4],
    forma: Option<&[Punto]>,
    malla: Option<&Malla>,
    h: &Homografia,
) -> Vec<(Asa, [f32; 2])> {
    match modo {
        ModoEdicion::Esquinas => esquinas
            .iter()
            .enumerate()
            .map(|(i, e)| (Asa::Esquina(i), *e))
            .collect(),
        ModoEdicion::Forma => forma
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(i, uv)| {
                (
                    Asa::Forma(i),
                    deformacion::proyectar(h, malla, uv[0], uv[1]),
                )
            })
            .collect(),
        ModoEdicion::Malla => {
            let Some(m) = malla else { return Vec::new() };
            let mut asas: Vec<(Asa, [f32; 2])> = esquinas
                .iter()
                .enumerate()
                .map(|(i, e)| (Asa::Esquina(i), *e))
                .collect();
            for fila in 0..m.filas {
                for columna in 0..m.columnas {
                    if !m.es_esquina(columna, fila) {
                        let [u, v] = m.uv_de_nodo(columna, fila);
                        asas.push((
                            Asa::Nodo(columna, fila),
                            deformacion::proyectar(h, Some(m), u, v),
                        ));
                    }
                }
            }
            asas
        }
        // Las máscaras y los huecos se editan como una forma (ver `editor_salida`): no tienen
        // puntos propios.
        ModoEdicion::Mascara | ModoEdicion::Hueco => Vec::new(),
    }
}

/// El punto más cercano al puntero (a menos de `RADIO_AGARRE`) entre los que se ofrecen.
fn asa_mas_cercana(
    asas: &[(Asa, [f32; 2])],
    puntero: egui::Pos2,
    rect_min: [f32; 2],
    rect_tam: [f32; 2],
    salida_tam: [f32; 2],
) -> Option<Asa> {
    asas.iter()
        .map(|(asa, p)| {
            let q = salida_a_pantalla(*p, rect_min, rect_tam, salida_tam);
            (*asa, (q[0] - puntero.x).hypot(q[1] - puntero.y))
        })
        .filter(|(_, d)| *d < RADIO_AGARRE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(asa, _)| asa)
}

/// Cuántos puntos se usan para dibujar cada curva de la malla o del contorno.
const MUESTRAS_DE_CURVA: usize = 24;

/// El contorno de la superficie en la salida, siguiendo la malla si la tiene (los lados se
/// curvan). Es un polígono cerrado de `4 × MUESTRAS_DE_CURVA` puntos.
fn contorno_de_superficie(h: &Homografia, malla: Option<&Malla>) -> Vec<[f32; 2]> {
    let n = MUESTRAS_DE_CURVA;
    let mut puntos = Vec::with_capacity(4 * n);
    for i in 0..n {
        puntos.push(deformacion::proyectar(h, malla, i as f32 / n as f32, 0.0)); // arriba, de izquierda a derecha
    }
    for i in 0..n {
        puntos.push(deformacion::proyectar(h, malla, 1.0, i as f32 / n as f32)); // derecha, hacia abajo
    }
    for i in 0..n {
        puntos.push(deformacion::proyectar(
            h,
            malla,
            1.0 - i as f32 / n as f32,
            1.0,
        )); // abajo, de derecha a izquierda
    }
    for i in 0..n {
        puntos.push(deformacion::proyectar(
            h,
            malla,
            0.0,
            1.0 - i as f32 / n as f32,
        )); // izquierda, hacia arriba
    }
    puntos
}

/// Las herramientas de la superficie elegida, en el panel de abajo: tamaño, lo que depende
/// del modo de edición (esquinas, forma o malla) y el recorte de la entrada. Edita el
/// proyecto directamente; las vistas de Entrada y Salida solo se ocupan del mouse.
/// La barra de superficie es una TABLA con dos bloques, cada uno con sus etiquetas en una columna
/// de ancho fijo y los controles en RANURAS del mismo ancho, alineadas de una fila a la otra:
///
/// ```text
/// Nombre   [______________]        Centro               [ x ] [ y ] [Centrar]
/// Visible  [x]                     Escala               [100 %] [−10 %] [+10 %] [Original]
/// Origen   [Composición   v]       Igualar a la entrada [ Proporción ] [ Tamaño real ]
/// ```
///
/// Con ancho de sobra van uno al lado del otro; si no, el segundo baja debajo del primero.
const RANURA_MAXIMA: f32 = 80.0;
/// La ranura más angosta a la que se achica cuando falta lugar.
const RANURA_MINIMA: f32 = 56.0;
/// Alto de cada bloque: tres filas con su separación.
const FILAS_POR_BLOQUE: f32 = 3.0;

/// Cuánto miden las etiquetas de cada bloque (la de más ancho, con un poco de aire).
fn etiquetas_de_la_barra(ui: &egui::Ui) -> (f32, f32) {
    let t = |texto: &str| ancho_de_texto(ui, texto);
    let datos = t("Nombre").max(t("Visible")).max(t("Origen")) + 6.0;
    let tamano = t("Centro").max(t("Escala")).max(t("Igualar a la entrada")) + 6.0;
    (datos, tamano)
}

/// El ancho de cada ranura para el ancho `disponible`: la máxima si los dos bloques entran uno al
/// lado del otro; si no, la que hace falta para que el bloque de tamaño entre solo (con un mínimo).
fn ranura_de_la_barra(ui: &egui::Ui, disponible: f32) -> f32 {
    let e = ui.spacing().item_spacing.x;
    let (etiqueta_datos, etiqueta_tamano) = etiquetas_de_la_barra(ui);
    let lado_a_lado = |r: f32| {
        (etiqueta_datos + e + 2.0 * r + e) + e + (etiqueta_tamano + e + 4.0 * r + 3.0 * e) + 8.0
    };
    if disponible >= lado_a_lado(RANURA_MAXIMA) {
        return RANURA_MAXIMA;
    }
    ((disponible - etiqueta_tamano - 4.0 * e - 4.0) / 4.0).clamp(RANURA_MINIMA, RANURA_MAXIMA)
}

/// El ancho de una ranura doble (dos ranuras y el espacio que hay entre ellas).
fn ranura_doble(ui: &egui::Ui, ranura: f32) -> f32 {
    2.0 * ranura + ui.spacing().item_spacing.x
}

/// Cuánto mide cada bloque de la barra: 0 datos (nombre, visible, origen), 1 tamaño y posición
/// (centro, escala, igualar). Se calcula con las mismas medidas con que se dibujan.
fn anchos_de_la_barra(ui: &egui::Ui, ranura: f32) -> [f32; 2] {
    let e = ui.spacing().item_spacing.x;
    let (etiqueta_datos, etiqueta_tamano) = etiquetas_de_la_barra(ui);
    let datos = etiqueta_datos + e + ranura_doble(ui, ranura);
    let tamano = etiqueta_tamano + e + 4.0 * ranura + 3.0 * e;
    [datos + 4.0, tamano + 4.0]
}

/// El alto de un bloque de la barra.
fn alto_del_bloque(ui: &egui::Ui) -> f32 {
    FILAS_POR_BLOQUE * ALTO_FILA + (FILAS_POR_BLOQUE - 1.0) * ui.spacing().item_spacing.y
}

/// Qué parte de las herramientas de la superficie se dibuja y dónde.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parte {
    /// Posición, escala e «igualar a la entrada»: la barra que va arriba de la vista de salida.
    Barra,
    /// Lo que depende del modo de edición (esquinas, recorte, malla, máscaras, huecos): el
    /// inspector de la derecha.
    Modo,
    /// El recorte de la composición que toma la superficie (pantalla de Entrada).
    Entrada,
}

fn herramientas_de_superficie(
    ui: &mut egui::Ui,
    editor: &mut ContextoEditor,
    salida: &Salida,
    parte: Parte,
    flujo: Option<&Flujo>,
) {
    let indice_de_superficie = editor.seleccion.superficie;
    let Some((mut esquinas, mut forma, mut malla, mut entrada)) =
        superficie(editor.proyecto, &salida.id, indice_de_superficie)
            .map(|s| (s.esquinas, s.forma.clone(), s.malla.clone(), s.entrada))
    else {
        ui.label("Esta salida no tiene superficies.");
        return;
    };
    let modo = editor.edicion.modo;
    let (ancho_comp, alto_comp) = (
        editor.proyecto.0.composicion.ancho as f32,
        editor.proyecto.0.composicion.alto as f32,
    );
    let (mut modificado, mut forma_cambio, mut malla_cambio, mut entrada_cambio) =
        (false, false, false, false);

    ui.vertical(|ui| {
        if parte == Parte::Barra {
            // Los controles de la barra tienen tamaños FIJOS (no se estiran ni se parten): si la
            // ventana es angosta, la barra pasa a dos líneas, nunca se solapa.
            let escala_vigente = editor.edicion.escala.as_ref().is_some_and(|b| {
                b.vale_para(&salida.id, indice_de_superficie, &esquinas, malla.as_ref())
            });
            // La escala cambia el tamaño de la superficie (y de su forma) desde su centro. El
            // porcentaje es relativo a un tamaño base (ver `escala.rs`). Si la base ya no vale
            // (se cambió de superficie o se movió una esquina), se renueva y el campo vuelve a 100.
            if !escala_vigente {
                editor.edicion.escala = Some(EscalaBase::nueva(
                    salida.id.clone(),
                    indice_de_superficie,
                    esquinas,
                    malla.as_ref(),
                ));
                editor.edicion.texto_escala = formatear_porcentaje(100.0);
            }
            let porcentaje_actual = editor
                .edicion
                .escala
                .as_ref()
                .map_or(100.0, |b| b.porcentaje);
            let mut nuevo_porcentaje: Option<f32> = None;
            let proporcion = proporcion_de_entrada(entrada, ancho_comp, alto_comp);
            let centro = proporcion::centro_de_superficie(&esquinas);
            let mut nuevo = centro;

            // El segundo bloque de la tabla (grupo 1 del `Flujo`): centro, escala e igualar, con las
            // etiquetas en una columna y los controles en ranuras alineadas.
            let flujo = flujo.expect("la barra necesita su flujo");
            let (_, etiqueta) = etiquetas_de_la_barra(ui);
            let ranura = ranura_de_la_barra(ui, flujo.ancho());
            let doble = ranura_doble(ui, ranura);
            flujo.grupo(ui, 1, |ui| {
                ui.vertical(|ui| {
                    // --- Posición ---
                    fila_con_etiqueta(ui, etiqueta, "Centro", |ui| {
                        ui.add_sized(
                            [ranura, ALTO_FILA],
                            egui::DragValue::new(&mut nuevo[0]).speed(1.0).prefix("x: "),
                        );
                        ui.add_sized(
                            [ranura, ALTO_FILA],
                            egui::DragValue::new(&mut nuevo[1]).speed(1.0).prefix("y: "),
                        );
                        if ui
                            .add_sized([ranura, ALTO_FILA], egui::Button::new("Centrar"))
                            .on_hover_text("Lleva el centro de la superficie al centro del proyector")
                            .clicked()
                        {
                            nuevo = [0.0, 0.0];
                        }
                    });

                    // --- Tamaño ---
                    fila_con_etiqueta(ui, etiqueta, "Escala", |ui| {
                        // La primera ranura: el campo y su «%».
                        let mut campo = None;
                        ui.allocate_ui_with_layout(
                            egui::vec2(ranura, ALTO_FILA),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_size(egui::vec2(ranura, ALTO_FILA));
                                let ancho_signo = ancho_de_texto(ui, "%");
                                let ancho_campo = ranura - ui.spacing().item_spacing.x - ancho_signo;
                                campo = Some(
                                    ui.add_sized(
                                        [ancho_campo, ALTO_FILA],
                                        egui::TextEdit::singleline(&mut editor.edicion.texto_escala),
                                    )
                                    .on_hover_text(
                                        "Tamaño de la superficie, desde su centro y conservando su forma. Escribí un \
                                         porcentaje y apretá Enter (50 la achica a la mitad; Esc cancela).",
                                    ),
                                );
                                ui.label("%");
                            },
                        );
                        if let Some(campo) = campo {
                            if campo.lost_focus() {
                                // Se aplica al confirmar (Enter, o salir del campo), no mientras se
                                // escribe: si no, escribir "90" aplicaría "9" al tipear el primer dígito.
                                let cancelar = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                match interpretar_porcentaje(&editor.edicion.texto_escala) {
                                    Some(p) if !cancelar => nuevo_porcentaje = Some(p),
                                    // Texto inválido, o Esc: se vuelve al valor que hay.
                                    _ => {
                                        editor.edicion.texto_escala =
                                            formatear_porcentaje(porcentaje_actual)
                                    }
                                }
                            } else if !campo.has_focus() {
                                // Sin foco, el campo muestra siempre el valor real.
                                let texto = formatear_porcentaje(porcentaje_actual);
                                if editor.edicion.texto_escala != texto {
                                    editor.edicion.texto_escala = texto;
                                }
                            }
                        }
                        if ui
                            .add_sized([ranura, ALTO_FILA], egui::Button::new("−10 %"))
                            .clicked()
                        {
                            nuevo_porcentaje = Some(porcentaje_actual * 0.9);
                        }
                        if ui
                            .add_sized([ranura, ALTO_FILA], egui::Button::new("+10 %"))
                            .clicked()
                        {
                            nuevo_porcentaje = Some(porcentaje_actual * 1.1);
                        }
                        if ui
                            .add_sized([ranura, ALTO_FILA], egui::Button::new("Original"))
                            .on_hover_text("Vuelve al tamaño que tenía al empezar a escalar")
                            .clicked()
                        {
                            nuevo_porcentaje = Some(100.0);
                        }
                    });

                    // --- Igualar a la entrada ---
                    fila_con_etiqueta(ui, etiqueta, "Igualar a la entrada", |ui| {
                        if ui
                            .add_enabled_ui(proporcion.is_some(), |ui| {
                                ui.add_sized([doble, ALTO_FILA], egui::Button::new("Proporción"))
                            })
                            .inner
                            .on_hover_text(
                                "La superficie pasa a ser un rectángulo con la misma proporción \
                                 (ancho/alto) que el recorte de la entrada, centrado y lo más grande \
                                 posible dentro de donde estaba",
                            )
                            .clicked()
                            && let Some(p) = proporcion
                        {
                            esquinas = proporcion::con_proporcion(&esquinas, p);
                            modificado = true;
                        }
                        if ui
                            .add_sized([doble, ALTO_FILA], egui::Button::new("Tamaño real"))
                            .on_hover_text(
                                "La superficie pasa a medir en píxeles del proyector lo mismo que el \
                                 recorte en píxeles de la composición (1 a 1), centrada donde estaba",
                            )
                            .clicked()
                        {
                            let tamano = tamano_de_entrada(entrada, ancho_comp, alto_comp);
                            esquinas = proporcion::con_tamano(&esquinas, tamano);
                            modificado = true;
                        }
                    });
                });
            });

            // Se mueve la superficie entera: las distancias del centro a cada esquina no cambian.
            if nuevo != centro {
                esquinas = proporcion::con_centro(&esquinas, nuevo);
                modificado = true;
            }
            if let Some(p) = nuevo_porcentaje
                && let Some(base) = editor.edicion.escala.as_mut()
            {
                base.porcentaje = p.clamp(
                    crate::escala::PORCENTAJE_MINIMO,
                    crate::escala::PORCENTAJE_MAXIMO,
                );
                esquinas = base.esquinas();
                modificado = true;
                // La deformación de la malla crece o se achica junto con la superficie.
                if malla.is_some() {
                    malla = base.malla_escalada();
                    malla_cambio = true;
                }
                editor.edicion.texto_escala = formatear_porcentaje(base.porcentaje);
            }
        }

        if parte == Parte::Modo {
            // --- Lo que depende del modo de edición ---
            seccion(
                ui,
                match modo {
                    ModoEdicion::Esquinas => "Esquinas",
                    ModoEdicion::Forma => "Recorte (se deforma con la superficie)",
                    ModoEdicion::Malla => "Malla (mesh warp)",
                    ModoEdicion::Mascara => "Máscaras (fijas sobre el proyector)",
                    ModoEdicion::Hueco => "Huecos (se deforman con la superficie)",
                },
            );
            // Si se quitaron puntos, el elegido puede haber quedado fuera de rango.
            if let Some(puntos) = &forma {
                editor.edicion.punto_forma = editor.edicion.punto_forma.min(puntos.len() - 1);
            }
            // Lo de crear o quitar va primero y a todo el ancho; abajo, lo que se acumula.
            acciones_del_modo(ui, editor, salida);
            match modo {
                ModoEdicion::Esquinas => {
                    for i in 0..4 {
                        let mut nuevo = esquinas[i];
                        let (clic, (cambio_x, cambio_y)) = fila_elegible(
                            ui,
                            &format!("Esquina {}", i + 1),
                            editor.seleccion.esquina == i,
                            |ui| {
                                let (mut x, mut y) = (false, false);
                                ui.columns(2, |c| {
                                    x = campo_numerico(&mut c[0], &mut nuevo[0], "x: ", 1.0)
                                        .changed();
                                    y = campo_numerico(&mut c[1], &mut nuevo[1], "y: ", 1.0)
                                        .changed();
                                });
                                (x, y)
                            },
                        );
                        if clic {
                            editor.seleccion.esquina = i;
                        }
                        if cambio_x || cambio_y {
                            let mut nuevas = esquinas;
                            nuevas[i] = nuevo;
                            if es_convexo(nuevas) {
                                esquinas = nuevas;
                                modificado = true;
                            }
                        }
                    }
                }
                ModoEdicion::Mascara => {
                    // La lista de máscaras de la salida, como la de superficies.
                    let nombres: Vec<String> = editor
                        .proyecto
                        .0
                        .salidas
                        .iter()
                        .find(|c| c.monitor == salida.id)
                        .map(|c| {
                            (1..=c.mascaras.len())
                                .map(|n| format!("Máscara {n}"))
                                .collect()
                        })
                        .unwrap_or_default();
                    let (clic, borrar) = lista_de_elementos(
                        ui,
                        egui_phosphor::regular::PROHIBIT,
                        rojo(),
                        &nombres,
                        editor.edicion.mascara,
                        "Borrar esta máscara",
                    );
                    if let Some(i) = clic {
                        editor.edicion.mascara = i;
                        editor.edicion.elegidos.clear();
                    }
                    if let Some(i) = borrar
                        && let Some(c) = editor
                            .proyecto
                            .0
                            .salidas
                            .iter_mut()
                            .find(|c| c.monitor == salida.id)
                    {
                        c.mascaras.remove(i);
                        editor.edicion.mascara = editor.edicion.mascara.saturating_sub(1);
                        editor.edicion.elegidos.clear();
                    }
                }
                ModoEdicion::Hueco => {
                    // La lista de huecos de la superficie elegida.
                    let nombres: Vec<String> =
                        superficie(editor.proyecto, &salida.id, indice_de_superficie)
                            .map(|s| (1..=s.huecos.len()).map(|n| format!("Hueco {n}")).collect())
                            .unwrap_or_default();
                    let (clic, borrar) = lista_de_elementos(
                        ui,
                        egui_phosphor::regular::SUBTRACT_SQUARE,
                        amarillo(),
                        &nombres,
                        editor.edicion.hueco,
                        "Borrar este hueco",
                    );
                    if let Some(i) = clic {
                        editor.edicion.hueco = i;
                        editor.edicion.elegidos.clear();
                    }
                    if let Some(i) = borrar
                        && let Some(sup) =
                            superficie_mut(editor.proyecto, &salida.id, indice_de_superficie)
                    {
                        sup.huecos.remove(i);
                        editor.edicion.hueco = editor.edicion.hueco.saturating_sub(1);
                        editor.edicion.elegidos.clear();
                    }
                }
                ModoEdicion::Malla => {
                    match malla.clone() {
                        // Sin malla no hay nada que mostrar: el botón «Crear malla» está arriba.
                        None => {}
                        Some(m) => {
                            fila(ui, "Puntos", |ui| {
                                ui.label(format!("{} × {}", m.columnas, m.filas));
                            });
                            // Más o menos columnas y filas: cuatro botones del mismo ancho.
                            let cambios = [
                                ("− col", -1_isize, 0_isize),
                                ("+ col", 1, 0),
                                ("− fila", 0, -1),
                                ("+ fila", 0, 1),
                            ];
                            botones_iguales(ui, 4, |ui, i| {
                                let (texto, columnas, filas) = cambios[i];
                                let nuevas_columnas = m.columnas as isize + columnas;
                                let nuevas_filas = m.filas as isize + filas;
                                let rango = PUNTOS_MINIMOS as isize..=PUNTOS_MAXIMOS as isize;
                                let permitido = rango.contains(&nuevas_columnas)
                                    && rango.contains(&nuevas_filas);
                                if ui
                                    .add_enabled_ui(permitido, |ui| boton(ui, texto))
                                    .inner
                                    .clicked()
                                {
                                    // Se conserva la deformación: los puntos nuevos toman el
                                    // valor interpolado.
                                    malla = Some(m.con_dimensiones(
                                        nuevas_columnas as usize,
                                        nuevas_filas as usize,
                                    ));
                                    malla_cambio = true;
                                    editor.edicion.nodo_malla = None;
                                }
                            });
                            if boton(ui, "Reiniciar puntos")
                                .on_hover_text("Deja todos los puntos en su lugar")
                                .clicked()
                            {
                                malla = Some(Malla::nueva(m.columnas, m.filas));
                                malla_cambio = true;
                            }
                        }
                    }
                }
                ModoEdicion::Forma => {
                    if let Some(puntos) = &mut forma {
                        let elegido = editor.edicion.punto_forma.min(puntos.len() - 1);
                        ui.label(format!("Punto {} de {}", elegido + 1, puntos.len()));
                        let mut uv = puntos[elegido];
                        let (mut cambio_u, mut cambio_v) = (false, false);
                        ui.columns(2, |c| {
                            cambio_u = campo_numerico_con(&mut c[0], &mut uv[0], |d| {
                                d.range(0.0..=1.0)
                                    .speed(0.002)
                                    .fixed_decimals(3)
                                    .prefix("u: ")
                            })
                            .changed();
                            cambio_v = campo_numerico_con(&mut c[1], &mut uv[1], |d| {
                                d.range(0.0..=1.0)
                                    .speed(0.002)
                                    .fixed_decimals(3)
                                    .prefix("v: ")
                            })
                            .changed();
                        });
                        if cambio_u || cambio_v {
                            let mut nueva = puntos.clone();
                            nueva[elegido] = uv;
                            if es_simple(&nueva) {
                                *puntos = nueva;
                                forma_cambio = true;
                            }
                        }
                    }
                }
            }
        }

        if parte == Parte::Entrada {
            // --- Recorte de la entrada ---
            seccion(ui, "Entrada (recorte de la composición)");
            for (i, nombre) in ["x mín", "y mín", "x máx", "y máx"].iter().enumerate() {
                let mut valor = entrada[i];
                fila(ui, nombre, |ui| {
                    if campo_numerico_con(ui, &mut valor, |d| {
                        d.range(0.0..=1.0).speed(0.002).fixed_decimals(3)
                    })
                    .changed()
                    {
                        let mut nueva = entrada;
                        nueva[i] = valor;
                        entrada = normalizar_entrada(nueva);
                        entrada_cambio = true;
                    }
                });
            }
            if boton(ui, "Composición entera").clicked() {
                entrada = ENTRADA_COMPLETA;
                entrada_cambio = true;
            }
        }
    });

    // Solo se escribe en el proyecto si algo cambió: así no se marca como modificado en cada frame.
    if (modificado || forma_cambio || malla_cambio || entrada_cambio)
        && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice_de_superficie)
    {
        if malla_cambio {
            sup.malla = malla;
        }
        if modificado {
            sup.esquinas = esquinas;
        }
        if forma_cambio {
            sup.forma = forma;
        }
        if entrada_cambio {
            sup.entrada = entrada;
        }
    }
}

/// Los botones que eligen qué se edita en la vista de la salida. Si la ventana es angosta, se
/// acomodan en dos líneas en vez de salirse del panel.
fn selector_de_modo(ui: &mut egui::Ui, editor: &mut ContextoEditor) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Editar:");
        use egui_phosphor::regular as icono_de;
        let modos = [
            (
                ModoEdicion::Esquinas,
                icono_de::FRAME_CORNERS,
                azul(),
                "Esquinas",
                "Mover las 4 esquinas (corner pin). Arrastrá una esquina, o la superficie por adentro para moverla entera. Con un marco sobre espacio vacío elegís varias esquinas (Shift suma).",
            ),
            (
                ModoEdicion::Forma,
                icono_de::POLYGON,
                rosa(),
                "Recorte",
                "Recorta la superficie ANTES de deformarla: el recorte se deforma junto con ella. Arrastrá un punto, o dibujá un marco para elegir varios (Shift suma). Un clic sobre un lado (donde aparece el +) agrega uno; clic derecho sobre un punto lo quita.",
            ),
            (
                ModoEdicion::Malla,
                icono_de::GRID_FOUR,
                verde(),
                "Malla",
                "Curvar la superficie con una grilla de puntos (mesh warp). Arrastrá un punto de control; con un marco elegís varios (Shift suma). Las 4 esquinas (blancas) también se arrastran y mueven las de la superficie.",
            ),
            (
                ModoEdicion::Mascara,
                icono_de::PROHIBIT,
                rojo(),
                "Máscara",
                "Tapa con negro una zona del proyector DESPUÉS de deformar: queda fija en la pared y no se mueve con la superficie. Arrastrá los puntos; un clic sobre un lado agrega uno y el clic derecho sobre un punto lo quita. Lo que cubre se proyecta en negro (sin luz).",
            ),
            (
                ModoEdicion::Hueco,
                icono_de::SUBTRACT_SQUARE,
                amarillo(),
                "Hueco",
                "Un agujero dentro de la superficie: no se dibuja y se deforma con ella (esquinas y malla). Arrastrá los puntos; un clic sobre un lado agrega uno y el clic derecho sobre un punto lo quita.",
            ),
        ];
        for (modo, glifo, color, nombre, ayuda) in modos {
            let texto = con_icono(ui, glifo, color, nombre);
            if crate::inspector::elegible(ui, editor.edicion.modo == modo, texto)
                .on_hover_text(ayuda)
                .clicked()
            {
                editor.edicion.modo = modo;
            }
        }
    });
}

/// La acción de crear o quitar del modo elegido, a todo el ancho y arriba del inspector (Recortar
/// forma, Crear malla, Agregar máscara, Agregar hueco…). Lo que se acumula (la lista de máscaras,
/// de huecos, los puntos de la malla) va abajo, en el mismo inspector, con un renglón por elemento.
fn acciones_del_modo(ui: &mut egui::Ui, editor: &mut ContextoEditor, salida: &Salida) {
    use egui_phosphor::regular as icono_de;
    let indice = editor.seleccion.superficie;
    let modo = editor.edicion.modo;
    match modo {
        ModoEdicion::Esquinas => {}
        ModoEdicion::Forma => {
            let tiene = superficie(editor.proyecto, &salida.id, indice)
                .is_some_and(|s| s.forma.is_some());
            if !tiene {
                if boton(ui, con_icono(ui, icono_de::PLUS, verde(), "Recortar forma"))
                    .on_hover_text(
                        "Parte de un rectángulo; después podés agregar puntos en los lados",
                    )
                    .clicked()
                    && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice)
                {
                    sup.forma = Some(rectangulo_unitario());
                    editor.edicion.punto_forma = 0;
                }
            } else if boton(ui, con_icono(ui, icono_de::TRASH, rojo(), "Quitar recorte")).clicked()
                && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice)
            {
                sup.forma = None;
                editor.edicion.modo = ModoEdicion::Esquinas;
            }
        }
        ModoEdicion::Malla => {
            let tiene = superficie(editor.proyecto, &salida.id, indice)
                .is_some_and(|s| s.malla.is_some());
            if !tiene {
                if boton(ui, con_icono(ui, icono_de::PLUS, verde(), "Crear malla"))
                    .on_hover_text("Una grilla de puntos de control para curvar la superficie")
                    .clicked()
                    && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice)
                {
                    sup.malla = Some(Malla::nueva(4, 4));
                    editor.edicion.nodo_malla = None;
                }
            } else if boton(ui, con_icono(ui, icono_de::TRASH, rojo(), "Quitar malla")).clicked()
                && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice)
            {
                sup.malla = None;
                editor.edicion.nodo_malla = None;
                editor.edicion.modo = ModoEdicion::Esquinas;
            }
        }
        ModoEdicion::Mascara => {
            if boton(ui, con_icono(ui, icono_de::PLUS, verde(), "Agregar máscara"))
                .on_hover_text(
                    "Un rectángulo en el centro del proyector; después se le agregan puntos y se lo mueve",
                )
                .clicked()
                && let Some(c) = editor
                    .proyecto
                    .0
                    .salidas
                    .iter_mut()
                    .find(|c| c.monitor == salida.id)
            {
                editor.edicion.mascara = c.agregar_mascara(salida.tamano.x, salida.tamano.y, 0.3);
                editor.edicion.elegidos.clear();
            }
        }
        ModoEdicion::Hueco => {
            if boton(ui, con_icono(ui, icono_de::PLUS, verde(), "Agregar hueco"))
                .on_hover_text(
                    "Un rectángulo en el centro de la superficie; después se le agregan puntos y se lo mueve",
                )
                .clicked()
                && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, indice)
            {
                editor.edicion.hueco = sup.agregar_hueco();
                editor.edicion.elegidos.clear();
            }
        }
    }
    ui.add_space(4.0);
}

/// El primer bloque de la tabla de la barra: nombre, visible y origen de la superficie elegida,
/// con las etiquetas en una columna fija y los controles alineados.
fn datos_en_linea(ui: &mut egui::Ui, editor: &mut ContextoEditor, salida: &Salida, flujo: &Flujo) {
    let actual = editor.seleccion.superficie;
    let Some((nombre, mut visible, mut origen)) = superficie(editor.proyecto, &salida.id, actual)
        .map(|s| (s.nombre_visible(actual), s.visible, s.origen))
    else {
        return;
    };
    let comp = &editor.proyecto.0.composicion;
    let mut opciones: Vec<(OrigenSuperficie, String)> =
        vec![(OrigenSuperficie::Composicion, "Composición".to_string())];
    opciones.extend((0..comp.capas.len()).map(|i| {
        let o = OrigenSuperficie::Capa(i);
        (o, o.nombre(comp))
    }));
    let nombre_origen = origen.nombre(comp);
    let mut nombre_nuevo: Option<String> = None;
    let (mut cambio_visible, mut cambio_origen) = (false, false);
    let (etiqueta, _) = etiquetas_de_la_barra(ui);

    flujo.grupo(ui, 0, |ui| {
        ui.vertical(|ui| {
            fila_con_etiqueta(ui, etiqueta, "Nombre", |ui| {
                let id = egui::Id::new(("nombre_de_superficie", &salida.id, actual));
                if let EdicionDeNombre::Confirmado(texto) =
                    campo_de_nombre(ui, id, &nombre, ui.available_width(), false)
                {
                    nombre_nuevo = Some(texto);
                }
            });
            fila_con_etiqueta(ui, etiqueta, "Visible", |ui| {
                cambio_visible = ui.checkbox(&mut visible, "").changed();
            });
            fila_con_etiqueta(ui, etiqueta, "Origen", |ui| {
                desplegable(ui, "origen_de_superficie", &nombre_origen, |ui| {
                    for (valor, texto) in &opciones {
                        if crate::inspector::opcion(ui, &mut origen, *valor, texto).changed() {
                            cambio_origen = true;
                        }
                    }
                })
                .on_hover_text("De dónde toma la imagen: la composición mezclada, o una capa sola");
            });
        });
    });

    // Solo se escribe en el proyecto si hubo un cambio.
    let nombre_nuevo = nombre_nuevo
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && *n != nombre);
    if (nombre_nuevo.is_some() || cambio_visible || cambio_origen)
        && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, actual)
    {
        if cambio_origen {
            sup.origen = origen;
        }
        if let Some(n) = nombre_nuevo {
            sup.nombre = n;
        }
        if cambio_visible {
            sup.visible = visible;
        }
    }
}

/// Editor de la salida: la vista previa del proyector, con la superficie y su forma.
///
/// - **Modo Esquinas:** se arrastran las esquinas, o toda la superficie arrastrando por adentro.
/// - **Modo Forma:** se arrastran los puntos del recorte; doble clic sobre un lado agrega uno.
///
/// Todo lo que dejaría un polígono o un cuadrilátero inválido se rechaza: la esquina o el
/// punto se queda en su última posición válida.
fn editor_salida(ui: &mut egui::Ui, editor: &mut ContextoEditor, salida: &Salida) {
    ui.label(texto_negrita("Salida (proyector)"));
    if superficie(editor.proyecto, &salida.id, editor.seleccion.superficie).is_none() {
        ui.label("Esta salida no tiene superficies.");
        return;
    }
    // Los botones de modo y, justo debajo, la barra de la superficie (nombre, posición, escala).
    // Van ANTES de copiar lo que se edita: así lo que cambian se ve en este mismo cuadro. Lo que
    // depende del modo (crear, listas) está en el inspector de la derecha.
    selector_de_modo(ui, editor);
    egui::Frame::NONE
        .fill(
            ui.visuals()
                .widgets
                .noninteractive
                .weak_bg_fill
                .gamma_multiply(0.6),
        )
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Los dos bloques de la tabla van uno al lado del otro si entran, y si no, uno debajo.
            let ranura = ranura_de_la_barra(ui, ui.available_width());
            let flujo =
                Flujo::nuevo_con_alto(ui, &anchos_de_la_barra(ui, ranura), alto_del_bloque(ui));
            datos_en_linea(ui, editor, salida, &flujo);
            herramientas_de_superficie(ui, editor, salida, Parte::Barra, Some(&flujo));
        });
    ui.add_space(6.0);
    let Some((mut esquinas, mut forma, mut malla)) =
        superficie(editor.proyecto, &salida.id, editor.seleccion.superficie)
            .map(|s| (s.esquinas, s.forma.clone(), s.malla.clone()))
    else {
        ui.label("Esta salida no tiene superficies.");
        return;
    };
    let indice_de_superficie = editor.seleccion.superficie;
    let tam = [salida.tamano.x, salida.tamano.y];
    let (mut modificado, mut forma_cambio, mut malla_cambio) = (false, false, false);

    // Las máscaras de esta salida (polígonos en píxeles del proyector, como las esquinas).
    let mut mascaras: Vec<Vec<Punto>> = editor
        .proyecto
        .0
        .salidas
        .iter()
        .find(|c| c.monitor == salida.id)
        .map(|c| c.mascaras.clone())
        .unwrap_or_default();
    let mut mascaras_cambio = false;
    let en_mascara = editor.edicion.modo == ModoEdicion::Mascara;
    editor.edicion.mascara = editor.edicion.mascara.min(mascaras.len().saturating_sub(1));
    let mascara_elegida = editor.edicion.mascara;
    // Los huecos de la superficie (en sus coordenadas, de 0 a 1: se deforman con ella).
    let mut huecos: Vec<Vec<Punto>> = superficie(editor.proyecto, &salida.id, indice_de_superficie)
        .map(|s| s.huecos.clone())
        .unwrap_or_default();
    let mut huecos_cambio = false;
    let en_hueco = editor.edicion.modo == ModoEdicion::Hueco;
    editor.edicion.hueco = editor.edicion.hueco.min(huecos.len().saturating_sub(1));
    let hueco_elegido = editor.edicion.hueco;
    // La forma real de la superficie (para seguir dibujándola mientras se edita otra cosa).
    let forma_de_superficie = forma.clone();
    if en_hueco {
        forma = huecos.get(hueco_elegido).cloned();
    }
    // En el modo Máscara, el polígono elegido se edita como una «forma» sobre la pantalla
    // entera del proyector (sus puntos van de 0 a 1 en ella): así se reutiliza toda la
    // edición de puntos de la forma (agregar, quitar, mover, elegir con un marco).
    let h_pantalla = Homografia::desde_esquinas([
        [-tam[0] / 2.0, tam[1] / 2.0],
        [tam[0] / 2.0, tam[1] / 2.0],
        [tam[0] / 2.0, -tam[1] / 2.0],
        [-tam[0] / 2.0, -tam[1] / 2.0],
    ]);
    if en_mascara {
        forma = mascaras.get(mascara_elegida).map(|m| {
            m.iter()
                .map(|p| [p[0] / tam[0] + 0.5, 0.5 - p[1] / tam[1]])
                .collect()
        });
    }
    // La malla con la que se proyecta la forma que se edita (las máscaras no tienen).
    let malla_forma: Option<Malla> = if en_mascara { None } else { malla.clone() };

    // Si se quitaron puntos, el elegido puede haber quedado fuera de rango.
    if let Some(puntos) = &forma {
        editor.edicion.punto_forma = editor.edicion.punto_forma.min(puntos.len() - 1);
    }

    // La vista previa ocupa el espacio disponible manteniendo la proporción del monitor.
    let id_textura = editor
        .texturas
        .add_image(EguiTextureHandle::Weak(salida.previa.id()));
    let (respuesta, rect_imagen) = lienzo(ui, id_textura, tam[0] / tam[1]);
    let rect_min = [rect_imagen.min.x, rect_imagen.min.y];
    let rect_tam = [rect_imagen.width(), rect_imagen.height()];
    // En el modo Máscara se edita como si fuera una forma (ver más arriba).
    let modo = if en_mascara || en_hueco {
        ModoEdicion::Forma
    } else {
        editor.edicion.modo
    };
    // Para saber qué punto de la superficie está bajo el mouse (y dónde cae cada punto de la forma).
    let h = Homografia::desde_esquinas(esquinas);
    let h_forma = if en_mascara { h_pantalla } else { h };

    // --- Selección ---
    // Los elegidos son de una superficie y de un modo: si cambió alguno, o si el punto ya no
    // existe (se quitó, o cambió la cantidad de puntos de la malla), se descartan.
    let de_esta = Some((
        salida.id.clone(),
        if en_mascara {
            10_000 + mascara_elegida
        } else if en_hueco {
            20_000 + hueco_elegido
        } else {
            indice_de_superficie
        },
    ));
    if editor.edicion.elegidos_de != de_esta {
        editor.edicion.elegidos.clear();
        editor.edicion.elegidos_de = de_esta;
    }
    editor.edicion.elegidos.retain(|asa| match (asa, modo) {
        (Asa::Esquina(_), ModoEdicion::Esquinas) => true,
        (Asa::Forma(i), ModoEdicion::Forma) => forma.as_ref().is_some_and(|f| *i < f.len()),
        (Asa::Esquina(_), ModoEdicion::Malla) => malla.is_some(),
        (Asa::Nodo(c, f), ModoEdicion::Malla) => malla
            .as_ref()
            .is_some_and(|m| *c < m.columnas && *f < m.filas && !m.es_esquina(*c, *f)),
        _ => false,
    });
    let sumar = ui.input(|i| i.modifiers.shift);
    // El punto que hay bajo el puntero en el modo actual.
    let asa_bajo = |puntero: egui::Pos2| -> Option<Asa> {
        match modo {
            ModoEdicion::Esquinas => {
                esquina_cercana(&esquinas, puntero, rect_min, rect_tam, tam).map(Asa::Esquina)
            }
            ModoEdicion::Forma => forma
                .as_ref()
                .and_then(|f| {
                    punto_forma_cercano(
                        f,
                        &h_forma,
                        malla_forma.as_ref(),
                        puntero,
                        rect_min,
                        rect_tam,
                        tam,
                    )
                })
                .map(Asa::Forma),
            ModoEdicion::Malla => {
                let asas = asas_en_salida(modo, &esquinas, None, malla.as_ref(), &h);
                asa_mas_cercana(&asas, puntero, rect_min, rect_tam, tam)
            }
            // Las máscaras y los huecos se editan como una forma: `modo` nunca vale esto acá.
            ModoEdicion::Mascara | ModoEdicion::Hueco => None,
        }
    };

    // --- Interacción con el mouse ---
    let origen = ui.input(|i| i.pointer.press_origin());

    // Al empezar a arrastrar se decide qué se agarró.
    if respuesta.drag_started()
        && let Some(origen) = origen
    {
        let origen_salida = pantalla_a_salida([origen.x, origen.y], rect_min, rect_tam, tam);
        editor.edicion.arrastre = match asa_bajo(origen) {
            // Sobre un punto: se mueve él, o todo el grupo si ya estaba elegido.
            Some(asa) => {
                editor.edicion.elegidos =
                    seleccion::al_agarrar(&editor.edicion.elegidos, asa, sumar);
                match asa {
                    Asa::Esquina(i) => editor.seleccion.esquina = i,
                    Asa::Forma(i) => editor.edicion.punto_forma = i,
                    Asa::Nodo(c, f) => editor.edicion.nodo_malla = Some((c, f)),
                }
                Some(ArrastreSalida::Grupo {
                    ultimo: origen_salida,
                })
            }
            // Lejos de las esquinas pero dentro de la superficie: se mueve entera.
            None if modo == ModoEdicion::Esquinas && contiene(&esquinas, origen_salida) => {
                Some(ArrastreSalida::Superficie {
                    ultimo: origen_salida,
                })
            }
            // Sobre espacio vacío: se dibuja un marco para elegir varios puntos.
            None => Some(ArrastreSalida::Marco {
                inicio: origen_salida,
                actual: origen_salida,
            }),
        };
    }

    // Un clic (sin arrastrar) elige la esquina o el punto más cercano.
    //
    // La posición del clic sale de `interact_pointer_pos()`, no de `press_origin()`: un clic
    // se detecta cuando se SUELTA el botón, y para entonces `press_origin()` ya es `None`
    // (ver `pruebas_egui.rs`). Al empezar un arrastre, en cambio, el botón sigue apretado.
    if respuesta.clicked()
        && let Some(origen) = respuesta.interact_pointer_pos()
    {
        let sobre = asa_bajo(origen);
        match sobre {
            Some(Asa::Esquina(i)) => editor.seleccion.esquina = i,
            Some(Asa::Forma(i)) => editor.edicion.punto_forma = i,
            Some(Asa::Nodo(c, f)) => editor.edicion.nodo_malla = Some((c, f)),
            None => {}
        }
        let mut agrego_punto = false;
        if sobre.is_none()
            && modo == ModoEdicion::Forma
            && let Some(f) = forma.as_mut()
        {
            // Un clic sobre un lado agrega un punto ahí: es el "+" que se ve al acercar el
            // mouse. Se busca el lado más cercano en pantalla, que es donde el usuario ve
            // la forma.
            let en_pantalla: Vec<Punto> = f
                .iter()
                .map(|uv| {
                    let en_salida =
                        deformacion::proyectar(&h_forma, malla_forma.as_ref(), uv[0], uv[1]);
                    salida_a_pantalla(en_salida, rect_min, rect_tam, tam)
                })
                .collect();
            if let Some((lado, t, distancia)) = lado_mas_cercano(&en_pantalla, [origen.x, origen.y])
                && distancia < DISTANCIA_LADO
                && let Some(i) = insertar_en_lado(f, lado, t)
            {
                editor.edicion.punto_forma = i;
                editor.edicion.elegidos = vec![Asa::Forma(i)];
                forma_cambio = true;
                agrego_punto = true;
            }
        }
        if !agrego_punto {
            editor.edicion.elegidos =
                seleccion::al_hacer_clic(&editor.edicion.elegidos, sobre, sumar);
        }
    }

    // Clic derecho sobre un punto de la forma: un menú para quitarlo. El punto lo elige el
    // usuario (el que está bajo el mouse); el programa no decide cuál quitar.
    if modo == ModoEdicion::Forma {
        if respuesta.secondary_clicked()
            && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
        {
            editor.edicion.punto_menu = forma.as_ref().and_then(|f| {
                punto_forma_cercano(
                    f,
                    &h_forma,
                    malla_forma.as_ref(),
                    pos,
                    rect_min,
                    rect_tam,
                    tam,
                )
            });
            if let Some(i) = editor.edicion.punto_menu {
                editor.edicion.punto_forma = i;
            }
        }
        if let Some(i) = editor.edicion.punto_menu {
            // Cómo quedaría la forma sin ese punto: tiene que seguir siendo válida.
            let sin_el_punto = forma.as_ref().filter(|f| i < f.len()).map(|f| {
                let mut menos = f.clone();
                menos.remove(i);
                menos
            });
            let se_puede = sin_el_punto
                .as_ref()
                .is_some_and(|menos| menos.len() >= 3 && es_simple(menos));
            let menu = respuesta.context_menu(|ui| {
                ui.label(format!("Punto {}", i + 1));
                if ui
                    .add_enabled(se_puede, egui::Button::new("Quitar punto"))
                    .on_disabled_hover_text(
                        "Una forma necesita al menos 3 puntos, y quitarlo no puede cruzar lados",
                    )
                    .clicked()
                {
                    forma = sin_el_punto;
                    forma_cambio = true;
                    editor.edicion.punto_forma = i.saturating_sub(1);
                    ui.close();
                }
            });
            // Cuando el menú se cierra, se olvida el punto.
            if menu.is_none() {
                editor.edicion.punto_menu = None;
            }
        }
    }

    if respuesta.dragged()
        && let Some(arrastre) = editor.edicion.arrastre
        && let Some(pos) = respuesta.interact_pointer_pos()
    {
        let en_salida = pantalla_a_salida([pos.x, pos.y], rect_min, rect_tam, tam);
        match arrastre {
            ArrastreSalida::Superficie { ultimo } => {
                let delta = [en_salida[0] - ultimo[0], en_salida[1] - ultimo[1]];
                esquinas = trasladar_esquinas(esquinas, delta);
                editor.edicion.arrastre = Some(ArrastreSalida::Superficie { ultimo: en_salida });
                modificado = true;
            }
            ArrastreSalida::Marco { inicio, .. } => {
                editor.edicion.arrastre = Some(ArrastreSalida::Marco {
                    inicio,
                    actual: en_salida,
                });
            }
            ArrastreSalida::Grupo { ultimo } => {
                let delta = [en_salida[0] - ultimo[0], en_salida[1] - ultimo[1]];
                let elegidos = editor.edicion.elegidos.clone();
                // Si el movimiento deja una figura inválida (cuadrilátero no convexo, forma
                // con lados cruzados), se ignora y `ultimo` no avanza: el grupo se queda en
                // su última posición válida y retoma cuando el puntero vuelve a un lugar válido.
                let aplicado = match modo {
                    ModoEdicion::Esquinas => {
                        let nuevas = seleccion::mover_esquinas(esquinas, &elegidos, delta);
                        if es_convexo(nuevas) {
                            esquinas = nuevas;
                            modificado = true;
                            true
                        } else {
                            false
                        }
                    }
                    ModoEdicion::Forma => match forma.as_mut() {
                        Some(f) => {
                            let nueva = seleccion::mover_forma(
                                f,
                                &elegidos,
                                delta,
                                &h_forma,
                                malla_forma.as_ref(),
                            );
                            if es_simple(&nueva) {
                                *f = nueva;
                                forma_cambio = true;
                                true
                            } else {
                                false
                            }
                        }
                        None => true,
                    },
                    ModoEdicion::Mascara | ModoEdicion::Hueco => true,
                    ModoEdicion::Malla => {
                        // Las esquinas elegidas mueven las de la superficie (si la figura
                        // sigue siendo convexa) y los puntos elegidos, la malla.
                        let nuevas = seleccion::mover_esquinas(esquinas, &elegidos, delta);
                        if es_convexo(nuevas) {
                            if nuevas != esquinas {
                                esquinas = nuevas;
                                modificado = true;
                            }
                            if let Some(m) = malla.as_mut() {
                                *m = seleccion::mover_nodos(m, &elegidos, delta);
                                malla_cambio = true;
                            }
                            true
                        } else {
                            false
                        }
                    }
                };
                if aplicado {
                    editor.edicion.arrastre = Some(ArrastreSalida::Grupo { ultimo: en_salida });
                }
            }
        }
    }
    if respuesta.drag_stopped() {
        // Al soltar un marco, quedan elegidos los puntos que cayeron adentro.
        if let Some(ArrastreSalida::Marco { inicio, actual }) = editor.edicion.arrastre {
            let marco = seleccion::rectangulo(inicio, actual);
            let adentro: Vec<Asa> = asas_en_salida(
                modo,
                &esquinas,
                forma.as_deref(),
                malla_forma.as_ref(),
                &h_forma,
            )
            .into_iter()
            .filter(|(_, p)| seleccion::dentro(marco, *p))
            .map(|(asa, _)| asa)
            .collect();
            editor.edicion.elegidos =
                seleccion::al_soltar_marco(&editor.edicion.elegidos, &adentro, sumar);
        }
        editor.edicion.arrastre = None;
    }

    // En el modo Máscara, lo editado como «forma» vuelve a ser el polígono de la máscara (en
    // píxeles del proyector). Nada de esto se escribe en la forma de la superficie.
    if en_mascara && forma_cambio {
        if let (Some(f), Some(m)) = (&forma, mascaras.get_mut(mascara_elegida)) {
            *m = f
                .iter()
                .map(|uv| [(uv[0] - 0.5) * tam[0], (0.5 - uv[1]) * tam[1]])
                .collect();
            mascaras_cambio = true;
        }
        forma_cambio = false;
    }

    // En el modo Hueco, lo editado como «forma» vuelve a ser el hueco elegido.
    if en_hueco && forma_cambio {
        if let (Some(f), Some(hueco)) = (&forma, huecos.get_mut(hueco_elegido)) {
            *hueco = f.clone();
            huecos_cambio = true;
        }
        forma_cambio = false;
    }

    // --- Dibujo encima de la vista previa ---
    // La homografía puede haber cambiado en este frame (se movió una esquina).
    let h = Homografia::desde_esquinas(esquinas);
    let h_forma = if en_mascara { h_pantalla } else { h };
    let pintor = ui.painter_at(respuesta.rect);
    let a_pos = |p: [f32; 2]| {
        let q = salida_a_pantalla(p, rect_min, rect_tam, tam);
        egui::pos2(q[0], q[1])
    };
    let esquinas_en_pantalla: Vec<egui::Pos2> = esquinas.iter().map(|e| a_pos(*e)).collect();
    // El contorno sigue la malla si la tiene (los lados se curvan).
    let contorno: Vec<egui::Pos2> = contorno_de_superficie(&h, malla.as_ref())
        .iter()
        .map(|p| a_pos(*p))
        .collect();
    pintor.add(egui::Shape::closed_line(
        contorno,
        egui::Stroke::new(2.0, azul()),
    ));

    // Los huecos de la superficie: se ve el área que no se dibuja, deformada como la superficie.
    for (j, hueco) in huecos.iter().enumerate() {
        let en_pantalla: Vec<egui::Pos2> = hueco
            .iter()
            .map(|uv| a_pos(deformacion::proyectar(&h, malla.as_ref(), uv[0], uv[1])))
            .collect();
        let mut relleno = egui::Mesh::default();
        for p in &en_pantalla {
            relleno.colored_vertex(*p, amarillo().gamma_multiply(0.28));
        }
        for [a, b, c] in vm_core::triangular(hueco) {
            relleno.add_triangle(a as u32, b as u32, c as u32);
        }
        pintor.add(egui::Shape::mesh(relleno));
        if !(en_hueco && j == hueco_elegido) {
            pintor.add(egui::Shape::closed_line(
                en_pantalla,
                egui::Stroke::new(1.5, amarillo()),
            ));
        }
    }
    // Mientras se edita una máscara o un hueco, la forma real de la superficie sigue a la vista.
    if (en_mascara || en_hueco)
        && let Some(f) = &forma_de_superficie
    {
        let contorno_forma: Vec<egui::Pos2> = f
            .iter()
            .map(|uv| a_pos(deformacion::proyectar(&h, malla.as_ref(), uv[0], uv[1])))
            .collect();
        pintor.add(egui::Shape::closed_line(
            contorno_forma,
            egui::Stroke::new(1.0, rosa()),
        ));
    }

    // Las máscaras: se ve el área que tapan y su contorno (el polígono elegido, además, se
    // dibuja con sus puntos como una forma).
    for (j, m) in mascaras.iter().enumerate() {
        let en_pantalla: Vec<egui::Pos2> = m.iter().map(|p| a_pos(*p)).collect();
        let mut relleno = egui::Mesh::default();
        for p in &en_pantalla {
            relleno.colored_vertex(*p, rojo().gamma_multiply(0.30));
        }
        for [a, b, c] in vm_core::triangular(m) {
            relleno.add_triangle(a as u32, b as u32, c as u32);
        }
        pintor.add(egui::Shape::mesh(relleno));
        if !(en_mascara && j == mascara_elegida) {
            pintor.add(egui::Shape::closed_line(
                en_pantalla,
                egui::Stroke::new(1.5, rojo()),
            ));
        }
    }

    // La malla de puntos de control: líneas que la recorren y los puntos.
    if let Some(m) = &malla
        && modo == ModoEdicion::Malla
    {
        let trazo = egui::Stroke::new(1.0, verde());
        let n = MUESTRAS_DE_CURVA;
        for fila in 0..m.filas {
            let v = fila as f32 / (m.filas - 1) as f32;
            let curva: Vec<egui::Pos2> = (0..=n)
                .map(|i| a_pos(deformacion::proyectar(&h, Some(m), i as f32 / n as f32, v)))
                .collect();
            pintor.add(egui::Shape::line(curva, trazo));
        }
        for columna in 0..m.columnas {
            let u = columna as f32 / (m.columnas - 1) as f32;
            let curva: Vec<egui::Pos2> = (0..=n)
                .map(|i| a_pos(deformacion::proyectar(&h, Some(m), u, i as f32 / n as f32)))
                .collect();
            pintor.add(egui::Shape::line(curva, trazo));
        }
        // Ayuda visual del margen: el punto que se agarraría con el mouse cerca.
        let agarrable = if editor.edicion.arrastre.is_none() {
            respuesta.hover_pos().and_then(|pos| {
                let asas = asas_en_salida(modo, &esquinas, None, malla.as_ref(), &h);
                asa_mas_cercana(&asas, pos, rect_min, rect_tam, tam)
            })
        } else {
            None
        };
        for fila in 0..m.filas {
            for columna in 0..m.columnas {
                let [u, v] = m.uv_de_nodo(columna, fila);
                let pos = a_pos(deformacion::proyectar(&h, Some(m), u, v));
                // Las esquinas de la malla son las de la superficie: se dibujan como en el
                // modo Esquinas (blancas) y se arrastran igual.
                let esquina = m.es_esquina(columna, fila);
                let asa = if esquina {
                    let derecha = columna != 0;
                    let abajo = fila != 0;
                    Asa::Esquina(match (derecha, abajo) {
                        (false, false) => 0,
                        (true, false) => 1,
                        (true, true) => 2,
                        (false, true) => 3,
                    })
                } else {
                    Asa::Nodo(columna, fila)
                };
                let elegido = !esquina && editor.edicion.nodo_malla == Some((columna, fila));
                let color = if esquina {
                    egui::Color32::WHITE
                } else if elegido {
                    rojo()
                } else {
                    verde()
                };
                pintor.circle_filled(pos, 6.0, color);
                pintor.circle_stroke(pos, 7.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                if agarrable == Some(asa) {
                    pintor.circle_stroke(
                        pos,
                        RADIO_AGARRE * 0.5,
                        egui::Stroke::new(2.0, egui::Color32::WHITE),
                    );
                }
            }
        }
    }

    if let Some(f) = &forma {
        let en_pantalla: Vec<egui::Pos2> = f
            .iter()
            .map(|uv| {
                a_pos(deformacion::proyectar(
                    &h_forma,
                    malla_forma.as_ref(),
                    uv[0],
                    uv[1],
                ))
            })
            .collect();
        let grosor = if modo == ModoEdicion::Forma { 2.0 } else { 1.0 };
        pintor.add(egui::Shape::closed_line(
            en_pantalla.clone(),
            egui::Stroke::new(grosor, rosa()),
        ));

        // Ayuda visual del margen: con el mouse cerca, se resalta lo que se agarraría (un
        // punto, para el clic derecho) o el lado y el lugar donde entraría un punto nuevo
        // (para el doble clic).
        if modo == ModoEdicion::Forma
            && editor.edicion.arrastre.is_none()
            && let Some(pos) = respuesta.hover_pos()
        {
            if let Some(i) = punto_forma_cercano(
                f,
                &h_forma,
                malla_forma.as_ref(),
                pos,
                rect_min,
                rect_tam,
                tam,
            ) {
                pintor.circle_stroke(
                    en_pantalla[i],
                    RADIO_AGARRE * 0.5,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
            } else {
                let puntos: Vec<Punto> = en_pantalla.iter().map(|p| [p.x, p.y]).collect();
                if let Some((lado, t, distancia)) = lado_mas_cercano(&puntos, [pos.x, pos.y])
                    && distancia < DISTANCIA_LADO
                {
                    let (a, b) = (
                        en_pantalla[lado],
                        en_pantalla[(lado + 1) % en_pantalla.len()],
                    );
                    pintor.line_segment([a, b], egui::Stroke::new(4.0, egui::Color32::WHITE));
                    let nuevo = a + (b - a) * t;
                    pintor.circle_filled(nuevo, 5.0, egui::Color32::WHITE);
                    pintor.text(
                        nuevo + egui::vec2(9.0, -9.0),
                        egui::Align2::LEFT_BOTTOM,
                        "+",
                        egui::FontId::proportional(16.0),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
        if modo == ModoEdicion::Forma {
            for (i, pos) in en_pantalla.iter().enumerate() {
                let elegido = editor.edicion.punto_forma == i;
                let color = if elegido { rojo() } else { rosa() };
                let lado = if elegido { 11.0 } else { 9.0 };
                let cuadro = egui::Rect::from_center_size(*pos, egui::vec2(lado, lado));
                pintor.rect_filled(cuadro, 0.0, color);
                pintor.rect_stroke(
                    cuadro,
                    0.0,
                    egui::Stroke::new(1.5, egui::Color32::BLACK),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }

    for (i, pos) in esquinas_en_pantalla.iter().enumerate() {
        let elegida = editor.seleccion.esquina == i && modo == ModoEdicion::Esquinas;
        let color = if elegida {
            rojo()
        } else if modo == ModoEdicion::Esquinas {
            egui::Color32::WHITE
        } else {
            egui::Color32::GRAY // en modo Forma las esquinas no se pueden mover
        };
        let radio = if modo == ModoEdicion::Esquinas {
            if elegida { 8.0 } else { 6.0 }
        } else {
            4.0
        };
        pintor.circle_filled(*pos, radio, color);
        pintor.circle_stroke(
            *pos,
            radio + 2.0,
            egui::Stroke::new(1.5, egui::Color32::BLACK),
        );
        if modo == ModoEdicion::Esquinas {
            pintor.text(
                *pos + egui::vec2(11.0, -11.0),
                egui::Align2::LEFT_BOTTOM,
                (i + 1).to_string(),
                egui::FontId::proportional(14.0),
                color,
            );
        }
    }

    // Los puntos elegidos se resaltan con un aro amarillo; y el marco, mientras se dibuja.
    for (asa, posicion) in asas_en_salida(modo, &esquinas, forma.as_deref(), malla.as_ref(), &h) {
        if editor.edicion.elegidos.contains(&asa) {
            pintor.circle_stroke(a_pos(posicion), 12.0, egui::Stroke::new(2.5, amarillo()));
        }
    }
    if let Some(ArrastreSalida::Marco { inicio, actual }) = editor.edicion.arrastre {
        dibujar_marco(&pintor, a_pos(inicio), a_pos(actual));
    }

    // Solo se escribe en el proyecto si algo cambió: así no se marca como modificado en cada frame.
    if (modificado || forma_cambio || malla_cambio || huecos_cambio)
        && let Some(sup) = superficie_mut(editor.proyecto, &salida.id, editor.seleccion.superficie)
    {
        if huecos_cambio {
            sup.huecos = huecos;
        }
        if malla_cambio {
            sup.malla = malla;
        }
        if modificado {
            sup.esquinas = esquinas;
        }
        if forma_cambio {
            sup.forma = forma;
        }
    }
    if mascaras_cambio
        && let Some(c) = editor
            .proyecto
            .0
            .salidas
            .iter_mut()
            .find(|c| c.monitor == salida.id)
    {
        c.mascaras = mascaras;
    }
}

#[cfg(test)]
mod pruebas_del_nombre {
    use super::*;
    use bevy_egui::egui::{Event, Key, Modifiers, Pos2, RawInput, Rect};

    /// Un frame con el campo de nombre, con los eventos dados.
    fn frame(
        ctx: &egui::Context,
        tiempo: f64,
        eventos: Vec<Event>,
        pedir_foco: bool,
    ) -> EdicionDeNombre {
        let mut resultado = EdicionDeNombre::Sigue;
        let entrada = RawInput {
            time: Some(tiempo),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0))),
            events: eventos,
            ..Default::default()
        };
        let mut salida = ctx.run_ui(entrada, |ui| {
            resultado = campo_de_nombre(
                ui,
                egui::Id::new("campo"),
                "Superficie 1",
                150.0,
                pedir_foco,
            );
        });
        salida.textures_delta.clear();
        resultado
    }

    fn tecla(tecla: Key) -> Vec<Event> {
        [true, false]
            .into_iter()
            .map(|pressed| Event::Key {
                key: tecla,
                physical_key: Some(tecla),
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
            .collect()
    }

    /// Un campo ya enfocado (como queda después de pedir el foco en el primer frame).
    fn campo_enfocado() -> (egui::Context, f64) {
        let ctx = egui::Context::default();
        frame(&ctx, 0.0, vec![], true);
        frame(&ctx, 0.02, vec![], false);
        (ctx, 0.04)
    }

    #[test]
    fn mientras_se_escribe_no_se_confirma_nada() {
        let (ctx, t) = campo_enfocado();
        let r = frame(&ctx, t, vec![Event::Text("X".into())], false);
        assert!(matches!(r, EdicionDeNombre::Sigue));
    }

    #[test]
    fn enter_confirma_con_lo_que_se_escribio() {
        let (ctx, t) = campo_enfocado();
        frame(&ctx, t, vec![Event::Text("X".into())], false);
        match frame(&ctx, t + 0.02, tecla(Key::Enter), false) {
            EdicionDeNombre::Confirmado(texto) => {
                assert!(texto.contains('X'), "se perdió lo escrito: {texto}");
                assert!(texto.contains("Superficie"), "se borró el resto: {texto}");
            }
            _ => panic!("Enter debía confirmar"),
        }
    }

    #[test]
    fn escape_cancela() {
        let (ctx, t) = campo_enfocado();
        frame(&ctx, t, vec![Event::Text("X".into())], false);
        assert!(matches!(
            frame(&ctx, t + 0.02, tecla(Key::Escape), false),
            EdicionDeNombre::Cancelado
        ));
    }

    #[test]
    fn un_campo_que_nunca_tuvo_el_foco_no_hace_nada() {
        let ctx = egui::Context::default();
        for i in 0..4 {
            let r = frame(&ctx, i as f64 * 0.02, vec![], false);
            assert!(matches!(r, EdicionDeNombre::Sigue));
        }
    }
}

#[cfg(test)]
mod pruebas_de_paneles {
    use super::*;

    #[test]
    fn los_paneles_laterales_se_achican_con_la_ventana_pero_no_crecen_de_mas() {
        // Ventana grande: los anchos de siempre.
        assert_eq!(anchos_de_paneles(1920.0), (ANCHO_ARBOL, ANCHO_PROPIEDADES));
        // Ventana chica: más angostos, y siempre dejan lugar al lienzo.
        let (arbol, propiedades) = anchos_de_paneles(800.0);
        assert!(arbol < ANCHO_ARBOL && propiedades < ANCHO_PROPIEDADES);
        assert!(800.0 - arbol - propiedades > 300.0);
        // Nunca por debajo de un mínimo legible.
        let (arbol, propiedades) = anchos_de_paneles(100.0);
        assert!(arbol >= 140.0 && propiedades >= 220.0);
    }
}
