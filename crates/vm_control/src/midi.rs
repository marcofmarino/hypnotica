//! Entrada MIDI: conectarse a un controlador, y asignar sus botones y perillas con «MIDI learn».
//!
//! La traducción de bytes a órdenes es lógica pura de `vm_core` (`interpretar_midi`,
//! `resolver_midi`); acá solo se abre el puerto (con `midir`, que en Linux usa ALSA) y se
//! aplican las órdenes con el mismo código que OSC.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::prelude::*;
use midir::{MidiInput, MidiInputConnection};
use vm_composicion::OrdenClip;
use vm_core::{
    AsignacionMidi, DestinoDeControl, EntradaMidi, asignar, interpretar_midi, resolver_midi,
};
use vm_proyecto::ProyectoActual;

use crate::aplicar;

/// El nombre con el que Hypnotica se presenta ante ALSA.
const NOMBRE_DEL_CLIENTE: &str = "Hypnotica";

/// Qué puerto MIDI se escucha (por su nombre). `None` es ninguno. Se recuerda entre sesiones,
/// aparte del proyecto: el controlador es del equipo; lo que maneja cada control, en cambio,
/// va en el proyecto.
#[derive(Resource, Clone, PartialEq, Eq, Debug, Default)]
pub struct ConfigMidi {
    pub puerto: Option<String>,
}

impl ConfigMidi {
    pub fn a_texto(&self) -> String {
        match &self.puerto {
            Some(p) => format!("puerto={p}\n"),
            None => String::new(),
        }
    }

    pub fn desde_texto(texto: &str) -> Self {
        let puerto = texto
            .lines()
            .find_map(|l| l.strip_prefix("puerto="))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        ConfigMidi { puerto }
    }
}

/// Lo que pasa con la conexión, para mostrarlo en la interfaz.
#[derive(Resource, Default)]
pub struct EstadoMidi {
    /// El puerto al que se está conectado.
    pub conectado: Option<String>,
    pub error: Option<String>,
    pub recibidos: u64,
    /// El último control que se movió.
    pub ultimo: Option<String>,
}

/// Los puertos MIDI que hay ahora (controladores conectados).
#[derive(Resource, Default)]
pub struct PuertosMidi {
    pub lista: Vec<String>,
    /// Ya se buscó alguna vez: la primera vez que se necesita se llena sola.
    pub buscados: bool,
}

impl PuertosMidi {
    /// Vuelve a buscar. Es rápido (unos milisegundos), así que no hace falta un hilo.
    pub fn actualizar(&mut self) {
        self.buscados = true;
        self.lista = listar_puertos();
    }
}

/// Qué destino se está aprendiendo: el próximo mensaje MIDI que llegue se asigna a él.
#[derive(Resource, Default)]
pub struct AprendizajeMidi {
    pub destino: Option<DestinoDeControl>,
}

/// Los nombres de los puertos MIDI de entrada que hay.
pub fn listar_puertos() -> Vec<String> {
    let Ok(entrada) = MidiInput::new(NOMBRE_DEL_CLIENTE) else {
        return Vec::new();
    };
    entrada
        .ports()
        .iter()
        .filter_map(|p| entrada.port_name(p).ok())
        .collect()
}

/// Una conexión abierta: mientras exista, los mensajes llegan por `receptor`.
pub struct ConexionMidi {
    // En un `Mutex` solo para que el tipo sea compartible entre hilos (lo exige Bevy); nunca se usa.
    _conexion: Mutex<MidiInputConnection<()>>,
    receptor: Mutex<Receiver<(EntradaMidi, u8)>>,
    pub nombre: String,
}

impl ConexionMidi {
    /// Se conecta al puerto con ese nombre.
    pub fn abrir(nombre: &str) -> Result<ConexionMidi, String> {
        let entrada = MidiInput::new(NOMBRE_DEL_CLIENTE).map_err(|e| e.to_string())?;
        let puerto = entrada
            .ports()
            .into_iter()
            .find(|p| entrada.port_name(p).is_ok_and(|n| n == nombre))
            .ok_or_else(|| format!("no está el puerto MIDI «{nombre}»"))?;
        let (emisor, receptor) = channel();
        let conexion = entrada
            .connect(
                &puerto,
                "entrada",
                move |_, bytes, _| {
                    // La traducción es pura: acá solo se reenvía lo que se entiende.
                    if let Some(mensaje) = interpretar_midi(bytes) {
                        let _ = emisor.send(mensaje);
                    }
                },
                (),
            )
            .map_err(|e| format!("no se pudo conectar a «{nombre}»: {e}"))?;
        Ok(ConexionMidi {
            _conexion: Mutex::new(conexion),
            receptor: Mutex::new(receptor),
            nombre: nombre.to_string(),
        })
    }

    /// El siguiente mensaje, si llegó.
    pub fn recibir(&self) -> Option<(EntradaMidi, u8)> {
        self.receptor.lock().ok()?.try_recv().ok()
    }
}

/// La conexión que está abierta, si la hay.
#[derive(Resource, Default)]
pub struct ConexionActiva(Option<ConexionMidi>);

pub struct MidiPlugin;

impl Plugin for MidiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(leer_configuracion().unwrap_or_default())
            .init_resource::<EstadoMidi>()
            .init_resource::<PuertosMidi>()
            .init_resource::<AprendizajeMidi>()
            .init_resource::<ConexionActiva>()
            .add_systems(
                Update,
                (sincronizar_conexion, aplicar_midi, guardar_la_configuracion).chain(),
            );
    }
}

/// Conecta, desconecta o cambia de puerto cuando cambia la configuración. Si el puerto elegido
/// no está (el controlador está desenchufado), se vuelve a intentar cada 2 segundos.
fn sincronizar_conexion(
    config: Res<ConfigMidi>,
    mut activa: ResMut<ConexionActiva>,
    mut estado: ResMut<EstadoMidi>,
    reloj: Res<Time>,
    mut aplicada: Local<Option<ConfigMidi>>,
    mut proximo_intento: Local<f64>,
) {
    let cambio = aplicada.as_ref() != Some(&*config);
    let reintento = activa.0.is_none()
        && config.puerto.is_some()
        && estado.error.is_some()
        && reloj.elapsed_secs_f64() >= *proximo_intento;
    if !cambio && !reintento {
        return;
    }
    *aplicada = Some(config.clone());
    *proximo_intento = reloj.elapsed_secs_f64() + 2.0;
    if cambio {
        activa.0 = None; // se desconecta solo al soltarla
        estado.conectado = None;
        estado.error = None;
        estado.recibidos = 0;
        estado.ultimo = None;
    }
    let Some(nombre) = &config.puerto else {
        return;
    };
    match ConexionMidi::abrir(nombre) {
        Ok(conexion) => {
            info!("MIDI: conectado a «{nombre}»");
            estado.conectado = Some(nombre.clone());
            estado.error = None;
            activa.0 = Some(conexion);
        }
        Err(e) => {
            if estado.error.as_deref() != Some(e.as_str()) {
                warn!("MIDI: {e}");
                estado.error = Some(e);
            }
        }
    }
}

/// Aplica lo que llegó del controlador. Si se está aprendiendo, el primer mensaje se asigna al
/// destino elegido; si no, se resuelve con las asignaciones del proyecto.
fn aplicar_midi(
    activa: Res<ConexionActiva>,
    mut estado: ResMut<EstadoMidi>,
    mut aprendizaje: ResMut<AprendizajeMidi>,
    mut ordenes: MessageWriter<OrdenClip>,
    mut proyecto: ResMut<ProyectoActual>,
) {
    let Some(conexion) = activa.0.as_ref() else {
        return;
    };
    // Un máximo por frame: una perilla que gira rápido no traba la interfaz.
    for _ in 0..512 {
        let Some((entrada, valor)) = conexion.recibir() else {
            break;
        };
        estado.recibidos += 1;
        estado.ultimo = Some(format!("{} = {valor}", entrada.descripcion()));

        if let Some(destino) = aprendizaje.destino {
            // Se aprende con una pulsación, no con el «soltar» de un botón (valor 0).
            if valor > 0 {
                info!(
                    "MIDI: {} asignado a «{}»",
                    entrada.descripcion(),
                    destino.descripcion()
                );
                asignar(
                    &mut proyecto.0.asignaciones_midi,
                    AsignacionMidi { entrada, destino },
                );
                aprendizaje.destino = None;
            }
            continue;
        }
        // Se resuelve sin tocar el proyecto (`resolver_midi` solo lee) y se aplica después.
        for orden in resolver_midi(&proyecto.0.asignaciones_midi, entrada, valor) {
            aplicar(&orden, &mut proyecto, &mut ordenes);
        }
    }
}

fn guardar_la_configuracion(config: Res<ConfigMidi>, mut primera_vez: Local<bool>) {
    if !*primera_vez {
        *primera_vez = true;
        return;
    }
    if config.is_changed() {
        guardar_configuracion(&config);
    }
}

fn ruta_de_la_configuracion() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hypnotica/midi.txt"))
}

fn leer_configuracion() -> Option<ConfigMidi> {
    let texto = std::fs::read_to_string(ruta_de_la_configuracion()?).ok()?;
    Some(ConfigMidi::desde_texto(&texto))
}

fn guardar_configuracion(config: &ConfigMidi) {
    let Some(ruta) = ruta_de_la_configuracion() else {
        return;
    };
    if let Some(carpeta) = ruta.parent() {
        let _ = std::fs::create_dir_all(carpeta);
    }
    if let Err(e) = std::fs::write(&ruta, config.a_texto()) {
        warn!(
            "no se pudo guardar la configuración de MIDI en {}: {e}",
            ruta.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midir::MidiOutput;
    use std::time::{Duration, Instant};
    use vm_core::TipoMidi;

    #[test]
    fn el_puerto_elegido_se_guarda_y_se_lee_en_texto() {
        for config in [
            ConfigMidi::default(),
            ConfigMidi {
                puerto: Some("Launchpad Mini 24:0".into()),
            },
        ] {
            assert_eq!(ConfigMidi::desde_texto(&config.a_texto()), config);
        }
        // Un archivo vacío o ilegible es «ningún puerto».
        assert_eq!(ConfigMidi::desde_texto("basura"), ConfigMidi::default());
        assert_eq!(
            ConfigMidi::desde_texto("puerto=  \n"),
            ConfigMidi::default()
        );
    }

    /// Espera hasta `n` mensajes (o 2 segundos).
    fn esperar(conexion: &ConexionMidi, n: usize) -> Vec<(EntradaMidi, u8)> {
        let limite = Instant::now() + Duration::from_secs(2);
        let mut recibidos = Vec::new();
        while recibidos.len() < n && Instant::now() < limite {
            match conexion.recibir() {
                Some(m) => recibidos.push(m),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        recibidos
    }

    /// Prueba con ALSA de verdad, sin ningún controlador físico: se crea un puerto de salida
    /// virtual (hace de controlador), se conecta la entrada a él y se mandan mensajes. Se
    /// corre a mano: `cargo test -p vm_control midi -- --ignored`
    #[test]
    #[ignore = "necesita el secuenciador de ALSA"]
    fn un_controlador_virtual_manda_notas_y_controles() {
        use midir::os::unix::VirtualOutput;
        let salida = MidiOutput::new("controlador de prueba").unwrap();
        let mut controlador = salida
            .create_virtual("ControladorDePruebaHypnotica")
            .unwrap();

        // El puerto aparece en la lista como cualquier controlador.
        let nombre = listar_puertos()
            .into_iter()
            .find(|n| n.contains("ControladorDePruebaHypnotica"))
            .expect("el puerto virtual no aparece en la lista");
        let conexion = ConexionMidi::abrir(&nombre).unwrap();

        controlador.send(&[0x90, 36, 100]).unwrap(); // nota 36 encendida
        controlador.send(&[0x90, 36, 0]).unwrap(); // y soltada
        controlador.send(&[0xB0, 7, 127]).unwrap(); // control 7 a fondo
        controlador.send(&[0xC0, 3]).unwrap(); // un cambio de programa: se ignora

        let r = esperar(&conexion, 3);
        assert_eq!(r.len(), 3, "{r:?}");
        let nota = EntradaMidi {
            canal: 0,
            tipo: TipoMidi::Nota,
            numero: 36,
        };
        assert_eq!(r[0], (nota, 100));
        assert_eq!(r[1], (nota, 0));
        assert_eq!(
            r[2],
            (
                EntradaMidi {
                    canal: 0,
                    tipo: TipoMidi::Control,
                    numero: 7
                },
                127
            )
        );
        // Y el cambio de programa nunca llegó.
        assert!(esperar(&conexion, 1).is_empty());
    }

    #[test]
    fn conectarse_a_un_puerto_que_no_existe_da_un_error_claro() {
        let e = ConexionMidi::abrir("Puerto Que No Existe 99:9")
            .err()
            .expect("debía fallar");
        assert!(e.contains("Puerto Que No Existe"), "{e}");
    }
}
