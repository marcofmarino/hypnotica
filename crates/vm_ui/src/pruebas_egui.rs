//! Pruebas de cómo egui entrega los eventos del mouse.
//!
//! El editor decide qué hacer con `double_clicked()`, `secondary_clicked()` y
//! `interact_pointer_pos()`. Como no hay forma cómoda de usar el mouse en una prueba de
//! la aplicación entera, acá se simulan los eventos en un frame de egui sin ventana, para
//! comprobar que esas llamadas se comportan como el editor supone.

use bevy_egui::egui::{self, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Sense};

/// Lo que se pudo medir de un área sensible en un frame.
#[derive(Default, Debug)]
struct Medicion {
    clic: bool,
    doble_clic: bool,
    clic_derecho: bool,
    posicion_de_interaccion: Option<Pos2>,
    ultima_posicion: Option<Pos2>,
    /// Lo que devuelve `press_origin()`: dónde empezó la pulsación, solo si hay un botón apretado.
    origen_de_pulsacion: Option<Pos2>,
}

/// El área sensible: como la imagen de la vista previa (`click_and_drag`).
fn area() -> Rect {
    Rect::from_min_size(Pos2::new(50.0, 50.0), egui::vec2(200.0, 200.0))
}

/// Corre un frame de egui con los eventos dados y mide el área sensible.
fn frame(ctx: &egui::Context, tiempo: f64, eventos: Vec<Event>) -> Medicion {
    let mut medicion = Medicion::default();
    let entrada = RawInput {
        time: Some(tiempo),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0))),
        events: eventos,
        ..Default::default()
    };
    let mut salida = ctx.run_ui(entrada, |ui| {
        let r = ui.allocate_rect(area(), Sense::click_and_drag());
        medicion.clic = r.clicked();
        medicion.doble_clic = r.double_clicked();
        medicion.clic_derecho = r.secondary_clicked();
        medicion.posicion_de_interaccion = r.interact_pointer_pos();
        medicion.ultima_posicion = ui.input(|i| i.pointer.latest_pos());
        medicion.origen_de_pulsacion = ui.input(|i| i.pointer.press_origin());
    });
    // Sin GPU no hay a dónde subir las texturas que egui pide: se descartan a propósito.
    salida.textures_delta.clear();
    medicion
}

fn mover(pos: Pos2) -> Event {
    Event::PointerMoved(pos)
}

fn boton(pos: Pos2, boton: PointerButton, apretado: bool) -> Event {
    Event::PointerButton {
        pos,
        button: boton,
        pressed: apretado,
        modifiers: Modifiers::NONE,
    }
}

/// Un contexto "calentado": egui decide sobre qué está el mouse con lo que se dibujó en
/// el frame anterior, así que hacen falta un par de frames antes de los eventos.
fn contexto_listo(pos: Pos2) -> (egui::Context, f64) {
    let ctx = egui::Context::default();
    frame(&ctx, 0.0, vec![]);
    frame(&ctx, 0.02, vec![mover(pos)]);
    (ctx, 0.04)
}

#[test]
fn un_clic_simple_se_detecta_con_su_posicion() {
    let pos = Pos2::new(120.0, 100.0);
    let (ctx, t) = contexto_listo(pos);
    frame(&ctx, t, vec![boton(pos, PointerButton::Primary, true)]);
    let m = frame(
        &ctx,
        t + 0.05,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    assert!(m.clic, "{m:?}");
    assert!(!m.doble_clic, "{m:?}");
    assert_eq!(m.posicion_de_interaccion, Some(pos), "{m:?}");
}

#[test]
fn un_doble_clic_se_detecta_y_trae_la_posicion_en_el_mismo_frame() {
    // Es la condición que usa el editor para agregar un punto en un lado.
    let pos = Pos2::new(120.0, 100.0);
    let (ctx, t) = contexto_listo(pos);
    frame(&ctx, t, vec![boton(pos, PointerButton::Primary, true)]);
    frame(
        &ctx,
        t + 0.05,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    frame(
        &ctx,
        t + 0.15,
        vec![boton(pos, PointerButton::Primary, true)],
    );
    let m = frame(
        &ctx,
        t + 0.20,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    assert!(m.doble_clic, "no se detectó el doble clic: {m:?}");
    assert_eq!(
        m.posicion_de_interaccion,
        Some(pos),
        "el doble clic no trajo posición: {m:?}"
    );
}

#[test]
fn dos_clics_muy_separados_en_el_tiempo_no_son_un_doble_clic() {
    let pos = Pos2::new(120.0, 100.0);
    let (ctx, t) = contexto_listo(pos);
    frame(&ctx, t, vec![boton(pos, PointerButton::Primary, true)]);
    frame(
        &ctx,
        t + 0.05,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    frame(
        &ctx,
        t + 2.0,
        vec![boton(pos, PointerButton::Primary, true)],
    );
    let m = frame(
        &ctx,
        t + 2.05,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    assert!(!m.doble_clic, "{m:?}");
}

#[test]
fn un_clic_derecho_se_detecta_y_se_conoce_donde_fue() {
    // Es la condición que usa el editor para abrir el menú de un punto.
    let pos = Pos2::new(180.0, 140.0);
    let (ctx, t) = contexto_listo(pos);
    frame(&ctx, t, vec![boton(pos, PointerButton::Secondary, true)]);
    let m = frame(
        &ctx,
        t + 0.05,
        vec![boton(pos, PointerButton::Secondary, false)],
    );
    assert!(m.clic_derecho, "no se detectó el clic derecho: {m:?}");
    assert!(!m.clic, "un clic derecho no es un clic normal: {m:?}");
    assert_eq!(
        m.ultima_posicion,
        Some(pos),
        "no se conoció la posición del clic derecho: {m:?}"
    );
}

#[test]
fn un_clic_fuera_del_area_no_cuenta() {
    let dentro = Pos2::new(120.0, 100.0);
    let fuera = Pos2::new(350.0, 350.0);
    let (ctx, t) = contexto_listo(dentro);
    frame(
        &ctx,
        t,
        vec![mover(fuera), boton(fuera, PointerButton::Primary, true)],
    );
    let m = frame(
        &ctx,
        t + 0.05,
        vec![boton(fuera, PointerButton::Primary, false)],
    );
    assert!(!m.clic && !m.doble_clic, "{m:?}");
}

#[test]
fn en_el_frame_del_clic_el_origen_de_la_pulsacion_ya_no_existe_pero_la_posicion_de_interaccion_si()
{
    // Este test existe por un error real: el editor leía `press_origin()` al detectar un
    // clic, y como `clicked()` se vuelve verdadero cuando se SUELTA el botón, `press_origin()`
    // ya es `None` y el clic no hacía nada. Para saber dónde se hizo clic hay que usar
    // `interact_pointer_pos()`. (Al EMPEZAR un arrastre el botón sigue apretado y
    // `press_origin()` sí sirve.)
    let pos = Pos2::new(120.0, 100.0);
    let (ctx, t) = contexto_listo(pos);
    let al_apretar = frame(&ctx, t, vec![boton(pos, PointerButton::Primary, true)]);
    assert_eq!(al_apretar.origen_de_pulsacion, Some(pos), "{al_apretar:?}");

    let al_soltar = frame(
        &ctx,
        t + 0.05,
        vec![boton(pos, PointerButton::Primary, false)],
    );
    assert!(al_soltar.clic, "{al_soltar:?}");
    assert_eq!(al_soltar.origen_de_pulsacion, None, "{al_soltar:?}");
    assert_eq!(
        al_soltar.posicion_de_interaccion,
        Some(pos),
        "{al_soltar:?}"
    );
}

// ---------------------------------------------------------------------------------------
// Fondo de fila: un área sensible creada ANTES que los botones de la fila
// ---------------------------------------------------------------------------------------

/// Lo que se midió en una fila de prueba: un fondo que la cubre entera y un botón adentro.
#[derive(Default, Debug)]
struct MedicionDeFila {
    fondo_clic: bool,
    fondo_arrastre_empezo: bool,
    fondo_arrastrando: bool,
    boton_clic: bool,
}

fn fila() -> Rect {
    Rect::from_min_size(Pos2::new(20.0, 100.0), egui::vec2(300.0, 60.0))
}

fn rect_del_boton() -> Rect {
    Rect::from_min_size(Pos2::new(30.0, 110.0), egui::vec2(80.0, 40.0))
}

/// Un frame con la misma disposición que las filas de capas: primero el fondo, después el botón.
fn frame_de_fila(ctx: &egui::Context, tiempo: f64, eventos: Vec<Event>) -> MedicionDeFila {
    let mut m = MedicionDeFila::default();
    let entrada = RawInput {
        time: Some(tiempo),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0))),
        events: eventos,
        ..Default::default()
    };
    let mut salida = ctx.run_ui(entrada, |ui| {
        // El fondo se crea primero: queda "debajo" de lo que se cree después.
        let fondo = ui.interact(fila(), egui::Id::new("fondo"), Sense::click_and_drag());
        let boton = ui.put(rect_del_boton(), egui::Button::new("Parar"));
        m.fondo_clic = fondo.clicked();
        m.fondo_arrastre_empezo = fondo.drag_started();
        m.fondo_arrastrando = fondo.dragged();
        m.boton_clic = boton.clicked();
    });
    salida.textures_delta.clear();
    m
}

fn contexto_de_fila(pos: Pos2) -> (egui::Context, f64) {
    let ctx = egui::Context::default();
    frame_de_fila(&ctx, 0.0, vec![]);
    frame_de_fila(&ctx, 0.02, vec![mover(pos)]);
    (ctx, 0.04)
}

#[test]
fn un_clic_en_espacio_vacio_de_la_fila_lo_recibe_el_fondo() {
    let vacio = Pos2::new(250.0, 130.0); // dentro de la fila, lejos del botón
    let (ctx, t) = contexto_de_fila(vacio);
    frame_de_fila(&ctx, t, vec![boton(vacio, PointerButton::Primary, true)]);
    let m = frame_de_fila(
        &ctx,
        t + 0.05,
        vec![boton(vacio, PointerButton::Primary, false)],
    );
    assert!(m.fondo_clic, "el fondo no recibió el clic: {m:?}");
    assert!(!m.boton_clic, "{m:?}");
}

#[test]
fn un_clic_sobre_el_boton_lo_recibe_el_boton_y_no_el_fondo() {
    let sobre_el_boton = Pos2::new(60.0, 130.0);
    let (ctx, t) = contexto_de_fila(sobre_el_boton);
    frame_de_fila(
        &ctx,
        t,
        vec![boton(sobre_el_boton, PointerButton::Primary, true)],
    );
    let m = frame_de_fila(
        &ctx,
        t + 0.05,
        vec![boton(sobre_el_boton, PointerButton::Primary, false)],
    );
    assert!(m.boton_clic, "el botón no recibió el clic: {m:?}");
    assert!(!m.fondo_clic, "el fondo le robó el clic al botón: {m:?}");
}

#[test]
fn un_arrastre_que_empieza_en_espacio_vacio_lo_recibe_el_fondo() {
    let vacio = Pos2::new(250.0, 130.0);
    let (ctx, t) = contexto_de_fila(vacio);
    frame_de_fila(&ctx, t, vec![boton(vacio, PointerButton::Primary, true)]);
    // Se mueve el puntero de a poco, más allá del umbral de arrastre.
    let mut empezo = false;
    let mut arrastrando = false;
    for paso in 1..=8 {
        let pos = Pos2::new(250.0, 130.0 + 10.0 * paso as f32);
        let m = frame_de_fila(&ctx, t + 0.02 * paso as f64, vec![mover(pos)]);
        empezo |= m.fondo_arrastre_empezo;
        arrastrando |= m.fondo_arrastrando;
    }
    assert!(empezo, "el fondo no vio empezar el arrastre");
    assert!(arrastrando, "el fondo no vio el arrastre");
}

#[test]
fn un_arrastre_que_empieza_sobre_un_boton_lo_toma_el_fondo_de_la_fila() {
    // Un botón no sabe arrastrar, así que egui le pasa el arrastre al fondo, que está
    // debajo y sí sabe. Por eso se puede reordenar una capa agarrándola desde cualquier
    // parte de su fila, incluso sobre un botón; un clic simple, en cambio, sigue yendo al botón.
    let sobre_el_boton = Pos2::new(60.0, 130.0);
    let (ctx, t) = contexto_de_fila(sobre_el_boton);
    frame_de_fila(
        &ctx,
        t,
        vec![boton(sobre_el_boton, PointerButton::Primary, true)],
    );
    let mut empezo = false;
    let mut hubo_clic_del_boton = false;
    for paso in 1..=8 {
        let pos = Pos2::new(60.0, 130.0 + 10.0 * paso as f32);
        let m = frame_de_fila(&ctx, t + 0.02 * paso as f64, vec![mover(pos)]);
        empezo |= m.fondo_arrastre_empezo;
        hubo_clic_del_boton |= m.boton_clic;
    }
    assert!(
        empezo,
        "el fondo no tomó el arrastre que empezó sobre el botón"
    );
    assert!(!hubo_clic_del_boton);
}

#[test]
fn soltar_un_arrastre_que_empezo_sobre_un_boton_no_cuenta_como_clic_del_boton() {
    // Si no, reordenar una capa disparaba también el botón "Parar" de esa fila.
    let sobre_el_boton = Pos2::new(60.0, 130.0);
    let (ctx, t) = contexto_de_fila(sobre_el_boton);
    frame_de_fila(
        &ctx,
        t,
        vec![boton(sobre_el_boton, PointerButton::Primary, true)],
    );
    for paso in 1..=8 {
        let pos = Pos2::new(60.0, 130.0 + 10.0 * paso as f32);
        frame_de_fila(&ctx, t + 0.02 * paso as f64, vec![mover(pos)]);
    }
    let fin = Pos2::new(60.0, 210.0);
    let m = frame_de_fila(
        &ctx,
        t + 0.3,
        vec![boton(fin, PointerButton::Primary, false)],
    );
    assert!(!m.boton_clic, "soltar el arrastre disparó el botón: {m:?}");
}

// ---------------------------------------------------------------------------------------
// Celda de clip: un botón que además siente el arrastre (`interact(click_and_drag)`)
// ---------------------------------------------------------------------------------------

/// Lo que se midió con una celda arrastrable dentro de una fila con fondo.
#[derive(Default, Debug)]
struct MedicionDeCelda {
    fondo_arrastre_empezo: bool,
    celda_arrastre_empezo: bool,
    celda_clic: bool,
}

fn frame_de_celda(ctx: &egui::Context, tiempo: f64, eventos: Vec<Event>) -> MedicionDeCelda {
    let mut m = MedicionDeCelda::default();
    let entrada = RawInput {
        time: Some(tiempo),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0))),
        events: eventos,
        ..Default::default()
    };
    let mut salida = ctx.run_ui(entrada, |ui| {
        let fondo = ui.interact(fila(), egui::Id::new("fondo"), Sense::click_and_drag());
        let celda = ui
            .put(rect_del_boton(), egui::Button::new("clip"))
            .interact(Sense::click_and_drag());
        m.fondo_arrastre_empezo = fondo.drag_started();
        m.celda_arrastre_empezo = celda.drag_started();
        m.celda_clic = celda.clicked();
    });
    salida.textures_delta.clear();
    m
}

#[test]
fn un_arrastre_que_empieza_sobre_una_celda_de_clip_lo_recibe_la_celda_y_no_el_fondo() {
    // Si lo recibiera el fondo, arrastrar un clip reordenaría la capa entera.
    let sobre_la_celda = Pos2::new(60.0, 130.0);
    let ctx = egui::Context::default();
    frame_de_celda(&ctx, 0.0, vec![]);
    frame_de_celda(&ctx, 0.02, vec![mover(sobre_la_celda)]);
    frame_de_celda(
        &ctx,
        0.04,
        vec![boton(sobre_la_celda, PointerButton::Primary, true)],
    );
    let (mut celda, mut fondo) = (false, false);
    for paso in 1..=8 {
        let pos = Pos2::new(60.0, 130.0 + 10.0 * paso as f32);
        let m = frame_de_celda(&ctx, 0.04 + 0.02 * paso as f64, vec![mover(pos)]);
        celda |= m.celda_arrastre_empezo;
        fondo |= m.fondo_arrastre_empezo;
    }
    assert!(celda, "la celda no vio empezar el arrastre");
    assert!(!fondo, "el fondo le robó el arrastre a la celda");
}

#[test]
fn un_clic_sobre_una_celda_de_clip_sigue_siendo_un_clic() {
    // Que la celda sienta el arrastre no le quita el clic (disparar el clip).
    let sobre_la_celda = Pos2::new(60.0, 130.0);
    let ctx = egui::Context::default();
    frame_de_celda(&ctx, 0.0, vec![]);
    frame_de_celda(&ctx, 0.02, vec![mover(sobre_la_celda)]);
    frame_de_celda(
        &ctx,
        0.04,
        vec![boton(sobre_la_celda, PointerButton::Primary, true)],
    );
    let m = frame_de_celda(
        &ctx,
        0.09,
        vec![boton(sobre_la_celda, PointerButton::Primary, false)],
    );
    assert!(m.celda_clic, "{m:?}");
}
