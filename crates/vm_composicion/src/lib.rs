//! Composición: la imagen "maestra" donde se mezcla todo el contenido.
//!
//! Una cámara dibuja las capas en una textura de resolución fija (elegida por proyecto),
//! independiente de los monitores. Las salidas no dibujan el contenido directamente:
//! toman porciones de esta textura y las deforman.
//!
//! Cada **capa** del proyecto es una entidad con un `Reproductor` (que lleva los frames de
//! su video a una textura) y un sprite que la muestra dentro de la composición. Las
//! órdenes de disparo (`OrdenClip`) deciden qué clip suena en cada capa.

mod material_capa;
mod miniaturas;

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use vm_core::{
    CambioCapa, EstadoReproduccion, Fuente, ModoMezcla, OrdenComposicion, OrigenSuperficie,
    avance_de_transicion, nueva_posicion, opacidades_de_transicion, ordenes_iniciales,
    tamano_de_ajuste,
};
use vm_core::{Composicion, resolver_ruta};
use vm_proyecto::{ProyectoActual, ProyectoCargado, RutaProyecto};
use vm_video::Reproductor;
use vm_video::decodificador::{Ajustes, Origen};

pub use miniaturas::{EstadoMiniatura, Miniaturas};

use material_capa::{MallaDeCapa, MaterialCapa, MaterialCapaPlugin, ParametrosCapa};

/// Capa de render exclusiva de la composición. Las salidas usan 1, 2, 3...,
/// y la 0 es la de por defecto; esta queda lejos para no pisarse nunca.
pub const CAPA_COMPOSICION: usize = 100;

pub struct ComposicionPlugin;

impl Plugin for ComposicionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialCapaPlugin)
            .add_message::<OrdenClip>()
            .add_message::<CapaQuitada>()
            .add_message::<CapaMovida>()
            .init_resource::<Reproduccion>()
            .init_resource::<TexturasDeCapas>()
            .init_resource::<ProgresoDeCapas>()
            .init_resource::<Miniaturas>()
            .init_resource::<MensajeComposicion>()
            .init_resource::<ErroresDeCapas>()
            .init_resource::<SenalesDeEntrada>()
            // `PostStartup` corre después de `Startup`, cuando el proyecto ya se cargó.
            .add_systems(PostStartup, crear_composicion)
            .add_systems(Update, miniaturas::gestionar_miniaturas)
            .add_systems(
                Update,
                (
                    reiniciar_al_cargar,
                    sincronizar_resolucion,
                    reindexar_capas_quitadas,
                    reindexar_capas_movidas,
                    sincronizar_capas,
                    ejecutar_ordenes,
                    sincronizar_ajustes_de_clips,
                    guardar_tokens_de_captura,
                    reportar_errores_de_video,
                    avanzar_transiciones,
                    sincronizar_capas_visuales,
                    sincronizar_salientes,
                    publicar_texturas_de_capas,
                    publicar_progreso,
                )
                    .chain(),
            );
    }
}

/// La textura donde se dibuja la composición. Las superficies de las salidas la leen.
/// **Si cambia la resolución, el handle cambia**: quien la use debe mirar `is_changed`.
#[derive(Resource)]
pub struct TexturaComposicion {
    pub imagen: Handle<Image>,
    pub ancho: u32,
    pub alto: u32,
}

/// Una textura con su tamaño en píxeles.
#[derive(Clone, PartialEq)]
pub struct TexturaFuente {
    pub imagen: Handle<Image>,
    pub ancho: u32,
    pub alto: u32,
}

/// La textura de cada capa: el último frame del clip que suena en ella, **sin mezclar** con
/// las demás capas. Una superficie con origen `Capa(n)` lee de acá. Los handles cambian si
/// el video cambia de resolución, así que hay que volver a leerlos cada frame.
///
/// El índice del `Vec` es el de la capa (la 0 es la de más arriba).
#[derive(Resource, Default)]
pub struct TexturasDeCapas(pub Vec<TexturaFuente>);

/// Qué textura debe leer una superficie según su origen.
///
/// Si pide una capa que no existe, se usa la composición (mejor mostrar algo que dejar la
/// superficie vacía por una referencia colgada).
pub fn textura_de_origen(
    origen: OrigenSuperficie,
    composicion: &TexturaComposicion,
    capas: &TexturasDeCapas,
) -> TexturaFuente {
    if let OrigenSuperficie::Capa(i) = origen
        && let Some(capa) = capas.0.get(i)
    {
        return capa.clone();
    }
    TexturaFuente {
        imagen: composicion.imagen.clone(),
        ancho: composicion.ancho,
        alto: composicion.alto,
    }
}

/// Las señales que mueven parámetros: una por cada fuente de sonido que se está escuchando
/// (la mide `vm_audio`; más adelante también lo que llegue por OSC o MIDI). Valen de 0 a 1 y
/// las escribe quien las mide: acá solo se leen para aplicar las modulaciones de las capas
/// (`vm_core::Modulacion`, que dice cuál fuente le toca a cada una).
#[derive(Resource, Default, Clone, PartialEq)]
pub struct SenalesDeEntrada {
    por_fuente: std::collections::HashMap<vm_core::FuenteDeAudio, vm_core::Senales>,
}

impl SenalesDeEntrada {
    /// Las señales de una fuente. Una que no se está escuchando suena a silencio.
    pub fn de(&self, fuente: &vm_core::FuenteDeAudio) -> vm_core::Senales {
        self.por_fuente.get(fuente).copied().unwrap_or_default()
    }

    /// Anota las señales de una fuente. Devuelve `true` si algo cambió.
    pub fn poner(&mut self, fuente: vm_core::FuenteDeAudio, senales: vm_core::Senales) -> bool {
        self.por_fuente.insert(fuente, senales) != Some(senales)
    }

    /// ¿Hay señales de fuentes que ya no se piden?
    pub fn tiene_otras_que(&self, fuentes: &[vm_core::FuenteDeAudio]) -> bool {
        self.por_fuente.keys().any(|f| !fuentes.contains(f))
    }

    /// Deja solo las fuentes que se piden (las demás ya no se escuchan).
    pub fn conservar(&mut self, fuentes: &[vm_core::FuenteDeAudio]) -> bool {
        let antes = self.por_fuente.len();
        self.por_fuente.retain(|f, _| fuentes.contains(f));
        self.por_fuente.len() != antes
    }
}

/// La opacidad de una capa **ahora**: la que se eligió, o esa más lo que le suma su
/// modulación con la señal de su fuente en este momento.
fn opacidad_efectiva(capa: &vm_core::Capa, senales: &SenalesDeEntrada) -> f32 {
    match &capa.modulacion_de_opacidad {
        Some(m) => vm_core::aplicar_modulacion(capa.opacidad, m, &senales.de(&m.fuente)),
        None => capa.opacidad,
    }
}

/// `HYPNOTICA_DIAGNOSTICO_MODULACION=1` registra cada segundo la opacidad de las capas que se
/// mueven con una señal (para comprobar que el sonido llega sin mirar la pantalla).
fn diagnostico_de_modulacion() -> bool {
    use std::sync::OnceLock;
    static VALOR: OnceLock<bool> = OnceLock::new();
    *VALOR.get_or_init(|| std::env::var_os("HYPNOTICA_DIAGNOSTICO_MODULACION").is_some())
}

/// Qué clip falló en cada capa y por qué: `(columna, motivo)`. La interfaz marca esa celda. Se
/// borra cuando se vuelve a disparar la capa o se la detiene.
#[derive(Resource, Default)]
pub struct ErroresDeCapas(pub Vec<Option<(usize, String)>>);

impl ErroresDeCapas {
    fn marcar(&mut self, capa: usize, columna: usize, motivo: String) {
        if self.0.len() <= capa {
            self.0.resize(capa + 1, None);
        }
        self.0[capa] = Some((columna, motivo));
    }

    fn limpiar(&mut self, capa: usize) {
        if let Some(e) = self.0.get_mut(capa) {
            *e = None;
        }
    }
}

/// Una orden de disparo. Es el mensaje de Bevy que envuelve a `OrdenComposicion`
/// (el tipo puro de `vm_core`); la interfaz, el teclado y MIDI/OSC envían esto.
#[derive(Message, Clone, Copy, Debug)]
pub struct OrdenClip(pub OrdenComposicion);

/// Aviso de que se quitó la capa con ese índice del proyecto.
///
/// Lo envía quien la quita (la interfaz), que también corrige el proyecto y el estado de
/// reproducción. Acá se acomodan las entidades: la de la capa quitada se borra, y las de
/// las capas siguientes **conservan su video** y solo bajan un lugar. Si en cambio se
/// borrara y se rehiciera todo, los clips que están sonando se reabrirían.
#[derive(Message, Clone, Copy, Debug)]
pub struct CapaQuitada(pub usize);

/// Aviso de que se movió la capa `desde` a la posición `hasta` (la 0 es la de más arriba).
///
/// Igual que `CapaQuitada`: quien la mueve (la interfaz) corrige el proyecto y el estado de
/// reproducción; acá se renumeran las entidades, que **conservan su video**: la capa que
/// suena sigue sonando, solo cambia el lugar que ocupa.
#[derive(Message, Clone, Copy, Debug)]
pub struct CapaMovida {
    pub desde: usize,
    pub hasta: usize,
}

/// Qué clip suena en cada capa. Es estado de ejecución: no se guarda en el proyecto.
#[derive(Resource, Default)]
pub struct Reproduccion(pub EstadoReproduccion);

/// Último aviso o error de la composición (por ejemplo, un video que no se pudo abrir),
/// para mostrarlo en la interfaz.
#[derive(Resource, Default)]
pub struct MensajeComposicion {
    pub texto: String,
}

/// Marca la cámara que dibuja la composición.
#[derive(Component)]
struct CamaraComposicion;

/// Una capa de la composición. `indice` 0 es la de más arriba.
#[derive(Component)]
pub struct CapaComposicion {
    pub indice: usize,
}

/// Una imagen pensada para que una cámara dibuje sobre ella (y luego se pueda leer como textura).
fn imagen_destino(imagenes: &mut Assets<Image>, ancho: u32, alto: u32) -> Handle<Image> {
    imagenes.add(Image::new_target_texture(
        ancho,
        alto,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ))
}

fn crear_composicion(
    mut commands: Commands,
    mut imagenes: ResMut<Assets<Image>>,
    proyecto: Res<ProyectoActual>,
) {
    let Composicion { ancho, alto, .. } = proyecto.0.composicion;
    let imagen = imagen_destino(&mut imagenes, ancho, alto);
    commands.insert_resource(TexturaComposicion {
        imagen: imagen.clone(),
        ancho,
        alto,
    });

    commands.spawn((
        CamaraComposicion,
        Camera2d,
        // Las cámaras se dibujan en orden creciente de `order`. Esta va primero (-1)
        // para que la textura esté lista cuando las salidas (orden 0) la lean.
        Camera {
            order: -1,
            ..default()
        },
        RenderTarget::Image(imagen.into()),
        // `layer(n)` solo llega hasta la capa 63; `from_layers` admite cualquier número.
        RenderLayers::from_layers(&[CAPA_COMPOSICION]),
    ));
}

/// Si el proyecto cambió la resolución de la composición, se crea una textura nueva y se
/// apunta la cámara a ella.
fn sincronizar_resolucion(
    proyecto: Res<ProyectoActual>,
    mut textura: ResMut<TexturaComposicion>,
    mut imagenes: ResMut<Assets<Image>>,
    mut camaras: Query<&mut RenderTarget, With<CamaraComposicion>>,
) {
    let comp = &proyecto.0.composicion;
    if textura.ancho == comp.ancho && textura.alto == comp.alto {
        return;
    }
    let nueva = imagen_destino(&mut imagenes, comp.ancho, comp.alto);
    textura.imagen = nueva.clone();
    textura.ancho = comp.ancho;
    textura.alto = comp.alto;
    for mut destino in &mut camaras {
        *destino = RenderTarget::Image(nueva.clone().into());
    }
}

/// Acomoda las entidades de las capas cuando se quitó una (ver `CapaQuitada`). Tiene que
/// correr **antes** de `sincronizar_capas`, que si no vería capas de más y borraría la
/// equivocada.
fn reindexar_capas_quitadas(
    mut commands: Commands,
    mut avisos: MessageReader<CapaQuitada>,
    mut capas: Query<(Entity, &mut CapaComposicion)>,
) {
    for aviso in avisos.read() {
        for (entidad, mut capa) in &mut capas {
            if capa.indice == aviso.0 {
                // Se le da un índice imposible para que ni `sincronizar_capas` la cuente
                // mientras el borrado (que se aplica al final del frame) no se haya hecho.
                capa.indice = usize::MAX;
                commands.entity(entidad).despawn();
            } else if capa.indice != usize::MAX && capa.indice > aviso.0 {
                capa.indice -= 1;
            }
        }
    }
}

/// Renumera las entidades de las capas cuando se movió una (ver `CapaMovida`). Tiene que
/// correr antes de `sincronizar_capas` y de `sincronizar_capas_visuales`, que dependen del
/// índice de cada entidad.
fn reindexar_capas_movidas(
    mut avisos: MessageReader<CapaMovida>,
    mut capas: Query<&mut CapaComposicion>,
) {
    for aviso in avisos.read() {
        for mut capa in &mut capas {
            if capa.indice == usize::MAX {
                continue; // una capa que se está borrando
            }
            let nuevo = nueva_posicion(capa.indice, aviso.desde, aviso.hasta);
            if nuevo != capa.indice {
                capa.indice = nuevo;
            }
        }
    }
}

/// Mantiene una entidad por cada capa del proyecto.
fn sincronizar_capas(
    mut commands: Commands,
    proyecto: Res<ProyectoActual>,
    capas: Query<(Entity, &CapaComposicion)>,
    mut imagenes: ResMut<Assets<Image>>,
    mut materiales: ResMut<Assets<MaterialCapa>>,
    malla: Res<MallaDeCapa>,
) {
    let necesarias = proyecto.0.composicion.capas.len();
    for (entidad, capa) in &capas {
        if capa.indice >= necesarias {
            commands.entity(entidad).despawn();
        }
    }
    for indice in 0..necesarias {
        if !capas.iter().any(|(_, c)| c.indice == indice) {
            let reproductor = Reproductor::nuevo(&mut imagenes);
            let material = materiales.add(MaterialCapa {
                parametros: ParametrosCapa::nuevos(1.0, ModoMezcla::Normal, false),
                textura: reproductor.textura.clone(),
            });
            // Cada capa es un rectángulo de 1×1 con su material; `sincronizar_capas_visuales`
            // le pone el tamaño, la profundidad y los parámetros.
            commands.spawn((
                CapaComposicion { indice },
                reproductor,
                Mesh2d(malla.0.clone()),
                MeshMaterial2d(material),
                Transform::default(),
                RenderLayers::from_layers(&[CAPA_COMPOSICION]),
            ));
        }
    }
}

/// Al cargar un proyecto, el estado de reproducción anterior ya no vale: se detiene todo
/// y se dispara el primer clip de cada capa.
fn reiniciar_al_cargar(
    mut cargas: MessageReader<ProyectoCargado>,
    proyecto: Res<ProyectoActual>,
    mut estado: ResMut<Reproduccion>,
    mut ordenes: MessageWriter<OrdenClip>,
) {
    if cargas.read().count() == 0 {
        return;
    }
    estado.0 = EstadoReproduccion::default();
    for orden in ordenes_iniciales(&proyecto.0.composicion) {
        ordenes.write(OrdenClip(orden));
    }
}

/// Convierte las órdenes en cambios de reproducción y los aplica a los reproductores.
// Un sistema de Bevy pide cada recurso que usa como parámetro: la cantidad es la de recursos.
#[allow(clippy::too_many_arguments)]
fn ejecutar_ordenes(
    mut ordenes: MessageReader<OrdenClip>,
    proyecto: Res<ProyectoActual>,
    ruta: Res<RutaProyecto>,
    mut estado: ResMut<Reproduccion>,
    mut mensaje: ResMut<MensajeComposicion>,
    mut capas: Query<(&CapaComposicion, &mut Reproductor)>,
    // Cambios que todavía no se pudieron aplicar porque la entidad de su capa recién
    // se está creando (los comandos se aplican al final del frame).
    mut pendientes: Local<Vec<CambioCapa>>,
    // La captura que hay que abrir mostrando el selector del escritorio (ver
    // `OrdenComposicion::ReelegirCaptura`).
    mut reelegir: Local<Option<(usize, usize)>>,
    mut errores: ResMut<ErroresDeCapas>,
) {
    let comp = &proyecto.0.composicion;
    let mut cambios: Vec<CambioCapa> = std::mem::take(&mut *pendientes);

    for orden in ordenes.read() {
        if let OrdenComposicion::ReelegirCaptura { capa, columna } = orden.0 {
            *reelegir = Some((capa, columna));
        }
        cambios.extend(estado.0.aplicar(orden.0, comp));
    }
    // Si se editó la composición (por ejemplo, se borró el clip que sonaba), se reconcilia.
    if proyecto.is_changed() {
        cambios.extend(estado.0.ajustar_a(comp));
    }

    let carpeta = ruta.carpeta();
    for cambio in cambios {
        let capa = match cambio {
            CambioCapa::Reproducir { capa, .. }
            | CambioCapa::Detener { capa }
            | CambioCapa::Pausar { capa, .. }
            | CambioCapa::Buscar { capa, .. } => capa,
        };
        let Some((_, mut reproductor)) = capas.iter_mut().find(|(c, _)| c.indice == capa) else {
            pendientes.push(cambio);
            continue;
        };
        // Volver a disparar o detener una capa borra la marca de su último fallo.
        if let CambioCapa::Reproducir { capa, .. } | CambioCapa::Detener { capa } = cambio {
            errores.limpiar(capa);
        }
        match cambio {
            CambioCapa::Detener { .. } => reproductor.detener(),
            CambioCapa::Pausar { en_pausa, .. } => reproductor.pausar(en_pausa),
            CambioCapa::Buscar { milesimas, .. } => reproductor.buscar(milesimas as f32 / 1000.0),
            CambioCapa::Reproducir { capa, columna } => {
                let Some(clip) = comp.clip(capa, columna) else {
                    continue;
                };
                // Una captura de pantalla no se abre acá: hay que esperar al portal (y al
                // usuario, que puede estar eligiendo qué compartir).
                if let Fuente::Captura {
                    token,
                    ocultar_cursor,
                    ..
                } = &clip.fuente
                {
                    // Al elegir otra captura se pide SIN token para que aparezca el selector,
                    // pero el token guardado en el clip no se toca: solo se reemplaza cuando el
                    // portal devuelve uno nuevo. Si se cancela el selector, el clip sigue igual.
                    let token = if *reelegir == Some((capa, columna)) {
                        *reelegir = None;
                        None
                    } else {
                        token.clone()
                    };
                    reproductor.abrir_captura(
                        token,
                        ajustes_de(clip),
                        Some((comp.ancho, comp.alto)),
                        // El sonido de la captura está retirado por ahora (ver docs/05): se
                        // captura solo la imagen, aunque el clip tenga un `audio` guardado.
                        None,
                        *ocultar_cursor,
                    );
                    mensaje.texto.clear();
                    continue;
                }
                // De dónde sale la imagen: un archivo, una cámara o una transmisión de red.
                let origen = match &clip.fuente {
                    // Una imagen se abre como un archivo más: el bucle apagado (ver `ajustes_de`)
                    // hace que se decodifique una vez y quede quieta.
                    Fuente::Video { ruta: guardada } | Fuente::Imagen { ruta: guardada } => {
                        Origen::Archivo(resolver_ruta(&carpeta, guardada))
                    }
                    Fuente::Camara { dispositivo } => Origen::Camara(dispositivo.clone()),
                    Fuente::Url { url } => Origen::Url(url.clone()),
                    Fuente::Captura { .. } => continue, // ya se atendió arriba
                };
                // Los cuadros no pasan del tamaño de la composición: más resolución se perdería
                // igual al mezclar, y cuesta CPU y ancho de banda de la GPU.
                let limite = Some((comp.ancho, comp.alto));
                match reproductor.abrir_origen(&origen, ajustes_de(clip), limite) {
                    Ok(()) => mensaje.texto.clear(),
                    Err(e) => {
                        mensaje.texto =
                            format!("No se pudo abrir {}: {e}", clip.fuente.descripcion());
                        warn!("{}", mensaje.texto);
                    }
                }
            }
        }
    }
}

/// Los ajustes de reproducción de un clip, en el formato que entiende el decodificador.
fn ajustes_de(clip: &vm_core::ConfigClip) -> Ajustes {
    let a = clip.ajustes;
    Ajustes {
        // Una imagen NUNCA hace bucle: termina al instante, y con el bucle volvería a
        // decodificarse sin parar (unas 190 veces por segundo) para mostrar algo quieto.
        bucle: a.bucle && !clip.fuente.es_imagen(),
        velocidad: a.velocidad,
        inicio: a.inicio,
        fin: a.fin,
        volumen: a.volumen,
        silenciado: a.silenciado,
    }
}

/// Segundos que tienen que pasar sin cambios en el proyecto para aplicar un cambio de
/// velocidad o de tramo a un video que suena: así, mientras se arrastra un deslizador, no se
/// hace un salto de video por cada frame.
const PAUSA_PARA_SALTAR: f64 = 0.25;

/// Mantiene los ajustes de cada video que suena al día con los de su clip en el proyecto.
/// El volumen, el silencio y el bucle se aplican al instante; la velocidad y el tramo
/// (que hacen saltar el video) esperan a que se deje de tocar el deslizador.
fn sincronizar_ajustes_de_clips(
    proyecto: Res<ProyectoActual>,
    estado: Res<Reproduccion>,
    tiempo: Res<Time>,
    mut capas: Query<(&CapaComposicion, &mut Reproductor)>,
    mut ultimo_cambio: Local<f64>,
) {
    let ahora = tiempo.elapsed_secs_f64();
    if proyecto.is_changed() {
        *ultimo_cambio = ahora;
    }
    let estable = ahora - *ultimo_cambio >= PAUSA_PARA_SALTAR;
    for (capa, mut reproductor) in &mut capas {
        let Some(columna) = estado.0.activo.get(capa.indice).copied().flatten() else {
            continue;
        };
        let Some(clip) = proyecto.0.composicion.clip(capa.indice, columna) else {
            continue;
        };
        let deseados = ajustes_de(clip);
        let Some(puestos) = reproductor.ajustes() else {
            continue;
        };
        if deseados == puestos {
            continue;
        }
        let cambia_el_salto = (deseados.velocidad, deseados.inicio, deseados.fin)
            != (puestos.velocidad, puestos.inicio, puestos.fin);
        if !cambia_el_salto || estable {
            reproductor.ajustar(deseados);
        } else {
            // Mientras se arrastra: lo que no hace saltar (volumen, silencio, bucle) ya va.
            reproductor.ajustar(Ajustes {
                velocidad: puestos.velocidad,
                inicio: puestos.inicio,
                fin: puestos.fin,
                ..deseados
            });
        }
    }
}

/// Guarda en el proyecto el token de restauración que el portal da al abrir una captura: con
/// él, la próxima vez la misma pantalla o ventana se reabre sin preguntar. Cada sesión puede
/// devolver un token nuevo, así que se actualiza el del clip que suena en esa capa.
fn guardar_tokens_de_captura(
    mut proyecto: ResMut<ProyectoActual>,
    estado: Res<Reproduccion>,
    mut capas: Query<(&CapaComposicion, &mut Reproductor)>,
) {
    for (capa, mut reproductor) in &mut capas {
        // Se mira primero sin pedir acceso de escritura, para no marcar nada como modificado.
        if !reproductor.hay_token_nuevo() {
            continue;
        }
        let Some(token) = reproductor.tomar_token_nuevo() else {
            continue;
        };
        let Some(columna) = estado.0.activo.get(capa.indice).copied().flatten() else {
            continue;
        };
        let es_distinto = matches!(
            proyecto.0.composicion.clip(capa.indice, columna).map(|c| &c.fuente),
            Some(Fuente::Captura { token: actual, .. }) if actual.as_deref() != Some(token.as_str())
        );
        if es_distinto
            && let Some(Some(clip)) = proyecto
                .0
                .composicion
                .capas
                .get_mut(capa.indice)
                .and_then(|c| c.celdas.get_mut(columna))
        {
            // Solo cambia el token: el audio elegido se conserva.
            if let Fuente::Captura { token: actual, .. } = &mut clip.fuente {
                *actual = Some(token);
            }
        }
    }
}

/// Dónde va el video de cada capa y cuánto dura, en segundos (`None` si no suena nada o
/// todavía no se sabe). Lo muestra la interfaz en la barra de progreso del clip.
#[derive(Resource, Default)]
pub struct ProgresoDeCapas(pub Vec<Option<(f64, f64)>>);

/// Deja en `ProgresoDeCapas` la posición de cada capa.
fn publicar_progreso(
    capas: Query<(&CapaComposicion, &Reproductor)>,
    mut publicado: ResMut<ProgresoDeCapas>,
) {
    let total = capas.iter().count();
    let mut actual = vec![None; total];
    for (capa, reproductor) in &capas {
        if let Some(sitio) = actual.get_mut(capa.indice) {
            *sitio = reproductor.posicion();
        }
    }
    if publicado.0 != actual {
        publicado.0 = actual;
    }
}

/// Pasa a la interfaz los errores de los videos que fallaron después de abrirse
/// (un archivo dañado, uno sin pista de video...).
fn reportar_errores_de_video(
    mut capas: Query<(&CapaComposicion, &mut Reproductor)>,
    mut mensaje: ResMut<MensajeComposicion>,
    mut estado: ResMut<Reproduccion>,
    mut errores: ResMut<ErroresDeCapas>,
) {
    for (capa, mut reproductor) in &mut capas {
        // Un aviso no detiene nada (lo que ya se mostraba sigue): solo se informa.
        if reproductor.hay_aviso()
            && let Some(aviso) = reproductor.tomar_aviso()
        {
            mensaje.texto = format!(
                "La capa {} sigue con su captura anterior: {aviso}",
                capa.indice + 1
            );
            warn!("{}", mensaje.texto);
        }
        // Se pregunta primero sin escribir, para no marcar el componente como modificado.
        if !reproductor.hay_error() {
            continue;
        }
        if let Some(motivo) = reproductor.tomar_error() {
            // El clip que falló ya no suena: la capa queda como detenida, y se anota cuál
            // fue para marcarlo en la grilla.
            if let Some(activo) = estado.0.activo.get_mut(capa.indice) {
                if let Some(columna) = *activo {
                    errores.marcar(capa.indice, columna, motivo.clone());
                }
                *activo = None;
            }
            if let Some(pausada) = estado.0.pausada.get_mut(capa.indice) {
                *pausada = false;
            }
            mensaje.texto = format!(
                "La capa {} no pudo reproducir el clip: {motivo}",
                capa.indice + 1
            );
            warn!("{}", mensaje.texto);
        }
    }
}

/// Deja en `TexturasDeCapas` la textura (y el tamaño) actual de cada capa.
fn publicar_texturas_de_capas(
    capas: Query<(&CapaComposicion, &Reproductor)>,
    imagenes: Res<Assets<Image>>,
    mut publicadas: ResMut<TexturasDeCapas>,
) {
    let mut actuales: Vec<(usize, TexturaFuente)> = capas
        .iter()
        .map(|(capa, reproductor)| {
            let tamano = imagenes
                .get(&reproductor.textura)
                .map(|i| (i.width(), i.height()))
                .unwrap_or((1, 1));
            (
                capa.indice,
                TexturaFuente {
                    imagen: reproductor.textura.clone(),
                    ancho: tamano.0,
                    alto: tamano.1,
                },
            )
        })
        .collect();
    actuales.sort_by_key(|(indice, _)| *indice);
    let actuales: Vec<TexturaFuente> = actuales.into_iter().map(|(_, t)| t).collect();

    // Se escribe solo si algo cambió, para no marcar el recurso como modificado en cada frame.
    if publicadas.0 != actuales {
        publicadas.0 = actuales;
    }
}

/// Lo que lee y escribe `sincronizar_capas_visuales` de cada capa.
type DatosVisualesDeCapa = (
    Entity,
    &'static CapaComposicion,
    &'static Reproductor,
    &'static MeshMaterial2d<MaterialCapa>,
    &'static mut Transform,
    &'static mut Visibility,
);

/// Mantiene cada capa al día con su configuración en el proyecto y con su video:
/// la textura (cuyo handle puede cambiar), la opacidad, el modo de mezcla, la visibilidad,
/// el tamaño según el ajuste, y la profundidad (la capa 0 arriba de todo).
///
/// Se compara antes de escribir para no marcar nada como modificado sin necesidad.
fn sincronizar_capas_visuales(
    mut commands: Commands,
    malla: Res<MallaDeCapa>,
    proyecto: Res<ProyectoActual>,
    composicion: Res<TexturaComposicion>,
    publicadas: Res<TexturasDeCapas>,
    senales: Res<SenalesDeEntrada>,
    mut materiales: ResMut<Assets<MaterialCapa>>,
    mut capas: Query<DatosVisualesDeCapa>,
    mut ultimo_registro: Local<Option<std::time::Instant>>,
) {
    let total = capas.iter().count();
    let registrar = diagnostico_de_modulacion()
        && ultimo_registro.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
    if registrar {
        *ultimo_registro = Some(std::time::Instant::now());
    }
    let tamano_composicion = (composicion.ancho as f32, composicion.alto as f32);
    for (entidad, capa, reproductor, material, mut transform, mut visibilidad) in &mut capas {
        let Some(config) = proyecto.0.composicion.capas.get(capa.indice) else {
            continue;
        };

        // El material: textura y parámetros.
        // Durante un fundido el clip nuevo sube con el avance (sin fundido, `avance` es 1 y
        // queda la opacidad de la capa tal cual).
        let opacidad = opacidad_efectiva(config, &senales);
        if registrar && config.modulacion_de_opacidad.is_some() {
            info!(
                "modulación: capa {} con opacidad {opacidad:.2} (base {:.2})",
                capa.indice + 1,
                config.opacidad
            );
        }
        let (_, entrante) =
            opacidades_de_transicion(opacidad, reproductor.avance_de_entrada(), config.modo);
        let parametros = ParametrosCapa::nuevos(entrante, config.modo, reproductor.ignora_alfa());
        let cambio = materiales
            .get(&material.0)
            .is_some_and(|m| m.textura != reproductor.textura || m.parametros != parametros);
        if cambio && let Some(mut m) = materiales.get_mut(&material.0) {
            m.textura = reproductor.textura.clone();
            m.parametros = parametros;
        }

        // El tamaño: depende del ajuste y del tamaño del video.
        let video = publicadas
            .0
            .get(capa.indice)
            .map_or(tamano_composicion, |t| (t.ancho as f32, t.alto as f32));
        let (ancho, alto) = tamano_de_ajuste(config.ajuste, tamano_composicion, video);
        let escala = Vec3::new(ancho, alto, 1.0);
        if transform.scale != escala {
            transform.scale = escala;
        }
        // La capa 0 es la de más arriba: la de mayor profundidad.
        let z = (total - capa.indice) as f32;
        if transform.translation.z != z {
            transform.translation.z = z;
            // El render de Bevy guarda el orden de dibujado con que encoló cada malla y NO
            // lo recalcula si solo cambia el `z` (se comprobó midiendo píxeles: al reordenar
            // capas seguía arriba la de antes). Volver a insertar el `Mesh2d` lo obliga a
            // encolarla de nuevo con el `z` nuevo. Pasa solo cuando cambia el orden.
            commands.entity(entidad).insert(Mesh2d(malla.0.clone()));
        }

        let deseada = if config.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibilidad != deseada {
            *visibilidad = deseada;
        }
    }
}

/// Le pasa a cada reproductor la duración del fundido del proyecto y hace avanzar el que
/// está en marcha. La curva (`avance_de_transicion`) es lógica pura de `vm_core`;
/// el reproductor solo cuenta el tiempo.
///
/// Se compara antes de escribir para no marcar el reproductor como modificado cada frame.
fn avanzar_transiciones(
    reloj: Res<Time>,
    proyecto: Res<ProyectoActual>,
    mut capas: Query<&mut Reproductor>,
) {
    let duracion = proyecto.0.composicion.transicion;
    let ahora = reloj.elapsed_secs_f64();
    for mut reproductor in &mut capas {
        if reproductor.transicion() != duracion {
            reproductor.fijar_transicion(duracion);
        }
        if let Some(segundos) = reproductor.segundos_en_transicion(ahora) {
            reproductor.avanzar_transicion(avance_de_transicion(segundos, duracion));
        }
    }
}

/// El clip que sale durante un fundido se dibuja con una malla propia (la capa tiene la del
/// clip nuevo). Vive solo mientras dura el fundido.
#[derive(Component)]
struct CapaSaliente {
    /// La entidad de la capa a la que pertenece.
    de: Entity,
}

/// Tamaño y posición de la malla del clip saliente: el mismo ajuste que la capa pero con el
/// tamaño de **su** video, y apenas debajo de la capa (más arriba que las capas de abajo,
/// que están a una unidad de distancia).
fn transform_de_saliente(
    ajuste: vm_core::Ajuste,
    composicion: (f32, f32),
    imagen: Option<&Image>,
    z_de_la_capa: f32,
) -> Transform {
    let video = imagen.map_or(composicion, |i| (i.width() as f32, i.height() as f32));
    let (ancho, alto) = tamano_de_ajuste(ajuste, composicion, video);
    Transform {
        translation: Vec3::new(0.0, 0.0, z_de_la_capa - 0.5),
        scale: Vec3::new(ancho, alto, 1.0),
        ..default()
    }
}

/// Crea, actualiza y borra las mallas de los clips salientes.
#[allow(clippy::too_many_arguments)]
fn sincronizar_salientes(
    mut commands: Commands,
    malla: Res<MallaDeCapa>,
    proyecto: Res<ProyectoActual>,
    composicion: Res<TexturaComposicion>,
    imagenes: Res<Assets<Image>>,
    senales: Res<SenalesDeEntrada>,
    mut materiales: ResMut<Assets<MaterialCapa>>,
    capas: Query<
        (
            Entity,
            &CapaComposicion,
            &Reproductor,
            &Transform,
            &Visibility,
        ),
        Without<CapaSaliente>,
    >,
    mut salientes: Query<(
        Entity,
        &CapaSaliente,
        &MeshMaterial2d<MaterialCapa>,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let tamano_composicion = (composicion.ancho as f32, composicion.alto as f32);

    // Las que ya existen: se actualizan, o se borran si el fundido terminó o la capa no está.
    let mut con_malla: Vec<Entity> = Vec::new();
    for (entidad, saliente, material, mut transform, mut visibilidad) in &mut salientes {
        let capa = capas.get(saliente.de).ok();
        let textura = capa.and_then(|(_, _, r, _, _)| r.textura_saliente());
        let (Some((_, indice, reproductor, transform_capa, visibilidad_capa)), Some(textura)) =
            (capa, textura)
        else {
            commands.entity(entidad).despawn();
            materiales.remove(&material.0);
            continue;
        };
        con_malla.push(saliente.de);
        let Some(config) = proyecto.0.composicion.capas.get(indice.indice) else {
            continue;
        };
        let (opacidad, _) = opacidades_de_transicion(
            opacidad_efectiva(config, &senales),
            reproductor.avance_de_entrada(),
            config.modo,
        );
        let parametros = ParametrosCapa::nuevos(opacidad, config.modo, false);
        let cambio = materiales
            .get(&material.0)
            .is_some_and(|m| m.textura != *textura || m.parametros != parametros);
        if cambio && let Some(mut m) = materiales.get_mut(&material.0) {
            m.textura = textura.clone();
            m.parametros = parametros;
        }
        let nuevo = transform_de_saliente(
            config.ajuste,
            tamano_composicion,
            imagenes.get(textura),
            transform_capa.translation.z,
        );
        if transform.scale != nuevo.scale {
            transform.scale = nuevo.scale;
        }
        if transform.translation.z != nuevo.translation.z {
            transform.translation.z = nuevo.translation.z;
            // Igual que en las capas: el render no reordena si solo cambia el `z`.
            commands.entity(entidad).insert(Mesh2d(malla.0.clone()));
        }
        if *visibilidad != *visibilidad_capa {
            *visibilidad = *visibilidad_capa;
        }
    }

    // Las capas con un fundido en marcha que todavía no tienen su malla.
    for (entidad, indice, reproductor, transform_capa, visibilidad_capa) in &capas {
        let Some(textura) = reproductor.textura_saliente() else {
            continue;
        };
        if con_malla.contains(&entidad) {
            continue;
        }
        let Some(config) = proyecto.0.composicion.capas.get(indice.indice) else {
            continue;
        };
        let (opacidad, _) = opacidades_de_transicion(
            opacidad_efectiva(config, &senales),
            reproductor.avance_de_entrada(),
            config.modo,
        );
        let material = materiales.add(MaterialCapa {
            parametros: ParametrosCapa::nuevos(opacidad, config.modo, false),
            textura: textura.clone(),
        });
        commands.spawn((
            CapaSaliente { de: entidad },
            Mesh2d(malla.0.clone()),
            MeshMaterial2d(material),
            transform_de_saliente(
                config.ajuste,
                tamano_composicion,
                imagenes.get(textura),
                transform_capa.translation.z,
            ),
            *visibilidad_capa,
            RenderLayers::from_layers(&[CAPA_COMPOSICION]),
        ));
    }
}

#[cfg(test)]
mod pruebas_de_fundido {
    use super::*;
    use std::time::{Duration, Instant};
    use vm_video::VideoPlugin;

    /// Corre la aplicación hasta que `listo` se cumpla (o se acaba el tiempo).
    fn correr_hasta(app: &mut App, mut listo: impl FnMut(&mut App) -> bool) -> bool {
        let limite = Instant::now() + Duration::from_secs(15);
        while Instant::now() < limite {
            app.update();
            if listo(app) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        false
    }

    /// Prueba con GStreamer de verdad y el video de `assets/`: abre un clip, cambia a otro
    /// con un fundido de 0,6 s y mira todo el ciclo. Se corre a mano:
    /// `cargo test -p vm_composicion fundido -- --ignored`
    #[test]
    #[ignore = "necesita GStreamer y el video de assets/"]
    fn al_cambiar_de_clip_con_fundido_el_viejo_sale_y_el_nuevo_entra() {
        let video = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/video_prueba.mp4");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .add_plugins(VideoPlugin)
            .add_systems(Update, avanzar_transiciones);
        let mut proyecto = ProyectoActual::default();
        proyecto.0.composicion.transicion = 0.6;
        app.insert_resource(proyecto);
        let reproductor = {
            let mut imagenes = app.world_mut().resource_mut::<Assets<Image>>();
            Reproductor::nuevo(&mut imagenes)
        };
        let capa = app.world_mut().spawn(reproductor).id();
        let ancho_de = |app: &App, textura: &Handle<Image>| {
            app.world()
                .resource::<Assets<Image>>()
                .get(textura)
                .map_or(0, |i| i.width())
        };

        // El primer clip: sin clip anterior, entra de golpe (no hay fundido).
        app.world_mut()
            .get_mut::<Reproductor>(capa)
            .unwrap()
            .abrir(&video)
            .unwrap();
        let con_imagen = correr_hasta(&mut app, |app| {
            let r = app.world().get::<Reproductor>(capa).unwrap();
            ancho_de(app, &r.textura) > 1
        });
        assert!(con_imagen, "el primer clip no entregó imagen");
        let r = app.world().get::<Reproductor>(capa).unwrap();
        assert!(
            r.textura_saliente().is_none(),
            "el primero no debe hacer fundido"
        );
        assert_eq!(r.avance_de_entrada(), 1.0);
        let textura_del_primero = r.textura.clone();

        // El segundo clip: el primero tiene que quedar como saliente con su propia textura.
        app.world_mut()
            .get_mut::<Reproductor>(capa)
            .unwrap()
            .abrir(&video)
            .unwrap();
        let mut visto_saliente = false;
        let mut avance_minimo = 1.0_f32;
        let mut avance_maximo = 0.0_f32;
        let termino = correr_hasta(&mut app, |app| {
            let r = app.world().get::<Reproductor>(capa).unwrap();
            if let Some(saliente) = r.textura_saliente() {
                visto_saliente = true;
                // La textura que se va es la del primero y no la misma que la del nuevo.
                assert_eq!(*saliente, textura_del_primero);
                assert_ne!(r.textura, textura_del_primero);
                avance_minimo = avance_minimo.min(r.avance_de_entrada());
                avance_maximo = avance_maximo.max(r.avance_de_entrada());
            }
            visto_saliente && r.textura_saliente().is_none()
        });
        assert!(visto_saliente, "nunca apareció el clip saliente");
        assert!(termino, "el fundido no terminó");
        let r = app.world().get::<Reproductor>(capa).unwrap();
        assert_eq!(
            r.avance_de_entrada(),
            1.0,
            "al terminar se ve solo el nuevo"
        );
        assert!(
            avance_minimo < 0.5,
            "el avance debía empezar cerca de 0: {avance_minimo}"
        );
        assert!(
            avance_maximo > 0.5,
            "el avance debía llegar cerca de 1: {avance_maximo}"
        );
        assert!(
            ancho_de(&app, &r.textura) > 1,
            "el clip nuevo no tiene imagen"
        );
    }
}

#[cfg(test)]
mod pruebas_de_modulacion {
    use super::*;
    use vm_core::{Capa, Modulacion, OrigenDeSenal, Senales};

    #[test]
    fn la_opacidad_efectiva_sigue_a_la_senal_solo_si_hay_modulacion() {
        let mut capa = Capa::nueva("C".into(), 1);
        capa.opacidad = 0.2;
        let mut senales = SenalesDeEntrada::default();
        let escritorio = vm_core::FuenteDeAudio::Escritorio;
        senales.poner(
            escritorio.clone(),
            Senales {
                medios: 0.5,
                ..Senales::default()
            },
        );
        // Sin modulación, la señal no importa.
        assert_eq!(opacidad_efectiva(&capa, &senales), 0.2);
        capa.modulacion_de_opacidad = Some(Modulacion {
            senal: OrigenDeSenal::Medios,
            cantidad: 0.8,
            fuente: escritorio,
        });
        // 0,2 + 0,8 · 0,5 = 0,6
        assert!((opacidad_efectiva(&capa, &senales) - 0.6).abs() < 1e-6);
        // Sin sonido vuelve a la base.
        assert_eq!(opacidad_efectiva(&capa, &SenalesDeEntrada::default()), 0.2);
    }

    #[test]
    fn cada_capa_escucha_su_propia_fuente() {
        use vm_core::FuenteDeAudio as F;
        let mut senales = SenalesDeEntrada::default();
        senales.poner(
            F::Escritorio,
            Senales {
                nivel: 1.0,
                ..Senales::default()
            },
        );
        senales.poner(F::Microfono, Senales::default());
        let con = |fuente: F| {
            let mut c = Capa::nueva("C".into(), 1);
            c.opacidad = 0.0;
            c.modulacion_de_opacidad = Some(Modulacion {
                fuente,
                senal: OrigenDeSenal::Nivel,
                cantidad: 1.0,
            });
            c
        };
        assert_eq!(opacidad_efectiva(&con(F::Escritorio), &senales), 1.0);
        assert_eq!(opacidad_efectiva(&con(F::Microfono), &senales), 0.0);
        // Una fuente que no se escucha es silencio.
        assert_eq!(opacidad_efectiva(&con(F::Capa(3)), &senales), 0.0);
    }

    #[test]
    fn conservar_olvida_las_fuentes_que_ya_no_se_usan() {
        use vm_core::FuenteDeAudio as F;
        let mut s = SenalesDeEntrada::default();
        let fuerte = Senales {
            nivel: 1.0,
            ..Senales::default()
        };
        s.poner(F::Escritorio, fuerte);
        s.poner(F::Microfono, fuerte);
        assert!(s.conservar(&[F::Escritorio]));
        assert_eq!(s.de(&F::Microfono), Senales::default());
        assert_eq!(s.de(&F::Escritorio), fuerte);
        assert!(!s.conservar(&[F::Escritorio]));
    }
}
