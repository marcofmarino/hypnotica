//! Plugin de salidas: la ventana de control y una ventana fullscreen por proyector.
//!
//! - El **monitor de control** (el principal) recibe una ventana normal, para la interfaz.
//! - Cada **monitor secundario** *activo* recibe una ventana fullscreen con su propia
//!   cámara y su propia capa de render: una *salida*.
//!
//! Qué salidas están activas lo dice el **proyecto** (`ConfigSalida::activa`). Este plugin
//! solo *reconcilia*: compara lo que el proyecto pide con lo que está abierto, y abre o
//! cierra ventanas hasta que coincidan. Así la interfaz no abre ventanas: solo cambia el proyecto.
//!
//! No sabe nada de superficies ni de video. Solo avisa "hay una salida nueva"
//! agregando el componente `Salida` a su cámara; otros plugins reaccionan a eso.

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{OrthographicProjection, Projection, RenderTarget, ScalingMode};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{
    CursorOptions, Monitor, MonitorSelection, OnMonitor, PresentMode, PrimaryMonitor, WindowMode,
    WindowPosition, WindowRef,
};
use vm_core::{ConfigSalida, PatronPrueba};
use vm_proyecto::ProyectoActual;

/// Qué patrón de prueba se está mostrando en cada salida (la clave es el id del monitor).
/// Si una salida no está en el mapa, muestra su contenido normal. Lo cambia la interfaz y lo
/// dibuja `vm_warp`. Es **temporal**: no se guarda en el proyecto, para que un evento nunca
/// arranque con una grilla proyectada por haber guardado con ella puesta.
#[derive(Resource, Default)]
pub struct PatronesActivos(pub HashMap<String, PatronActivo>);

/// Un patrón de prueba puesto en una salida y cómo se muestra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatronActivo {
    pub patron: PatronPrueba,
    /// `false`: se proyecta en pantalla completa, sin deformar (para ver el proyector).
    /// `true`: pasa por las superficies como si fuera la imagen de la composición, así se
    /// deforma con ellas (para alinear las esquinas y la malla con la pared).
    pub en_superficie: bool,
}

/// ¿La interfaz está mostrando alguna vista previa de las salidas? Mientras no (por ejemplo, en la
/// pestaña de Composición), las cámaras de vista previa se apagan: no tiene sentido dibujar a una
/// imagen que nadie mira. La escribe la interfaz según la pestaña abierta.
#[derive(Resource, Default)]
pub struct VistasPreviasNecesarias(pub bool);

pub struct SalidasPlugin;

impl Plugin for SalidasPlugin {
    fn build(&self, app: &mut App) {
        // Lo que se ve en una salida es lo que se proyecta, y el negro es "sin luz": el fondo
        // de todas las cámaras (salidas, vistas previas y composición) es negro, no el gris
        // que trae Bevy por defecto.
        app.insert_resource(ClearColor(Color::BLACK))
            .init_resource::<PatronesActivos>()
            .init_resource::<VistasPreviasNecesarias>()
            .add_systems(
                Update,
                (
                    abrir_control_inicial,
                    sincronizar_salidas,
                    activar_vistas_previas,
                    ajustar_presentacion_del_control,
                )
                    .chain(),
            );
    }
}

/// Prende o apaga las cámaras de vista previa según `VistasPreviasNecesarias`. Se mira antes de
/// escribir: pedir escritura marcaría la cámara como modificada en cada cuadro.
fn activar_vistas_previas(
    necesarias: Res<VistasPreviasNecesarias>,
    salidas: Query<&Salida>,
    mut camaras: Query<&mut Camera>,
) {
    for salida in &salidas {
        let Ok(camara) = camaras.get(salida.camara_previa) else {
            continue;
        };
        if camara.is_active != necesarias.0
            && let Ok(mut camara) = camaras.get_mut(salida.camara_previa)
        {
            camara.is_active = necesarias.0;
        }
    }
}

/// Las guías de cada salida (marcadores de esquinas) viven en una capa propia: la cámara
/// del proyector la ve solo si las guías están activadas, y la vista previa de la ventana
/// de control nunca (ahí las dibuja egui, y se pueden arrastrar).
const CAPA_GUIAS_BASE: usize = 1000;

/// Ancho en píxeles de la imagen de vista previa de cada salida. Es mucho menos que un
/// proyector real: la vista previa no necesita más y así cuesta poco renderizarla.
const ANCHO_VISTA_PREVIA: u32 = 960;

/// Cuánto más rápido tiene que ser el refresco de una salida (en Hz) para que cuente como
/// "más rápida" que el monitor de la ventana de control.
const DIFERENCIA_DE_REFRESCO_HZ: f32 = 1.0;

/// Bevy tiene un solo bucle de dibujo y, con vsync, avanza al ritmo del monitor **más lento**
/// entre sus ventanas. Si la ventana de control está en un monitor más lento que una salida
/// activa (por ejemplo, 60 Hz contra un proyector de 75 Hz), frenaría al proyector. Ese caso
/// se evita sacándole el vsync a la ventana de control; en cualquier otro, se deja el vsync
/// normal (sin cortes de imagen). **Sin probar con monitores de distinta frecuencia.**
fn ajustar_presentacion_del_control(
    control: Query<&Control>,
    en_monitor: Query<&OnMonitor>,
    monitores: Query<&Monitor>,
    salidas: Query<&Salida>,
    mut ventanas: Query<&mut Window>,
) {
    let Some(ventana) = control.iter().next().map(|c| c.ventana) else {
        return;
    };
    let hz = |monitor: Entity| {
        monitores
            .get(monitor)
            .ok()
            .and_then(|m| m.refresh_rate_millihertz)
            .map(|mhz| mhz as f32 / 1000.0)
    };
    let hz_del_control = en_monitor.get(ventana).ok().and_then(|e| hz(e.0));
    let salida_mas_rapida = salidas
        .iter()
        .filter_map(|s| hz(s.monitor))
        .fold(0.0_f32, f32::max);
    let frena = hz_del_control.is_some_and(|c| salida_mas_rapida > c + DIFERENCIA_DE_REFRESCO_HZ);
    let deseado = if frena {
        PresentMode::AutoNoVsync
    } else {
        PresentMode::AutoVsync
    };
    // Se mira primero con acceso de solo lectura: pedir escritura marcaría la ventana como
    // modificada en cada frame.
    if ventanas
        .get(ventana)
        .is_ok_and(|w| w.present_mode != deseado)
        && let Ok(mut w) = ventanas.get_mut(ventana)
    {
        w.present_mode = deseado;
        info!(
            "ventana de control: presentación {deseado:?} (control {hz_del_control:?} Hz, \
             salidas hasta {salida_mas_rapida} Hz)"
        );
    }
}

/// Marca la cámara de una salida (un monitor / proyector).
#[derive(Component)]
pub struct Salida {
    /// Identificador del monitor (ver `id_de_monitor`); es la clave en el proyecto.
    pub id: String,
    /// La entidad del monitor en Bevy.
    pub monitor: Entity,
    /// La ventana donde se dibuja esta salida (se cierra junto con ella).
    pub ventana: Entity,
    /// Capa de render propia: lo que esté en esta capa solo se ve en esta salida.
    pub capa: usize,
    /// Capa de las guías de esta salida (ver `CAPA_GUIAS_BASE`).
    pub capa_guias: usize,
    /// Imagen donde se renderiza la vista previa de esta salida.
    pub previa: Handle<Image>,
    /// La cámara que dibuja la vista previa (se cierra junto con la salida).
    pub camara_previa: Entity,
    /// Tamaño del área de dibujo en píxeles lógicos (los que usa la cámara 2D).
    pub tamano: Vec2,
}

/// Marca la cámara de la ventana de control (donde está la interfaz).
#[derive(Component)]
pub struct Control {
    /// La ventana donde está la interfaz.
    pub ventana: Entity,
}

/// El monitor donde se **pidió** abrir la ventana de control. En Wayland es solo una
/// sugerencia (el compositor decide), así que sirve de respaldo: para saber dónde está
/// *realmente* la ventana, usar `UbicacionControl`.
#[derive(Resource)]
pub struct MonitorControl(pub Entity);

/// Responde dónde está la ventana de control ahora mismo. Bevy mantiene en cada ventana
/// un componente `OnMonitor` con el monitor en el que está; así, si la movés con el
/// compositor (Hyprland), el programa se entera.
#[derive(SystemParam)]
pub struct UbicacionControl<'w, 's> {
    control: Query<'w, 's, &'static Control>,
    en_monitor: Query<'w, 's, &'static OnMonitor>,
    sugerido: Option<Res<'w, MonitorControl>>,
}

impl UbicacionControl<'_, '_> {
    /// El monitor donde está la ventana de control **según el sistema**, o `None` si
    /// todavía no lo informó (la ventana recién se abrió).
    pub fn monitor_real(&self) -> Option<Entity> {
        self.control
            .iter()
            .find_map(|c| self.en_monitor.get(c.ventana).ok())
            .map(|en| en.0)
    }

    /// El monitor donde está la ventana de control, o el sugerido si todavía no se sabe.
    pub fn monitor(&self) -> Option<Entity> {
        self.monitor_real().or(self.sugerido.as_ref().map(|m| m.0))
    }
}

/// Estado del arranque de `sincronizar_salidas`.
#[derive(Default)]
struct Arranque {
    /// Frames esperando a saber dónde está de verdad la ventana de control.
    espera: u32,
    /// Si ya se hizo la primera pasada, que comprueba que ninguna salida arranque tapando
    /// la ventana de control.
    protegido: bool,
}

/// Cuántos frames se espera a que el sistema informe dónde está la ventana de control,
/// antes de conformarse con el monitor sugerido (por si algún sistema nunca lo informa).
const FRAMES_DE_ESPERA_DEL_CONTROL: u32 = 120;

/// Identificador estable de un monitor, la clave con la que el proyecto recuerda
/// su configuración. **Es el único lugar que decide cómo se identifica un monitor**:
/// si algún día se cambia el criterio (por ejemplo, modelo y número de serie), se
/// cambia acá y se agrega una migración del archivo.
///
/// Hoy es el nombre que informa el sistema (en Hyprland, el puerto: `DP-1`, `HDMI-A-1`).
pub fn id_de_monitor(monitor: &Monitor) -> String {
    monitor.name.clone().unwrap_or_else(|| {
        // Sin nombre, lo mejor que hay es su posición y tamaño.
        format!(
            "monitor-{}x{}@{},{}",
            monitor.physical_width,
            monitor.physical_height,
            monitor.physical_position.x,
            monitor.physical_position.y
        )
    })
}

/// Tamaño del monitor en píxeles lógicos (los que usa la cámara 2D).
pub fn tamano_logico(monitor: &Monitor) -> Vec2 {
    Vec2::new(
        monitor.physical_width as f32,
        monitor.physical_height as f32,
    ) / monitor.scale_factor as f32
}

/// Dónde se abre una salida.
pub enum DestinoSalida {
    /// Fullscreen en un monitor (lo normal: un proyector).
    Monitor { tamano: Vec2 },
    /// Ventana normal. Sirve para probar cuando hay un solo monitor.
    Ventana { tamano: Vec2 },
}

/// Abre una salida: una ventana (sin cursor), una cámara que dibuja en ella y su capa de render.
fn abrir_salida(
    commands: &mut Commands,
    imagenes: &mut Assets<Image>,
    id: String,
    monitor: Entity,
    capa: usize,
    destino: DestinoSalida,
) {
    let (modo, tamano) = match destino {
        DestinoSalida::Monitor { tamano } => (
            WindowMode::BorderlessFullscreen(MonitorSelection::Entity(monitor)),
            tamano,
        ),
        DestinoSalida::Ventana { tamano } => (WindowMode::Windowed, tamano),
    };

    let ventana = commands
        .spawn((
            Window {
                title: format!("Salida {id}"),
                mode: modo,
                resolution: (tamano.x as u32, tamano.y as u32).into(),
                ..default()
            },
            // Lo que se ve en la salida es lo que se proyecta: el cursor no debe verse.
            CursorOptions {
                visible: false,
                ..default()
            },
        ))
        .id();

    let capa_guias = CAPA_GUIAS_BASE + capa;

    // Vista previa: otra cámara que ve lo mismo que el proyector (pero sin las guías) y
    // dibuja en una imagen chica. `ScalingMode::Fixed` hace que el área visible siga
    // siendo la del monitor (mismas coordenadas), aunque la imagen sea más pequeña.
    let alto_previa = (ANCHO_VISTA_PREVIA as f32 * tamano.y / tamano.x).round() as u32;
    let previa = imagenes.add(Image::new_target_texture(
        ANCHO_VISTA_PREVIA,
        alto_previa,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let camara_previa = commands
        .spawn((
            Camera2d,
            RenderTarget::Image(previa.clone().into()),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::Fixed {
                    width: tamano.x,
                    height: tamano.y,
                },
                ..OrthographicProjection::default_2d()
            }),
            RenderLayers::layer(capa),
        ))
        .id();

    commands.spawn((
        Camera2d,
        RenderTarget::Window(WindowRef::Entity(ventana)),
        // El proyector ve el contenido y, si están activadas, las guías.
        RenderLayers::from_layers(&[capa, capa_guias]),
        Salida {
            id,
            monitor,
            ventana,
            capa,
            capa_guias,
            previa,
            camara_previa,
            tamano,
        },
    ));
}

/// Cierra una salida: despawnear la ventana la cierra, y la cámara ya no tiene dónde dibujar.
fn cerrar_salida(commands: &mut Commands, camara: Entity, salida: &Salida) {
    commands.entity(salida.ventana).despawn();
    commands.entity(salida.camara_previa).despawn();
    commands.entity(camara).despawn();
}

/// Abre la ventana de control, centrada en el monitor indicado.
///
/// En Wayland una ventana normal **no** puede pedir en qué monitor aparecer: lo
/// decide el compositor (Hyprland la pone donde está el foco). `Centered` se
/// respeta en otros sistemas, pero acá es solo una sugerencia.
fn abrir_control(commands: &mut Commands, monitor: Entity) {
    let ventana = commands
        .spawn(Window {
            title: "Hypnotica — Control".into(),
            resolution: (1280, 720).into(),
            position: WindowPosition::Centered(MonitorSelection::Entity(monitor)),
            ..default()
        })
        .id();

    commands.spawn((
        Camera2d,
        RenderTarget::Window(WindowRef::Entity(ventana)),
        Control { ventana },
    ));
}

/// Decide qué monitor es el de control. Cada candidato es `(marcado como
/// principal por Bevy, posición en el escritorio)`; devuelve el índice elegido.
///
/// 1. El que Bevy marca como principal. (En Wayland no existe: winit no lo informa.)
/// 2. El que está en la posición (0, 0) del escritorio.
/// 3. El primero de la lista.
fn indice_monitor_control(candidatos: &[(bool, IVec2)]) -> usize {
    candidatos
        .iter()
        .position(|(principal, _)| *principal)
        .or_else(|| candidatos.iter().position(|(_, pos)| *pos == IVec2::ZERO))
        .unwrap_or(0)
}

/// Una sola vez, cuando ya hay monitores: elige el de control y abre su ventana.
fn abrir_control_inicial(
    mut commands: Commands,
    existente: Option<Res<MonitorControl>>,
    monitores: Query<(Entity, &Monitor, Has<PrimaryMonitor>)>,
) {
    if existente.is_some() {
        return;
    }
    let lista: Vec<_> = monitores.iter().collect();
    if lista.is_empty() {
        return;
    }
    let candidatos: Vec<_> = lista
        .iter()
        .map(|(_, monitor, principal)| (*principal, monitor.physical_position))
        .collect();
    let (control, _, _) = lista[indice_monitor_control(&candidatos)];

    abrir_control(&mut commands, control);
    commands.insert_resource(MonitorControl(control));
}

/// Hace que las salidas abiertas coincidan con el proyecto:
/// 1. Todo monitor que pueda ser salida tiene su configuración en el proyecto.
/// 2. Una salida activa se abre; una inactiva se cierra.
/// 3. Si un monitor desaparece (se desenchufa), su salida se cierra.
fn sincronizar_salidas(
    mut commands: Commands,
    ubicacion: UbicacionControl,
    monitores: Query<(Entity, &Monitor)>,
    salidas: Query<(Entity, &Salida)>,
    mut proyecto: ResMut<ProyectoActual>,
    mut imagenes: ResMut<Assets<Image>>,
    mut arranque: Local<Arranque>,
) {
    // No se abre ninguna salida hasta saber en qué monitor cayó la ventana de control:
    // en Wayland lo decide el compositor, y una salida a pantalla completa en ese
    // monitor taparía la interfaz.
    let monitor_control = match ubicacion.monitor_real() {
        Some(real) => real,
        None => {
            arranque.espera += 1;
            match ubicacion.monitor() {
                Some(sugerido) if arranque.espera >= FRAMES_DE_ESPERA_DEL_CONTROL => sugerido,
                _ => return,
            }
        }
    };

    // Los monitores se ordenan de izquierda a derecha para que, al adoptar
    // configuraciones viejas, el resultado sea siempre el mismo.
    let mut lista: Vec<_> = monitores.iter().collect();
    lista.sort_by_key(|(_, m)| (m.physical_position.x, m.physical_position.y));
    let un_solo_monitor = lista.len() == 1;

    // 3. Salidas cuyo monitor ya no existe.
    for (camara, salida) in &salidas {
        if monitores.get(salida.monitor).is_err() {
            cerrar_salida(&mut commands, camara, salida);
        }
    }

    // Capas ya ocupadas; se va completando con las que se abren en esta pasada.
    let mut capas_usadas: Vec<usize> = salidas.iter().map(|(_, s)| s.capa).collect();

    // Se activa recién después de la primera pasada completa (ver `Arranque::protegido`).
    let primera_pasada = !arranque.protegido;
    for (entidad, monitor) in lista {
        // Cualquier monitor puede ser salida; el del control, solo si el usuario lo tilda.
        let es_control = entidad == monitor_control && !un_solo_monitor;
        let id = id_de_monitor(monitor);
        let tamano = tamano_logico(monitor);

        // 1. Configuración en el proyecto. Se pregunta primero con acceso de solo lectura:
        // pedir acceso de escritura marcaría el proyecto como modificado en cada frame.
        if !proyecto.0.salidas.iter().any(|s| s.monitor == id) {
            // Una configuración vieja sin monitor asignado la adopta un monitor que no
            // sea el de control.
            let huerfana = if es_control {
                None
            } else {
                proyecto.0.salidas.iter_mut().find(|s| s.monitor.is_empty())
            };
            match huerfana {
                Some(huerfana) => huerfana.monitor = id.clone(),
                None => {
                    let mut nueva = ConfigSalida::nueva(id.clone(), tamano.x, tamano.y);
                    // El monitor con la ventana de control no se enciende solo: taparía la interfaz.
                    nueva.activa = !es_control;
                    proyecto.0.salidas.push(nueva);
                }
            }
        }

        // Protección: la primera vez, si la salida del monitor de la ventana de control figura
        // activa (por ejemplo, de una sesión anterior en la que la ventana estaba en otro
        // monitor), se desactiva: nunca se arranca tapando la interfaz. Para proyectar ahí
        // a propósito, se tilda a mano en el panel de monitores.
        if es_control
            && primera_pasada
            && let Some(config) = proyecto
                .0
                .salidas
                .iter_mut()
                .find(|s| s.monitor == id && s.activa)
        {
            config.activa = false;
            warn!(
                "la ventana de control quedó en {id}: se desactivó su salida para no taparla \
                 (se puede volver a activar en el panel de monitores)"
            );
        }

        // 2. Abrir o cerrar según lo que diga el proyecto.
        let activa = proyecto
            .0
            .salidas
            .iter()
            .find(|s| s.monitor == id)
            .is_some_and(|s| s.activa);
        let abierta = salidas.iter().find(|(_, s)| s.monitor == entidad);

        match (activa, abierta) {
            (true, None) => {
                // La capa libre más baja; la 0 es la de por defecto, así que se empieza en 1.
                let capa = (1..).find(|c| !capas_usadas.contains(c)).unwrap_or(1);
                capas_usadas.push(capa);
                let destino = if un_solo_monitor {
                    info!("hay un solo monitor: la salida se abre en una ventana normal");
                    DestinoSalida::Ventana {
                        tamano: Vec2::new(1280.0, 720.0),
                    }
                } else {
                    DestinoSalida::Monitor { tamano }
                };
                abrir_salida(&mut commands, &mut imagenes, id, entidad, capa, destino);
            }
            (false, Some((camara, salida))) => cerrar_salida(&mut commands, camara, salida),
            _ => {}
        }
    }

    // Terminada la primera pasada, ya no se desactiva nada solo: si el usuario tilda la
    // salida del monitor de control, es a propósito.
    arranque.protegido = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gana_el_marcado_como_principal() {
        let c = [(false, IVec2::ZERO), (true, IVec2::new(2560, 0))];
        assert_eq!(indice_monitor_control(&c), 1);
    }

    #[test]
    fn sin_principal_gana_el_del_origen() {
        let c = [(false, IVec2::new(2560, 0)), (false, IVec2::ZERO)];
        assert_eq!(indice_monitor_control(&c), 1);
    }

    #[test]
    fn sin_pistas_gana_el_primero() {
        let c = [(false, IVec2::new(1920, 0)), (false, IVec2::new(-1920, 0))];
        assert_eq!(indice_monitor_control(&c), 0);
    }
}

#[cfg(test)]
mod pruebas_de_vistas_previas {
    use super::*;

    fn salida_con(camara: Entity) -> Salida {
        Salida {
            id: "HDMI-1".into(),
            monitor: Entity::PLACEHOLDER,
            ventana: Entity::PLACEHOLDER,
            capa: 1,
            capa_guias: 1001,
            previa: Handle::default(),
            camara_previa: camara,
            tamano: Vec2::new(1920.0, 1080.0),
        }
    }

    #[test]
    fn la_vista_previa_solo_se_dibuja_cuando_alguien_la_mira() {
        let mut app = App::new();
        app.init_resource::<VistasPreviasNecesarias>()
            .add_systems(Update, activar_vistas_previas);
        let camara = app.world_mut().spawn(Camera::default()).id();
        app.world_mut().spawn(salida_con(camara));

        // Nadie la mira: se apaga.
        app.update();
        assert!(!app.world().get::<Camera>(camara).unwrap().is_active);

        // La interfaz muestra una vista previa: se prende.
        app.world_mut().resource_mut::<VistasPreviasNecesarias>().0 = true;
        app.update();
        assert!(app.world().get::<Camera>(camara).unwrap().is_active);

        // Y se vuelve a apagar al cambiar de pestaña.
        app.world_mut().resource_mut::<VistasPreviasNecesarias>().0 = false;
        app.update();
        assert!(!app.world().get::<Camera>(camara).unwrap().is_active);
    }
}
