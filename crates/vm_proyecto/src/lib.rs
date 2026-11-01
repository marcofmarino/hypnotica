//! Plugin que guarda el proyecto como recurso de Bevy y lo guarda/carga en disco.
//!
//! Nadie guarda ni carga directamente: se envía un mensaje `OrdenProyecto` y este
//! plugin lo ejecuta. Atajos: **Ctrl+S** guarda, **Ctrl+Shift+S** guarda como y
//! **Ctrl+O** abre otro proyecto, **Ctrl+Z** deshace, **Ctrl+Shift+Z** (o **Ctrl+Y**)
//! rehace y **Ctrl+Q** sale (pidiendo confirmación si hay cambios sin guardar). Los botones de la interfaz (y más adelante MIDI u OSC)
//! mandan las mismas órdenes. Al arrancar, si existe el archivo, se carga solo.
//!
//! "Guardar como" y "Abrir" muestran el selector de archivos del sistema. El selector
//! **bloquea** hasta que se elige, así que corre en un hilo aparte y el resultado vuelve
//! por un canal (si no, la aplicación entera —y los proyectores— se congelaría).

pub mod historial;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::WindowCloseRequested;
use historial::{Historial, cambio_estructural};
use vm_core::Proyecto;

const RUTA_POR_DEFECTO: &str = "proyecto.ron";

/// Cada cuántos segundos se autoguarda (si hay cambios sin guardar).
const INTERVALO_AUTOGUARDADO: f64 = 30.0;

/// Durante los primeros segundos, los cambios que el programa hace solo (por ejemplo, dar de
/// alta las salidas de los monitores) no cuentan como cambios del usuario.
const SEGUNDOS_DE_ARRANQUE: f64 = 3.0;

pub struct ProyectoPlugin;

/// Las rutas donde puede estar el proyecto al arrancar, de la preferida a la última opción.
/// Es lógica pura (no mira el disco) para poder probarla: `existe_local` dice si hay un
/// `proyecto.ron` en la carpeta actual, `home` es la carpeta personal y `datos` la carpeta
/// de datos del usuario (si se pudieron averiguar).
///
/// Un `proyecto.ron` en la carpeta actual gana (así se sigue trabajando como en desarrollo);
/// si no, `~/Proyectos/hypnotica/`. Dentro de un Flatpak esa carpeta solo se puede escribir
/// si el manifiesto lo permite, así que si no sirve queda la carpeta de datos del usuario,
/// que el sandbox siempre da.
fn rutas_candidatas(
    existe_local: bool,
    home: Option<PathBuf>,
    datos: Option<PathBuf>,
) -> Vec<PathBuf> {
    if existe_local {
        return vec![PathBuf::from(RUTA_POR_DEFECTO)];
    }
    let mut rutas = Vec::new();
    if let Some(home) = home {
        rutas.push(
            home.join("Proyectos")
                .join("hypnotica")
                .join(RUTA_POR_DEFECTO),
        );
    }
    if let Some(datos) = datos {
        rutas.push(
            datos
                .join("Proyectos")
                .join("hypnotica")
                .join(RUTA_POR_DEFECTO),
        );
    }
    rutas.push(PathBuf::from(RUTA_POR_DEFECTO));
    rutas
}

/// `$XDG_DATA_HOME` o, si no está, `~/.local/share`.
fn carpeta_de_datos() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// La ruta con la que arranca el programa: la primera candidata cuya carpeta existe o se
/// puede crear (para que guardar no falle).
fn ruta_inicial() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidatas = rutas_candidatas(
        Path::new(RUTA_POR_DEFECTO).exists(),
        home,
        carpeta_de_datos(),
    );
    for ruta in &candidatas {
        match ruta.parent().filter(|c| !c.as_os_str().is_empty()) {
            None => return ruta.clone(),
            Some(carpeta) => match std::fs::create_dir_all(carpeta) {
                Ok(()) => return ruta.clone(),
                Err(e) => warn!("No se pudo usar {}: {e}", carpeta.display()),
            },
        }
    }
    PathBuf::from(RUTA_POR_DEFECTO)
}

impl Plugin for ProyectoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProyectoActual>()
            .init_resource::<EstadoProyecto>()
            .init_resource::<TecladoOcupado>()
            .init_resource::<DialogoProyecto>()
            .init_resource::<HistorialProyecto>()
            .init_resource::<SalidaPendiente>()
            .insert_resource(RutaProyecto(ruta_inicial()))
            .add_message::<OrdenProyecto>()
            .add_message::<ProyectoCargado>()
            .add_systems(Startup, cargar_al_inicio)
            .add_systems(
                Update,
                (
                    atajos_guardar_cargar,
                    pedir_salida_al_cerrar,
                    ejecutar_ordenes,
                    recibir_dialogo,
                    registrar_historial,
                    autoguardar,
                )
                    .chain(),
            );
    }
}

/// El proyecto abierto. Es la fuente de verdad: el resto de los plugins lo leen
/// y lo modifican; nadie guarda su propia copia de los datos.
///
/// `#[derive(Resource, Default)]`: `Resource` lo vuelve un recurso global de Bevy
/// y `Default` lo crea vacío con `init_resource`.
#[derive(Resource, Default)]
pub struct ProyectoActual(pub Proyecto);

/// Archivo donde se guarda y de donde se carga.
#[derive(Resource)]
pub struct RutaProyecto(pub PathBuf);

impl RutaProyecto {
    /// La carpeta (absoluta) que contiene el proyecto. Las rutas de los videos se guardan
    /// relativas a ella.
    pub fn carpeta(&self) -> PathBuf {
        let padre = self
            .0
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::canonicalize(padre).unwrap_or_else(|_| padre.to_path_buf())
    }
}

/// La interfaz está usando el teclado (por ejemplo, se está escribiendo en un campo de
/// texto). Los atajos globales (flechas, números, Esc, Ctrl+S...) **no deben reaccionar**
/// mientras sea `true`: si no, escribir un nombre movería esquinas o cerraría el programa.
///
/// Lo actualiza la interfaz en cada frame; los demás plugins solo lo consultan.
#[derive(Resource, Default)]
pub struct TecladoOcupado(pub bool);

/// Aviso de que se **cargó** un proyecto (al arrancar o con Ctrl+O). Quien tenga estado
/// propio que dependa del proyecto (por ejemplo, qué clip suena en cada capa) lo reinicia.
#[derive(Message, Clone, Copy, Debug)]
pub struct ProyectoCargado;

/// Resultado de la última orden, para mostrarlo en la interfaz.
#[derive(Resource, Default)]
pub struct EstadoProyecto {
    pub mensaje: String,
    /// Hay un autoguardado más nuevo que el archivo del proyecto (el programa se cerró mal
    /// la vez anterior): la interfaz ofrece recuperarlo o descartarlo.
    pub autoguardado_disponible: bool,
}

/// El historial de deshacer y rehacer, y el control de cambios sin guardar.
#[derive(Resource)]
pub struct HistorialProyecto(pub Historial);

impl Default for HistorialProyecto {
    fn default() -> Self {
        HistorialProyecto(Historial::nuevo(&Proyecto::default()))
    }
}

/// Se pidió salir y hay cambios sin guardar: la interfaz pregunta qué hacer.
#[derive(Resource, Default)]
pub struct SalidaPendiente(pub bool);

/// El archivo donde se autoguarda: oculto, al lado del proyecto (`.proyecto.ron.autosave`).
pub fn ruta_autoguardado(ruta: &Path) -> PathBuf {
    let nombre = ruta.file_name().map_or_else(
        || RUTA_POR_DEFECTO.into(),
        |n| n.to_string_lossy().into_owned(),
    );
    ruta.with_file_name(format!(".{nombre}.autosave"))
}

/// Una orden sobre el archivo del proyecto. Un *mensaje* de Bevy es un aviso que
/// un sistema envía (`MessageWriter`) y otro recibe (`MessageReader`), sin que
/// se conozcan entre sí.
#[derive(Message, Clone, Copy, Debug)]
pub enum OrdenProyecto {
    /// Guarda en el archivo actual.
    Guardar,
    /// Pide (con un selector de archivos) dónde y con qué nombre guardar, y guarda ahí.
    GuardarComo,
    /// Pide (con un selector de archivos) qué proyecto abrir, y lo abre.
    Cargar,
    /// Vuelve al estado anterior del proyecto.
    Deshacer,
    /// Repite lo que se acaba de deshacer.
    Rehacer,
    /// Quiere salir: si hay cambios sin guardar, se pregunta antes.
    PedirSalida,
    /// Guarda y sale.
    GuardarYSalir,
    /// Sale sin guardar.
    SalirSinGuardar,
    /// Se arrepintió de salir.
    CancelarSalida,
    /// Carga el autoguardado que dejó la sesión anterior.
    RecuperarAutoguardado,
    /// Borra el autoguardado de la sesión anterior.
    DescartarAutoguardado,
}

/// Qué se está pidiendo en el selector de archivos.
#[derive(Clone, Copy)]
enum PedidoDialogo {
    Abrir,
    GuardarComo,
}

struct DialogoPendiente {
    pedido: PedidoDialogo,
    /// `Receiver` no se puede compartir entre hilos sin esto; un recurso de Bevy tiene que
    /// poder, así que se guarda dentro de un `Mutex`.
    respuesta: Mutex<Receiver<Option<PathBuf>>>,
}

/// El selector de archivos abierto, si lo hay (solo puede haber uno).
#[derive(Resource, Default)]
struct DialogoProyecto {
    pendiente: Option<DialogoPendiente>,
}

impl DialogoProyecto {
    /// Abre el selector en un hilo aparte. Si ya hay uno abierto, no hace nada.
    fn abrir(&mut self, pedido: PedidoDialogo, ruta_actual: &Path, carpeta: PathBuf) {
        if self.pendiente.is_some() {
            return;
        }
        let nombre = ruta_actual.file_name().map_or_else(
            || RUTA_POR_DEFECTO.into(),
            |n| n.to_string_lossy().into_owned(),
        );
        let (emisor, receptor) = mpsc::channel();
        std::thread::spawn(move || {
            let dialogo = rfd::FileDialog::new()
                .set_directory(carpeta)
                .add_filter("Proyecto de Hypnotica", &["ron"]);
            let elegido = match pedido {
                PedidoDialogo::Abrir => dialogo.set_title("Abrir proyecto").pick_file(),
                PedidoDialogo::GuardarComo => dialogo
                    .set_title("Guardar proyecto como")
                    .set_file_name(nombre)
                    .save_file(),
            };
            let _ = emisor.send(elegido);
        });
        self.pendiente = Some(DialogoPendiente {
            pedido,
            respuesta: Mutex::new(receptor),
        });
    }
}

fn cargar_al_inicio(
    mut proyecto: ResMut<ProyectoActual>,
    ruta: Res<RutaProyecto>,
    mut estado: ResMut<EstadoProyecto>,
    mut historial: ResMut<HistorialProyecto>,
    mut avisos: MessageWriter<ProyectoCargado>,
) {
    if !ruta.0.exists() {
        info!(
            "no hay {}: se empieza con un proyecto nuevo",
            ruta.0.display()
        );
    } else {
        match Proyecto::cargar(&ruta.0) {
            Ok(p) => {
                info!("proyecto cargado desde {}", ruta.0.display());
                proyecto.0 = p;
                avisos.write(ProyectoCargado);
            }
            Err(e) => error!("no se pudo cargar {}: {e}", ruta.0.display()),
        }
    }
    historial.0.reiniciar(&proyecto.0);

    // ¿Quedó un autoguardado más nuevo que el proyecto? (El programa se cerró sin guardar.)
    let auto = ruta_autoguardado(&ruta.0);
    let fecha = |r: &Path| std::fs::metadata(r).and_then(|m| m.modified()).ok();
    estado.autoguardado_disponible = match (fecha(&auto), fecha(&ruta.0)) {
        (Some(a), Some(p)) => a > p,
        (Some(_), None) => true,
        _ => false,
    };
    if estado.autoguardado_disponible {
        warn!("hay un autoguardado más nuevo que {}", ruta.0.display());
    }
}

fn atajos_guardar_cargar(
    teclas: Res<ButtonInput<KeyCode>>,
    ocupado: Res<TecladoOcupado>,
    mut ordenes: MessageWriter<OrdenProyecto>,
) {
    if ocupado.0 {
        return;
    }
    if !teclas.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        return;
    }
    let shift = teclas.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if teclas.just_pressed(KeyCode::KeyS) {
        if shift {
            ordenes.write(OrdenProyecto::GuardarComo);
        } else {
            ordenes.write(OrdenProyecto::Guardar);
        }
    }
    if teclas.just_pressed(KeyCode::KeyO) {
        ordenes.write(OrdenProyecto::Cargar);
    }
    if teclas.just_pressed(KeyCode::KeyZ) {
        if shift {
            ordenes.write(OrdenProyecto::Rehacer);
        } else {
            ordenes.write(OrdenProyecto::Deshacer);
        }
    }
    if teclas.just_pressed(KeyCode::KeyY) {
        ordenes.write(OrdenProyecto::Rehacer);
    }
    if teclas.just_pressed(KeyCode::KeyQ) {
        ordenes.write(OrdenProyecto::PedirSalida);
    }
}

/// Cerrar la ventana de control (o el pedido de cierre del compositor) no cierra la ventana
/// sin más: es un pedido de salir, que pasa por la confirmación.
fn pedir_salida_al_cerrar(
    mut cierres: MessageReader<WindowCloseRequested>,
    mut ordenes: MessageWriter<OrdenProyecto>,
) {
    if cierres.read().count() > 0 {
        ordenes.write(OrdenProyecto::PedirSalida);
    }
}

/// Lo que las órdenes necesitan de la sesión, agrupado para no pasar tantos parámetros.
#[derive(SystemParam)]
struct Sesion<'w> {
    historial: ResMut<'w, HistorialProyecto>,
    salida_pendiente: ResMut<'w, SalidaPendiente>,
    salir: MessageWriter<'w, AppExit>,
    cargado: MessageWriter<'w, ProyectoCargado>,
}

/// Guarda el proyecto en su archivo y lo anota (mensaje, historial, autoguardado).
fn guardar_en_su_archivo(
    proyecto: &Proyecto,
    ruta: &RutaProyecto,
    estado: &mut EstadoProyecto,
    historial: &mut Historial,
) -> bool {
    let nombre = ruta.0.display();
    match proyecto.guardar(&ruta.0) {
        Ok(()) => {
            historial.marcar_guardado(proyecto);
            let _ = std::fs::remove_file(ruta_autoguardado(&ruta.0));
            estado.mensaje = format!("Guardado en {nombre}");
            info!("{}", estado.mensaje);
            true
        }
        Err(e) => {
            estado.mensaje = format!("No se pudo guardar: {e}");
            error!("{}", estado.mensaje);
            false
        }
    }
}

fn ejecutar_ordenes(
    mut ordenes: MessageReader<OrdenProyecto>,
    mut proyecto: ResMut<ProyectoActual>,
    mut estado: ResMut<EstadoProyecto>,
    ruta: Res<RutaProyecto>,
    mut dialogo: ResMut<DialogoProyecto>,
    mut sesion: Sesion,
) {
    for orden in ordenes.read() {
        match *orden {
            OrdenProyecto::Guardar => {
                guardar_en_su_archivo(&proyecto.0, &ruta, &mut estado, &mut sesion.historial.0);
            }
            OrdenProyecto::GuardarComo => {
                dialogo.abrir(PedidoDialogo::GuardarComo, &ruta.0, ruta.carpeta());
            }
            OrdenProyecto::Cargar => {
                dialogo.abrir(PedidoDialogo::Abrir, &ruta.0, ruta.carpeta());
            }
            OrdenProyecto::Deshacer | OrdenProyecto::Rehacer => {
                let deshacer = matches!(orden, OrdenProyecto::Deshacer);
                let cambio = if deshacer {
                    sesion.historial.0.deshacer(&proyecto.0)
                } else {
                    sesion.historial.0.rehacer(&proyecto.0)
                };
                if let Some(nuevo) = cambio {
                    // Si cambió qué clips hay y dónde, la reproducción empieza de nuevo.
                    if cambio_estructural(&proyecto.0, &nuevo) {
                        sesion.cargado.write(ProyectoCargado);
                    }
                    proyecto.0 = nuevo;
                    estado.mensaje = if deshacer { "Deshecho" } else { "Rehecho" }.into();
                }
            }
            OrdenProyecto::PedirSalida => {
                if sesion.historial.0.sin_guardar(&proyecto.0) {
                    sesion.salida_pendiente.0 = true;
                } else {
                    let _ = std::fs::remove_file(ruta_autoguardado(&ruta.0));
                    sesion.salir.write(AppExit::Success);
                }
            }
            OrdenProyecto::GuardarYSalir => {
                if guardar_en_su_archivo(&proyecto.0, &ruta, &mut estado, &mut sesion.historial.0) {
                    sesion.salir.write(AppExit::Success);
                } else {
                    // No se pudo guardar: no se sale (se perdería el trabajo).
                    sesion.salida_pendiente.0 = false;
                }
            }
            OrdenProyecto::SalirSinGuardar => {
                let _ = std::fs::remove_file(ruta_autoguardado(&ruta.0));
                sesion.salir.write(AppExit::Success);
            }
            OrdenProyecto::CancelarSalida => sesion.salida_pendiente.0 = false,
            OrdenProyecto::RecuperarAutoguardado => {
                let auto = ruta_autoguardado(&ruta.0);
                match Proyecto::cargar(&auto) {
                    Ok(p) => {
                        // El proyecto queda "con cambios": lo guardado sigue siendo el archivo.
                        sesion.historial.0.reiniciar_sobre(&p);
                        proyecto.0 = p;
                        sesion.cargado.write(ProyectoCargado);
                        let _ = std::fs::remove_file(&auto);
                        estado.autoguardado_disponible = false;
                        estado.mensaje = "Se recuperó el autoguardado (falta guardar)".into();
                    }
                    Err(e) => estado.mensaje = format!("No se pudo recuperar: {e}"),
                }
            }
            OrdenProyecto::DescartarAutoguardado => {
                let _ = std::fs::remove_file(ruta_autoguardado(&ruta.0));
                estado.autoguardado_disponible = false;
            }
        }
    }
}

/// Anota los cambios del proyecto en el historial. Durante los primeros segundos los cambios
/// son del propio arranque (altas de monitores) y se toman como el punto de partida.
fn registrar_historial(
    proyecto: Res<ProyectoActual>,
    mut historial: ResMut<HistorialProyecto>,
    tiempo: Res<Time>,
) {
    let ahora = tiempo.elapsed_secs_f64();
    if ahora < SEGUNDOS_DE_ARRANQUE {
        if proyecto.is_changed() {
            historial.0.reiniciar(&proyecto.0);
        }
        return;
    }
    historial
        .0
        .observar(&proyecto.0, proyecto.is_changed(), ahora);
}

/// Cada tanto, si hay cambios sin guardar, deja una copia al lado del proyecto. Si el
/// programa se cierra mal, la próxima vez se ofrece recuperarla.
fn autoguardar(
    proyecto: Res<ProyectoActual>,
    historial: Res<HistorialProyecto>,
    estado: Res<EstadoProyecto>,
    ruta: Res<RutaProyecto>,
    tiempo: Res<Time>,
    mut ultimo: Local<(f64, String)>,
) {
    // Mientras haya un autoguardado viejo esperando decisión, no se lo pisa.
    if estado.autoguardado_disponible {
        return;
    }
    let ahora = tiempo.elapsed_secs_f64();
    if ahora - ultimo.0 < INTERVALO_AUTOGUARDADO {
        return;
    }
    ultimo.0 = ahora;
    let archivo = ruta_autoguardado(&ruta.0);
    if historial.0.sin_guardar(&proyecto.0) {
        if let Ok(texto) = proyecto.0.a_texto()
            && texto != ultimo.1
            && std::fs::write(&archivo, &texto).is_ok()
        {
            ultimo.1 = texto;
        }
    } else if archivo.exists() {
        let _ = std::fs::remove_file(&archivo);
        ultimo.1.clear();
    }
}

/// Recoge lo que se eligió en el selector de archivos (si ya se eligió) y lo aplica.
fn recibir_dialogo(
    mut dialogo: ResMut<DialogoProyecto>,
    mut proyecto: ResMut<ProyectoActual>,
    mut estado: ResMut<EstadoProyecto>,
    mut ruta: ResMut<RutaProyecto>,
    mut avisos: MessageWriter<ProyectoCargado>,
    mut historial: ResMut<HistorialProyecto>,
) {
    // Se mira primero sin pedir acceso de escritura: pedirlo marcaría el recurso como
    // modificado en cada frame.
    let Some(pendiente) = dialogo.pendiente.as_ref() else {
        return;
    };
    let pedido = pendiente.pedido;
    let respuesta = match pendiente.respuesta.lock() {
        Ok(receptor) => receptor.try_recv(),
        Err(_) => Err(TryRecvError::Disconnected),
    };
    let elegido = match respuesta {
        Err(TryRecvError::Empty) => return, // todavía no eligió
        Ok(elegido) => elegido,
        Err(TryRecvError::Disconnected) => None,
    };
    dialogo.pendiente = None;
    let Some(elegido) = elegido else {
        return; // canceló
    };

    estado.mensaje = match pedido {
        PedidoDialogo::Abrir => match Proyecto::cargar(&elegido) {
            // Al asignar, Bevy marca el recurso como modificado y las mallas se actualizan.
            Ok(p) => {
                ruta.0 = elegido;
                historial.0.reiniciar(&p);
                proyecto.0 = p;
                avisos.write(ProyectoCargado);
                format!("Abierto {}", ruta.0.display())
            }
            Err(e) => format!("No se pudo abrir: {e}"),
        },
        PedidoDialogo::GuardarComo => {
            let destino = con_extension_ron(elegido);
            // Las rutas de los videos son relativas a la carpeta del proyecto: al cambiar de
            // carpeta hay que reescribirlas para que sigan apuntando a los mismos archivos.
            let mut copia = proyecto.0.clone();
            let carpeta_nueva = destino
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .and_then(|p| std::fs::canonicalize(p).ok());
            if let Some(nueva) = &carpeta_nueva {
                copia.composicion.cambiar_carpeta(&ruta.carpeta(), nueva);
            }
            match copia.guardar(&destino) {
                Ok(()) => {
                    if copia.composicion != proyecto.0.composicion {
                        proyecto.0 = copia;
                    }
                    let _ = std::fs::remove_file(ruta_autoguardado(&ruta.0));
                    ruta.0 = destino;
                    historial.0.marcar_guardado(&proyecto.0);
                    format!("Guardado en {}", ruta.0.display())
                }
                Err(e) => format!("No se pudo guardar: {e}"),
            }
        }
    };
    info!("{}", estado.mensaje);
}

/// Agrega `.ron` si el nombre elegido no tiene extensión.
fn con_extension_ron(mut ruta: PathBuf) -> PathBuf {
    if ruta.extension().is_none() {
        ruta.set_extension("ron");
    }
    ruta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrega_la_extension_solo_si_falta() {
        assert_eq!(
            con_extension_ron(PathBuf::from("/a/show")),
            PathBuf::from("/a/show.ron")
        );
        assert_eq!(
            con_extension_ron(PathBuf::from("/a/show.ron")),
            PathBuf::from("/a/show.ron")
        );
        assert_eq!(
            con_extension_ron(PathBuf::from("/a/show.v2")),
            PathBuf::from("/a/show.v2")
        );
    }

    #[test]
    fn un_proyecto_en_la_carpeta_actual_gana() {
        assert_eq!(
            rutas_candidatas(true, Some("/home/m".into()), Some("/datos".into())),
            vec![PathBuf::from("proyecto.ron")]
        );
    }

    #[test]
    fn sin_proyecto_local_se_prefiere_proyectos_en_la_carpeta_personal() {
        assert_eq!(
            rutas_candidatas(false, Some("/home/m".into()), Some("/datos".into())),
            vec![
                PathBuf::from("/home/m/Proyectos/hypnotica/proyecto.ron"),
                PathBuf::from("/datos/Proyectos/hypnotica/proyecto.ron"),
                PathBuf::from("proyecto.ron"),
            ]
        );
    }

    #[test]
    fn sin_carpetas_conocidas_queda_la_actual() {
        assert_eq!(
            rutas_candidatas(false, None, None),
            vec![PathBuf::from("proyecto.ron")]
        );
    }
}
