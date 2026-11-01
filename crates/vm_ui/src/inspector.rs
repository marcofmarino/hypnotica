//! Piezas para armar paneles de propiedades alineados, como el inspector de Godot: cada
//! propiedad es una fila con la **etiqueta en una columna de ancho fijo** y el control que
//! **ocupa todo el resto del ancho**, con el mismo alto en todas las filas.
//!
//! Antes cada control tenía el ancho que le tocaba (un campo de 170 px, un deslizador de 100…),
//! y al apilarlos quedaba un borde derecho escalonado. Acá el ancho lo decide la fila, no el
//! control.

use bevy_egui::egui;

/// Alto de toda fila de propiedad: campos, desplegables, botones y deslizadores miden lo mismo.
pub(crate) const ALTO_FILA: f32 = 24.0;

/// Ancho de la columna de etiquetas. Fijo: así todos los controles de un panel arrancan en el
/// mismo x.
pub(crate) const ANCHO_ETIQUETA: f32 = 88.0;

/// Lo que ocupa la cajita numérica que egui dibuja a la derecha de un deslizador.
const CAJA_DE_VALOR: f32 = 74.0;

/// Una fila de propiedad: la etiqueta a la izquierda (en su columna fija) y, con
/// `contenido`, el control en el resto del ancho. Dentro de `contenido`,
/// `ui.available_width()` es justo el ancho que le toca al control.
///
/// La fila reserva SU rectángulo (todo el ancho × `ALTO_FILA`) y reparte adentro la etiqueta y el
/// contenido con coordenadas explícitas: el resultado no depende de cómo egui acomode un
/// `horizontal`, así que no «baila» entre cuadros ni se corre de una fila a otra.
pub(crate) fn fila<R>(
    ui: &mut egui::Ui,
    etiqueta: &str,
    contenido: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    fila_con_etiqueta(ui, ANCHO_ETIQUETA, etiqueta, contenido)
}

/// Como `fila`, con el ancho de la columna de etiquetas a elección (para tablas cuyas etiquetas
/// son más largas que las de un inspector común). Todas las filas de una tabla usan el MISMO
/// ancho, así sus controles empiezan en el mismo x.
pub(crate) fn fila_con_etiqueta<R>(
    ui: &mut egui::Ui,
    ancho_etiqueta: f32,
    etiqueta: &str,
    contenido: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ALTO_FILA),
        egui::Sense::hover(),
    );
    let (rect_etiqueta, rect_contenido) =
        partir_fila(rect, ancho_etiqueta, ui.spacing().item_spacing.x);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect_etiqueta)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(egui::Label::new(etiqueta).truncate());
        },
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect_contenido)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        contenido,
    )
    .inner
}

/// Parte el rectángulo de una fila en la etiqueta (`ancho_etiqueta`) y el contenido (el resto,
/// después de un espacio). Función pura: la usan `fila` y sus pruebas.
pub(crate) fn partir_fila(
    rect: egui::Rect,
    ancho_etiqueta: f32,
    espacio: f32,
) -> (egui::Rect, egui::Rect) {
    let corte = (rect.left() + ancho_etiqueta).min(rect.right());
    (
        egui::Rect::from_min_max(rect.min, egui::pos2(corte, rect.max.y)),
        egui::Rect::from_min_max(
            egui::pos2((corte + espacio).min(rect.right()), rect.min.y),
            rect.max,
        ),
    )
}

/// Como `fila`, pero la etiqueta es un botón que se puede marcar como elegido (para elegir, por
/// ejemplo, una de las cuatro esquinas). Devuelve si se hizo clic en la etiqueta y lo que
/// devuelva `contenido`.
pub(crate) fn fila_elegible<R>(
    ui: &mut egui::Ui,
    etiqueta: &str,
    elegida: bool,
    contenido: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ALTO_FILA),
        egui::Sense::hover(),
    );
    let (rect_etiqueta, rect_contenido) =
        partir_fila(rect, ANCHO_ETIQUETA, ui.spacing().item_spacing.x);
    let clic = elegible_en(ui, rect_etiqueta, etiqueta, elegida, etiqueta, false).clicked();
    let r = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect_contenido)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            contenido,
        )
        .inner;
    (clic, r)
}

/// Una lista de elementos como la de superficies: un renglón por elemento, con su ícono, el
/// nombre (marcado si está elegido) y la papelera siempre al final, en la misma columna en todos
/// los renglones. Devuelve `(elemento en el que se hizo clic, elemento a borrar)`.
///
/// Cada renglón reserva su rectángulo y coloca adentro el nombre y la papelera con coordenadas
/// explícitas (antes, la papelera caía unos píxeles más a la derecha en cada renglón).
pub(crate) fn lista_de_elementos(
    ui: &mut egui::Ui,
    glifo: &str,
    color: egui::Color32,
    nombres: &[String],
    elegido: usize,
    ayuda_borrar: &str,
) -> (Option<usize>, Option<usize>) {
    let (mut clic, mut borrar) = (None, None);
    for (i, nombre) in nombres.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ALTO_FILA),
            egui::Sense::hover(),
        );
        let papelera = egui::Rect::from_min_size(
            egui::pos2(rect.right() - ALTO_FILA, rect.top()),
            egui::vec2(ALTO_FILA, ALTO_FILA),
        );
        let nombre_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(
                (papelera.left() - ui.spacing().item_spacing.x).max(rect.left()),
                rect.bottom(),
            ),
        );
        // Ícono y nombre, alineados a la izquierda, como en el árbol de superficies.
        let texto = crate::estilo::con_icono(ui, glifo, color, nombre);
        let respuesta = elegible_en(ui, nombre_rect, ("lista", i), elegido == i, texto, false);
        if respuesta.clicked() {
            clic = Some(i);
        }
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(papelera), |ui| {
                boton_cuadrado(ui, icono_papelera())
            })
            .inner;
        if r.on_hover_text(ayuda_borrar.to_string()).clicked() {
            borrar = Some(i);
        }
    }
    (clic, borrar)
}

fn icono_papelera() -> egui::RichText {
    crate::estilo::icono(egui_phosphor::regular::TRASH, crate::estilo::rojo())
}

/// Un encabezado de sección con un «ⓘ» a la derecha que explica la sección al pasar el mouse
/// (así el texto de ayuda no ocupa renglones del panel).
pub(crate) fn seccion_con_ayuda(ui: &mut egui::Ui, titulo: &str, ayuda: &str) {
    ui.add_space(6.0);
    let (rect, respuesta) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ALTO_FILA),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().widgets.noninteractive.weak_bg_fill);
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        titulo,
        egui::FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
    );
    ui.painter().text(
        rect.right_center() - egui::vec2(10.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        egui_phosphor::regular::INFO,
        egui::FontId::proportional(14.0),
        crate::estilo::texto_suave(),
    );
    respuesta.on_hover_text(ayuda.to_string());
    ui.add_space(2.0);
}

/// Reparte grupos de controles (cada uno con su ancho) en filas: un grupo nunca se parte, y si no
/// entra en lo que queda de la fila, pasa entero a la siguiente. Devuelve `(fila, x)` de cada
/// grupo. Función pura. Sirve para que lo que pertenece junto (una propiedad y sus botones)
/// siga junto cuando la ventana es angosta.
pub(crate) fn repartir_en_filas(
    anchos: &[f32],
    disponible: f32,
    espacio: f32,
) -> Vec<(usize, f32)> {
    let mut lugares = Vec::with_capacity(anchos.len());
    let (mut fila, mut x) = (0usize, 0.0_f32);
    for &ancho in anchos {
        // Si no entra y la fila ya tiene algo, baja. (Un grupo más ancho que todo el espacio se
        // queda solo en su fila.)
        if x > 0.0 && x + ancho > disponible {
            fila += 1;
            x = 0.0;
        }
        lugares.push((fila, x));
        x += ancho + espacio;
    }
    lugares
}

/// Lo que mide un texto de una línea con la tipografía normal.
pub(crate) fn ancho_de_texto(ui: &egui::Ui, texto: &str) -> f32 {
    let fuente = egui::TextStyle::Body.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(texto.to_string(), fuente, egui::Color32::WHITE)
        .size()
        .x
}

/// Grupos de controles repartidos en filas (ver `repartir_en_filas`). Se crea una vez con el
/// ancho de cada grupo —lo que reserva el alto de todas las filas— y después cada grupo se dibuja
/// en su lugar con `grupo`. Cada grupo es una fila de `ALTO_FILA` con todo centrado
/// verticalmente, así las etiquetas y los botones quedan a la misma altura.
pub(crate) struct Flujo {
    lugares: Vec<egui::Rect>,
    disponible: f32,
}

impl Flujo {
    /// Como `nuevo`, pero cada grupo mide `alto_de_grupo` de alto (una tabla de varias filas).
    pub(crate) fn nuevo_con_alto(ui: &mut egui::Ui, anchos: &[f32], alto_de_grupo: f32) -> Flujo {
        let espacio = ui.spacing().item_spacing.x;
        let separacion_vertical = ui.spacing().item_spacing.y;
        let disponible = ui.available_width();
        let lugares = repartir_en_filas(anchos, disponible, espacio);
        let filas = lugares.iter().map(|(f, _)| f + 1).max().unwrap_or(0);
        let alto =
            filas as f32 * alto_de_grupo + filas.saturating_sub(1) as f32 * separacion_vertical;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(disponible, alto.max(0.0)), egui::Sense::hover());
        let rects = lugares
            .iter()
            .zip(anchos)
            .map(|(&(fila, x), &ancho)| {
                egui::Rect::from_min_size(
                    egui::pos2(
                        rect.left() + x,
                        rect.top() + fila as f32 * (alto_de_grupo + separacion_vertical),
                    ),
                    egui::vec2(ancho, alto_de_grupo),
                )
            })
            .collect();
        Flujo {
            lugares: rects,
            disponible,
        }
    }

    /// El ancho total sobre el que se repartieron los grupos.
    pub(crate) fn ancho(&self) -> f32 {
        self.disponible
    }

    /// Dibuja el grupo `i` en su lugar.
    pub(crate) fn grupo<R>(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        contenido: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(self.lugares[i])
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            contenido,
        )
        .inner
    }
}

/// Dibuja `contenido` DENTRO de `rect` y recortado a él: lo que se pase se corta, en vez de
/// empujar al contenedor. Lo usan las columnas de la zona de abajo, cuyo tamaño sale de un reparto
/// calculado aparte (`reparto`): el contenido nunca puede agrandarlas.
pub(crate) fn contenido_en_rect(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    contenido: impl FnOnce(&mut egui::Ui),
) {
    let mut hijo = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    hijo.set_clip_rect(rect.intersect(ui.clip_rect()));
    contenido(&mut hijo);
}

/// Un encabezado de sección: una barra que ocupa todo el ancho, con el título adentro (como
/// «Transform» o «Visibility» en Godot). Separa los grupos de propiedades.
pub(crate) fn seccion(ui: &mut egui::Ui, titulo: &str) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ALTO_FILA),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().widgets.noninteractive.weak_bg_fill);
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        titulo,
        egui::FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
    );
    ui.add_space(2.0);
}

/// Un campo de texto de una línea que ocupa todo el ancho que queda.
pub(crate) fn campo_de_texto(ui: &mut egui::Ui, texto: &mut String) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), ALTO_FILA],
        egui::TextEdit::singleline(texto),
    )
}

/// Un deslizador con su cajita numérica que ocupa todo el ancho que queda, menos `reservado`
/// píxeles para lo que haya a la derecha (un botón, por ejemplo).
pub(crate) fn deslizador(
    ui: &mut egui::Ui,
    valor: &mut f32,
    rango: std::ops::RangeInclusive<f32>,
    sufijo: &str,
    decimales: usize,
    reservado: f32,
) -> egui::Response {
    deslizador_con(ui, valor, rango, reservado, |s| {
        s.suffix(sufijo.to_string()).fixed_decimals(decimales)
    })
}

/// Lo mismo, pero `ajustar` termina de configurar el deslizador (escala logarítmica, sufijo…).
pub(crate) fn deslizador_con(
    ui: &mut egui::Ui,
    valor: &mut f32,
    rango: std::ops::RangeInclusive<f32>,
    reservado: f32,
    ajustar: impl FnOnce(egui::Slider<'_>) -> egui::Slider<'_>,
) -> egui::Response {
    ui.scope(|ui| {
        let separacion = ui.spacing().item_spacing.x;
        ui.spacing_mut().slider_width =
            (ui.available_width() - CAJA_DE_VALOR - separacion - reservado).max(40.0);
        ui.add(ajustar(egui::Slider::new(valor, rango)))
    })
    .inner
}

/// Un número editable (arrastrando o escribiendo) que ocupa todo el ancho que queda.
pub(crate) fn campo_numerico(
    ui: &mut egui::Ui,
    valor: &mut f32,
    prefijo: &str,
    velocidad: f64,
) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), ALTO_FILA],
        egui::DragValue::new(valor)
            .speed(velocidad)
            .prefix(prefijo.to_string()),
    )
}

/// Lo mismo, pero `ajustar` termina de configurar el campo (rango, decimales, prefijo…).
pub(crate) fn campo_numerico_con(
    ui: &mut egui::Ui,
    valor: &mut f32,
    ajustar: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>,
) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), ALTO_FILA],
        ajustar(egui::DragValue::new(valor)),
    )
}

/// Un desplegable que ocupa todo el ancho que queda.
pub(crate) fn desplegable(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    elegido: &str,
    opciones: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    egui::ComboBox::from_id_salt(id)
        .selected_text(elegido)
        .width(ui.available_width())
        .show_ui(ui, opciones)
        .response
}

/// Un botón «elegible» (una pestaña, un modo, una opción de un desplegable, un renglón de una
/// lista) cuyo TAMAÑO NO DEPENDE de si está resaltado.
///
/// El botón seleccionable de egui no tiene marco cuando está inactivo y sí lo tiene (con su
/// borde) al pasar el mouse o al elegirlo; ese borde suma 2 px de ancho, así que al hacer hover
/// el botón crecía y EMPUJABA a sus vecinos («temblaban» las pestañas y las entradas de los
/// desplegables). Este se pinta siempre dentro de un rectángulo del mismo tamaño y solo cambia de
/// color y de borde (por dentro).
pub(crate) fn elegible(
    ui: &mut egui::Ui,
    elegido: bool,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    let relleno = ui.spacing().button_padding;
    let galley = texto.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    // En un menú (layout justificado) la opción ocupa todo el ancho; suelta, lo que mide su texto.
    let ancho = if ui.layout().horizontal_justify() {
        ui.available_width()
    } else {
        galley.size().x + 2.0 * relleno.x
    };
    let alto = (galley.size().y + 2.0 * relleno.y).max(ui.spacing().interact_size.y);
    let (rect, respuesta) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::click());
    pintar_elegible(ui, rect, &respuesta, elegido, galley, false);
    respuesta
}

/// Como `elegible`, pero en un rectángulo dado (que ya se reservó) y con el texto centrado o a la
/// izquierda. `sal` distingue los elegibles de un mismo `Ui`.
pub(crate) fn elegible_en(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    sal: impl std::hash::Hash + std::fmt::Debug,
    elegido: bool,
    texto: impl Into<egui::WidgetText>,
    centrado: bool,
) -> egui::Response {
    let relleno = ui.spacing().button_padding;
    let galley = texto.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - 2.0 * relleno.x).max(1.0),
        egui::TextStyle::Button,
    );
    let respuesta = ui.interact(rect, ui.id().with(("elegible", sal)), egui::Sense::click());
    pintar_elegible(ui, rect, &respuesta, elegido, galley, centrado);
    respuesta
}

/// Un elegible de todo el ancho disponible y alto de fila, con el texto centrado (los
/// interruptores como «Bucle» o «Silencio»). Cada uno recibe su propio id al reservar su lugar
/// (con una «sal» fija, dos en el mismo panel chocaban: «Second use of widget ID»).
pub(crate) fn boton_elegible(
    ui: &mut egui::Ui,
    elegido: bool,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    elegible_de_tamano(
        ui,
        egui::vec2(ui.available_width(), ALTO_FILA),
        elegido,
        texto,
    )
}

/// Un interruptor cuadrado de `ALTO_FILA` × `ALTO_FILA` para un ícono.
pub(crate) fn boton_cuadrado_elegible(
    ui: &mut egui::Ui,
    elegido: bool,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    elegible_de_tamano(ui, egui::vec2(ALTO_FILA, ALTO_FILA), elegido, texto)
}

/// Reserva `tamano` (con un id propio, único) y pinta ahí un elegible con el texto centrado.
fn elegible_de_tamano(
    ui: &mut egui::Ui,
    tamano: egui::Vec2,
    elegido: bool,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    let relleno = ui.spacing().button_padding;
    let galley = texto.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (tamano.x - 2.0 * relleno.x).max(1.0),
        egui::TextStyle::Button,
    );
    let (rect, respuesta) = ui.allocate_exact_size(tamano, egui::Sense::click());
    pintar_elegible(ui, rect, &respuesta, elegido, galley, true);
    respuesta
}

/// Lo que reemplaza a `selectable_value`: marca `valor` como elegido en `actual` al hacer clic.
pub(crate) fn opcion<V: PartialEq>(
    ui: &mut egui::Ui,
    actual: &mut V,
    valor: V,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    let mut respuesta = elegible(ui, *actual == valor, texto);
    if respuesta.clicked() && *actual != valor {
        *actual = valor;
        respuesta.mark_changed();
    }
    respuesta
}

/// Pinta un elegible: borde y relleno por DENTRO del rectángulo (no cambian su tamaño).
fn pintar_elegible(
    ui: &egui::Ui,
    rect: egui::Rect,
    respuesta: &egui::Response,
    elegido: bool,
    galley: std::sync::Arc<egui::Galley>,
    centrado: bool,
) {
    let v = ui.visuals();
    let (relleno, trazo, color_de_texto) = if elegido {
        (
            v.selection.bg_fill,
            v.selection.stroke,
            v.strong_text_color(),
        )
    } else if respuesta.hovered() || respuesta.has_focus() {
        (
            v.widgets.hovered.weak_bg_fill,
            v.widgets.hovered.bg_stroke,
            v.widgets.hovered.fg_stroke.color,
        )
    } else {
        (
            egui::Color32::TRANSPARENT,
            egui::Stroke::NONE,
            v.widgets.inactive.fg_stroke.color,
        )
    };
    let esquinas = v.widgets.inactive.corner_radius;
    let relleno_interno = ui.spacing().button_padding;
    let pos = if centrado {
        rect.center() - galley.size() / 2.0
    } else {
        egui::pos2(
            rect.left() + relleno_interno.x,
            rect.center().y - galley.size().y / 2.0,
        )
    };
    let pintor = ui.painter();
    pintor.rect(rect, esquinas, relleno, trazo, egui::StrokeKind::Inside);
    pintor.galley(pos, galley.clone(), color_de_texto);
    respuesta.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            elegido,
            galley.text(),
        )
    });
}

/// Un botón cuadrado de `ALTO_FILA` × `ALTO_FILA` para un ícono. Lleva poco relleno: con el
/// relleno normal un ícono necesita más de 24 px, el botón se sale de su lugar y egui agranda el
/// espacio disponible de todo lo que viene después (las filas siguientes quedaban corridas).
pub(crate) fn boton_cuadrado(
    ui: &mut egui::Ui,
    texto: impl Into<egui::WidgetText>,
) -> egui::Response {
    boton_cuadrado_con(ui, egui::Button::new(texto))
}

/// Lo mismo, con el botón ya armado (por ejemplo, uno que se puede marcar).
pub(crate) fn boton_cuadrado_con(ui: &mut egui::Ui, boton: egui::Button<'_>) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);
        ui.add_sized([ALTO_FILA, ALTO_FILA], boton)
    })
    .inner
}

/// Un botón que ocupa todo el ancho que queda.
pub(crate) fn boton(ui: &mut egui::Ui, texto: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add_sized([ui.available_width(), ALTO_FILA], egui::Button::new(texto))
}

/// Una fila de `n` botones del mismo ancho (se reparten todo el ancho) y del mismo alto.
/// `dibujar(ui, i)` dibuja el botón `i` dentro de su columna, con `boton(ui, …)` por ejemplo.
pub(crate) fn botones_iguales(
    ui: &mut egui::Ui,
    n: usize,
    mut dibujar: impl FnMut(&mut egui::Ui, usize),
) {
    ui.columns(n, |columnas| {
        for (i, columna) in columnas.iter_mut().enumerate() {
            dibujar(columna, i);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Corre un frame de egui sin ventana y devuelve el rectángulo que ocupó cada control.
    fn medir(contenido: impl FnOnce(&mut egui::Ui, &mut Vec<egui::Rect>)) -> Vec<egui::Rect> {
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        // `run_ui` pide un closure que se pueda llamar más de una vez; el nuestro se usa una.
        let mut contenido = Some(contenido);
        let mut salida = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| {
                if let Some(c) = contenido.take() {
                    c(ui, &mut rects)
                }
            },
        );
        // Sin GPU no hay a dónde subir las texturas que egui pide: se descartan a propósito.
        salida.textures_delta.clear();
        rects
    }

    /// Lo que se busca: todos los controles de un panel terminan en el mismo borde derecho y
    /// arrancan en el mismo x, midan lo que midan sus etiquetas.
    #[test]
    fn los_controles_de_filas_distintas_quedan_alineados() {
        let r = medir(|ui, rects| {
            let mut texto = "Capa 1".to_string();
            let mut numero = 3.0_f32;
            let mut otro = 40.0_f32;
            fila(ui, "Nombre", |ui| {
                rects.push(campo_de_texto(ui, &mut texto).rect)
            });
            fila(ui, "Un nombre largo", |ui| {
                rects.push(campo_numerico(ui, &mut numero, "x: ", 1.0).rect)
            });
            fila(ui, "Opac.", |ui| {
                rects.push(deslizador(ui, &mut otro, 0.0..=100.0, " %", 0, 0.0).rect)
            });
        });
        assert_eq!(r.len(), 3);
        // El campo de texto y el numérico: mismo x de arranque y mismo borde derecho.
        assert!((r[0].left() - r[1].left()).abs() < 0.5, "{r:?}");
        assert!((r[0].right() - r[1].right()).abs() < 0.5, "{r:?}");
        // El deslizador también arranca ahí.
        assert!((r[0].left() - r[2].left()).abs() < 0.5, "{r:?}");
        // Y todos miden lo mismo de alto.
        assert!((r[0].height() - r[1].height()).abs() < 0.5, "{r:?}");
    }

    #[test]
    fn los_botones_iguales_miden_lo_mismo() {
        let r = medir(|ui, rects| {
            botones_iguales(ui, 3, |ui, i| {
                rects.push(boton(ui, ["x", "Centrar", "Un botón largo"][i]).rect);
            });
        });
        assert_eq!(r.len(), 3);
        for par in r.windows(2) {
            assert!((par[0].width() - par[1].width()).abs() < 0.5, "{r:?}");
            assert!((par[0].height() - par[1].height()).abs() < 0.5, "{r:?}");
        }
    }
}

#[cfg(test)]
mod pruebas_de_lista {
    use super::*;

    /// Una lista con nombres de largos muy distintos se dibuja sin fallar y sin pedir nada.
    #[test]
    fn la_lista_se_dibuja_sin_cambiar_nada() {
        let ctx = egui::Context::default();
        let nombres: Vec<String> = ["A", "Un nombre bastante más largo", "Máscara 3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut ancho_de_panel = 0.0;
        let mut salida = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                ancho_de_panel = ui.available_width();
                let (a, b) =
                    lista_de_elementos(ui, "x", egui::Color32::WHITE, &nombres, 1, "Borrar");
                assert!(a.is_none() && b.is_none());
            },
        );
        salida.textures_delta.clear();
        assert!(ancho_de_panel > 100.0);
    }
}

#[cfg(test)]
mod pruebas_de_filas {
    use super::*;

    #[test]
    fn la_fila_se_parte_en_etiqueta_y_contenido_sin_huecos_ni_solapes() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 24.0));
        let (etiqueta, contenido) = partir_fila(rect, 88.0, 8.0);
        assert_eq!(etiqueta.left(), 10.0);
        assert_eq!(etiqueta.width(), 88.0);
        assert_eq!(contenido.left(), 10.0 + 88.0 + 8.0);
        assert_eq!(contenido.right(), rect.right());
        assert!(etiqueta.right() <= contenido.left());
        // Alto completo en las dos.
        assert_eq!(etiqueta.height(), 24.0);
        assert_eq!(contenido.height(), 24.0);
        // En una fila más angosta que la etiqueta, el contenido queda vacío (no negativo).
        let chica = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(50.0, 24.0));
        let (e, c) = partir_fila(chica, 88.0, 8.0);
        assert_eq!(e.right(), 50.0);
        assert!(c.width() >= 0.0);
    }
}

#[cfg(test)]
mod pruebas_de_flujo {
    use super::*;

    #[test]
    fn un_grupo_que_no_entra_pasa_entero_a_la_fila_de_abajo() {
        // Cuatro grupos en 500 px con 8 de espacio: 200, 150 | 180 | 90 (el 3.º no entra tras 200+150).
        let lugares = repartir_en_filas(&[200.0, 150.0, 180.0, 90.0], 500.0, 8.0);
        assert_eq!(lugares[0], (0, 0.0));
        assert_eq!(lugares[1], (0, 208.0));
        // 208 + 150 + 8 = 366; 366 + 180 = 546 > 500: baja.
        assert_eq!(lugares[2], (1, 0.0));
        // 188 + 90 entra en la segunda fila.
        assert_eq!(lugares[3], (1, 188.0));
    }

    #[test]
    fn con_mucho_lugar_todo_queda_en_una_fila() {
        let lugares = repartir_en_filas(&[100.0, 100.0, 100.0], 1000.0, 8.0);
        assert!(lugares.iter().all(|(f, _)| *f == 0));
    }

    #[test]
    fn un_grupo_mas_ancho_que_todo_se_queda_solo_en_su_fila() {
        let lugares = repartir_en_filas(&[100.0, 900.0, 50.0], 500.0, 8.0);
        assert_eq!(lugares[0], (0, 0.0));
        assert_eq!(lugares[1], (1, 0.0));
        // Después del ancho, el siguiente baja a otra fila.
        assert_eq!(lugares[2], (2, 0.0));
    }

    #[test]
    fn los_grupos_no_se_pisan_entre_si() {
        let anchos = [220.0, 330.0, 280.0, 300.0];
        for disponible in [300.0, 450.0, 600.0, 800.0, 1000.0, 1300.0] {
            let lugares = repartir_en_filas(&anchos, disponible, 8.0);
            for i in 0..anchos.len() {
                for j in (i + 1)..anchos.len() {
                    if lugares[i].0 == lugares[j].0 {
                        assert!(
                            lugares[i].1 + anchos[i] <= lugares[j].1 + 0.001,
                            "{disponible}: el grupo {i} pisa al {j}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod pruebas_de_ids {
    use super::*;

    /// Dos interruptores en el mismo panel (como «Bucle» y «Silencio») no pueden compartir id.
    #[test]
    fn los_interruptores_de_un_mismo_panel_tienen_ids_distintos() {
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        let mut salida = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                ids.push(boton_elegible(ui, false, "Bucle").id);
                ids.push(boton_elegible(ui, true, "Silencio").id);
                ids.push(boton_cuadrado_elegible(ui, false, "a").id);
                ids.push(boton_cuadrado_elegible(ui, false, "b").id);
                ids.push(elegible(ui, false, "uno").id);
                ids.push(elegible(ui, false, "dos").id);
            },
        );
        // Sin GPU no hay a dónde subir las texturas que egui pide: se descartan a propósito.
        salida.textures_delta.clear();
        let mut distintos = ids.clone();
        distintos.sort_by_key(|i| i.value());
        distintos.dedup();
        assert_eq!(distintos.len(), ids.len(), "hay ids repetidos: {ids:?}");
    }
}
