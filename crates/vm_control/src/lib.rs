//! Control externo: recibir órdenes desde un celular, una consola u otro programa. Hoy por
//! **OSC** (un mensaje por UDP); MIDI queda para después. Se usa la misma traducción
//! (`vm_core::interpretar`) para todo lo que llegue.
//!
//! Como en el resto del programa, lo que llega **modifica el modelo** (el proyecto) o manda una
//! orden de reproducción; nunca toca lo que se dibuja. El servidor está **apagado por
//! defecto**: mientras está prendido escucha en toda la red local, y cualquiera que llegue a
//! ese puerto puede disparar clips.

mod envio;
mod midi;

use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::prelude::*;
use rosc::{OscMessage, OscPacket, OscType};
use vm_composicion::OrdenClip;
use vm_core::{Argumento, OrdenComposicion, OrdenDeControl, interpretar};
use vm_proyecto::ProyectoActual;

/// El puerto en el que escucha Resolume, para que las plantillas de controladores sirvan
/// tal cual.
pub const PUERTO_POR_DEFECTO: u16 = 7000;

pub use envio::{DestinoOsc, mensajes_de_estado};
pub use midi::{
    AprendizajeMidi, ConexionMidi, ConfigMidi, EstadoMidi, MidiPlugin, PuertosMidi, listar_puertos,
};

pub struct ControlPlugin;

impl Plugin for ControlPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MidiPlugin, envio::EnvioPlugin))
            .insert_resource(leer_configuracion().unwrap_or_default())
            .init_resource::<EstadoOsc>()
            .init_resource::<ServidorActivo>()
            .add_systems(
                Update,
                (
                    sincronizar_servidor,
                    aplicar_ordenes_osc,
                    guardar_la_configuracion,
                )
                    .chain(),
            );
    }
}

/// Si el servidor OSC está prendido y en qué puerto. Se recuerda entre sesiones, aparte del
/// proyecto (es del equipo, no del trabajo).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConfigOsc {
    pub activo: bool,
    pub puerto: u16,
}

impl Default for ConfigOsc {
    fn default() -> Self {
        ConfigOsc {
            activo: false,
            puerto: PUERTO_POR_DEFECTO,
        }
    }
}

impl ConfigOsc {
    /// Cómo se guarda: dos líneas, `activo=1` y `puerto=7000`.
    pub fn a_texto(&self) -> String {
        format!("activo={}\npuerto={}\n", u8::from(self.activo), self.puerto)
    }

    /// Lo inverso. Lo que falta o no se entiende queda en su valor por defecto, y un puerto
    /// menor a 1024 (reservado) se descarta.
    pub fn desde_texto(texto: &str) -> Self {
        let mut config = ConfigOsc::default();
        for linea in texto.lines() {
            match linea.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                Some(("activo", v)) => config.activo = v == "1",
                Some(("puerto", v)) => {
                    if let Ok(p) = v.parse::<u16>()
                        && p >= 1024
                    {
                        config.puerto = p;
                    }
                }
                _ => {}
            }
        }
        config
    }
}

/// Lo que pasa con el servidor, para mostrarlo en la interfaz.
#[derive(Resource, Default)]
pub struct EstadoOsc {
    /// El puerto en el que se está escuchando de verdad (puede diferir del pedido si falló).
    pub escuchando: Option<u16>,
    /// Por qué no se pudo escuchar (por ejemplo, el puerto está ocupado).
    pub error: Option<String>,
    /// Cuántos mensajes llegaron desde que se prendió.
    pub recibidos: u64,
    /// El último mensaje y qué pasó con él.
    pub ultimo: Option<String>,
}

// ---------------------------------------------------------------------------------------
// El servidor
// ---------------------------------------------------------------------------------------

/// Un mensaje que llegó, ya interpretado.
struct Recibido {
    direccion: String,
    resultado: Result<Option<OrdenDeControl>, String>,
}

/// Un hilo que escucha UDP. Al soltarlo se apaga.
pub struct ServidorOsc {
    receptor: Mutex<Receiver<Recibido>>,
    terminar: Arc<AtomicBool>,
    hilo: Option<JoinHandle<()>>,
    puerto: u16,
}

impl ServidorOsc {
    /// Empieza a escuchar en `puerto` (con 0 el sistema elige uno libre: sirve en los tests).
    pub fn iniciar(puerto: u16) -> Result<ServidorOsc, String> {
        let socket = UdpSocket::bind(("0.0.0.0", puerto))
            .map_err(|e| format!("no se pudo escuchar en el puerto {puerto}: {e}"))?;
        // Se despierta cada tanto para ver si lo mandaron a terminar.
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| e.to_string())?;
        let puerto_real = socket.local_addr().map_err(|e| e.to_string())?.port();
        let (emisor, receptor) = channel();
        let terminar = Arc::new(AtomicBool::new(false));
        let senal = Arc::clone(&terminar);
        let hilo = std::thread::spawn(move || {
            // 64 KiB: lo máximo que cabe en un paquete UDP.
            let mut buffer = vec![0u8; 65_535];
            while !senal.load(Ordering::Relaxed) {
                let Ok((largo, _)) = socket.recv_from(&mut buffer) else {
                    continue; // venció el tiempo de espera, o un error pasajero
                };
                let Ok((_, paquete)) = rosc::decoder::decode_udp(&buffer[..largo]) else {
                    continue; // no era OSC
                };
                let mut mensajes = Vec::new();
                aplanar(paquete, &mut mensajes);
                for m in mensajes {
                    let argumentos: Vec<Argumento> =
                        m.args.iter().filter_map(argumento_de).collect();
                    let resultado = interpretar(&m.addr, &argumentos);
                    if emisor
                        .send(Recibido {
                            direccion: m.addr,
                            resultado,
                        })
                        .is_err()
                    {
                        return; // nadie escucha del otro lado
                    }
                }
            }
        });
        Ok(ServidorOsc {
            receptor: Mutex::new(receptor),
            terminar,
            hilo: Some(hilo),
            puerto: puerto_real,
        })
    }

    /// El puerto en el que escucha de verdad.
    pub fn puerto(&self) -> u16 {
        self.puerto
    }

    fn recibir(&self) -> Option<Recibido> {
        self.receptor.lock().ok()?.try_recv().ok()
    }
}

impl Drop for ServidorOsc {
    fn drop(&mut self) {
        self.terminar.store(true, Ordering::Relaxed);
        if let Some(hilo) = self.hilo.take() {
            // Se espera en otro hilo: puede tardar hasta 100 ms.
            std::thread::spawn(move || {
                let _ = hilo.join();
            });
        }
    }
}

/// Un paquete OSC puede ser un mensaje o un *bundle* (varios mensajes, a veces anidados): se
/// aplana a una lista de mensajes.
fn aplanar(paquete: OscPacket, salida: &mut Vec<OscMessage>) {
    match paquete {
        OscPacket::Message(m) => salida.push(m),
        OscPacket::Bundle(b) => {
            for contenido in b.content {
                aplanar(contenido, salida);
            }
        }
    }
}

/// Un argumento de OSC a uno del modelo. Los tipos que no sirven (colores, tiempos, blobs...)
/// se ignoran.
fn argumento_de(osc: &OscType) -> Option<Argumento> {
    match osc {
        OscType::Int(n) => Some(Argumento::Entero(*n)),
        OscType::Long(n) => Some(Argumento::Entero(i32::try_from(*n).ok()?)),
        OscType::Float(x) => Some(Argumento::Real(*x)),
        OscType::Double(x) => Some(Argumento::Real(*x as f32)),
        OscType::String(s) => Some(Argumento::Texto(s.clone())),
        OscType::Bool(b) => Some(Argumento::Booleano(*b)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// Sistemas de Bevy
// ---------------------------------------------------------------------------------------

/// El servidor que está corriendo, si lo hay.
#[derive(Resource, Default)]
pub struct ServidorActivo(Option<ServidorOsc>);

/// Prende, apaga o reinicia el servidor cuando cambia la configuración.
fn sincronizar_servidor(
    config: Res<ConfigOsc>,
    mut servidor: ResMut<ServidorActivo>,
    mut estado: ResMut<EstadoOsc>,
    mut aplicada: Local<Option<ConfigOsc>>,
) {
    if *aplicada == Some(*config) {
        return;
    }
    *aplicada = Some(*config);
    servidor.0 = None; // el anterior se apaga solo al soltarlo
    estado.escuchando = None;
    estado.error = None;
    estado.recibidos = 0;
    estado.ultimo = None;
    if !config.activo {
        return;
    }
    match ServidorOsc::iniciar(config.puerto) {
        Ok(s) => {
            info!("OSC: escuchando en el puerto {}", s.puerto());
            estado.escuchando = Some(s.puerto());
            servidor.0 = Some(s);
        }
        Err(e) => {
            warn!("OSC: {e}");
            estado.error = Some(e);
        }
    }
}

/// Aplica lo que llegó. Las órdenes de reproducción se mandan como cualquier otra (un
/// `OrdenClip`); lo demás edita el proyecto.
fn aplicar_ordenes_osc(
    servidor: Res<ServidorActivo>,
    mut estado: ResMut<EstadoOsc>,
    mut ordenes: MessageWriter<OrdenClip>,
    mut proyecto: ResMut<ProyectoActual>,
) {
    let Some(servidor) = servidor.0.as_ref() else {
        return;
    };
    // Se procesa un máximo por frame: una avalancha de mensajes no traba la interfaz.
    for _ in 0..512 {
        let Some(recibido) = servidor.recibir() else {
            break;
        };
        estado.recibidos += 1;
        let texto = match recibido.resultado {
            Ok(Some(orden)) => aplicar(&orden, &mut proyecto, &mut ordenes),
            Ok(None) => "sin efecto".to_string(),
            Err(motivo) => {
                debug!("OSC: {motivo}");
                motivo
            }
        };
        estado.ultimo = Some(format!("{} → {texto}", recibido.direccion));
    }
}

/// Aplica una orden y dice qué hizo (o por qué no). Es la misma para OSC y para MIDI.
pub(crate) fn aplicar(
    orden: &OrdenDeControl,
    proyecto: &mut ResMut<ProyectoActual>,
    ordenes: &mut MessageWriter<OrdenClip>,
) -> String {
    let comp = &proyecto.0.composicion;
    let (capas, columnas) = (comp.capas.len(), comp.columnas.len());
    let capa_ok = |capa: usize| capa < capas;
    let fuera = |que: &str, n: usize| format!("no hay {que} {}", n + 1);

    match *orden {
        OrdenDeControl::Clip { capa, columna } => {
            if !capa_ok(capa) {
                return fuera("capa", capa);
            }
            if columna >= columnas {
                return fuera("columna", columna);
            }
            info!(
                "OSC: clip de la capa {} y la columna {}",
                capa + 1,
                columna + 1
            );
            ordenes.write(OrdenClip(OrdenComposicion::Disparar { capa, columna }));
            "disparado".into()
        }
        OrdenDeControl::Columna(columna) => {
            if columna >= columnas {
                return fuera("columna", columna);
            }
            info!("OSC: columna {}", columna + 1);
            ordenes.write(OrdenClip(OrdenComposicion::DispararColumna(columna)));
            "columna disparada".into()
        }
        OrdenDeControl::Detener { capa } => {
            if !capa_ok(capa) {
                return fuera("capa", capa);
            }
            info!("OSC: detener la capa {}", capa + 1);
            ordenes.write(OrdenClip(OrdenComposicion::Detener { capa }));
            "detenida".into()
        }
        OrdenDeControl::DetenerTodo => {
            info!("OSC: detener todo");
            ordenes.write(OrdenClip(OrdenComposicion::DetenerTodo));
            "todo detenido".into()
        }
        OrdenDeControl::AlternarPausa { capa } => {
            if !capa_ok(capa) {
                return fuera("capa", capa);
            }
            ordenes.write(OrdenClip(OrdenComposicion::AlternarPausa { capa }));
            "pausa".into()
        }
        // Lo que sigue edita el proyecto. Se compara antes de escribir: pedir escritura
        // marcaría el proyecto como modificado aunque no cambie nada (y un fader que manda el
        // mismo valor veinte veces por segundo llenaría el historial de deshacer).
        OrdenDeControl::Opacidad { capa, valor } => {
            if !capa_ok(capa) {
                return fuera("capa", capa);
            }
            if proyecto.0.composicion.capas[capa].opacidad != valor {
                proyecto.0.composicion.capas[capa].opacidad = valor;
            }
            format!("opacidad {valor:.2}")
        }
        OrdenDeControl::Visible { capa, visible } => {
            if !capa_ok(capa) {
                return fuera("capa", capa);
            }
            if proyecto.0.composicion.capas[capa].visible != visible {
                proyecto.0.composicion.capas[capa].visible = visible;
            }
            if visible { "visible" } else { "oculta" }.into()
        }
        OrdenDeControl::Transicion(segundos) => {
            if proyecto.0.composicion.transicion != segundos {
                proyecto.0.composicion.transicion = segundos;
            }
            format!("fundido {segundos:.1} s")
        }
    }
}

/// Guarda la configuración cuando cambia (no en el primer frame, que es la que se leyó).
fn guardar_la_configuracion(config: Res<ConfigOsc>, mut primera_vez: Local<bool>) {
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
    Some(base.join("hypnotica/osc.txt"))
}

fn leer_configuracion() -> Option<ConfigOsc> {
    let texto = std::fs::read_to_string(ruta_de_la_configuracion()?).ok()?;
    Some(ConfigOsc::desde_texto(&texto))
}

fn guardar_configuracion(config: &ConfigOsc) {
    let Some(ruta) = ruta_de_la_configuracion() else {
        return;
    };
    if let Some(carpeta) = ruta.parent() {
        let _ = std::fs::create_dir_all(carpeta);
    }
    if let Err(e) = std::fs::write(&ruta, config.a_texto()) {
        warn!(
            "no se pudo guardar la configuración de OSC en {}: {e}",
            ruta.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscTime};
    use std::time::Instant;

    #[test]
    fn la_configuracion_se_guarda_y_se_lee_en_texto() {
        for config in [
            ConfigOsc::default(),
            ConfigOsc {
                activo: true,
                puerto: 9000,
            },
        ] {
            assert_eq!(ConfigOsc::desde_texto(&config.a_texto()), config);
        }
    }

    #[test]
    fn una_configuracion_ilegible_o_con_un_puerto_reservado_usa_los_valores_por_defecto() {
        assert_eq!(ConfigOsc::desde_texto(""), ConfigOsc::default());
        assert_eq!(ConfigOsc::desde_texto("basura\n???"), ConfigOsc::default());
        // Un puerto reservado (menor a 1024) se descarta, pero lo demás se respeta.
        let c = ConfigOsc::desde_texto("activo=1\npuerto=80\n");
        assert!(c.activo);
        assert_eq!(c.puerto, PUERTO_POR_DEFECTO);
        assert_eq!(
            ConfigOsc::desde_texto("puerto=99999").puerto,
            PUERTO_POR_DEFECTO
        );
    }

    /// Manda un paquete OSC por UDP al servidor.
    fn mandar(puerto: u16, paquete: &OscPacket) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let bytes = rosc::encoder::encode(paquete).unwrap();
        socket.send_to(&bytes, ("127.0.0.1", puerto)).unwrap();
    }

    fn mensaje(direccion: &str, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: direccion.into(),
            args,
        })
    }

    /// Espera hasta `n` mensajes (o 2 segundos).
    fn esperar(servidor: &ServidorOsc, n: usize) -> Vec<Recibido> {
        let limite = Instant::now() + Duration::from_secs(2);
        let mut recibidos = Vec::new();
        while recibidos.len() < n && Instant::now() < limite {
            match servidor.recibir() {
                Some(r) => recibidos.push(r),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        recibidos
    }

    #[test]
    fn un_mensaje_por_udp_llega_interpretado() {
        let servidor = ServidorOsc::iniciar(0).unwrap();
        let puerto = servidor.puerto();
        assert!(puerto >= 1024);
        mandar(
            puerto,
            &mensaje(
                "/composition/layers/2/clips/3/connect",
                vec![OscType::Int(1)],
            ),
        );
        let r = esperar(&servidor, 1);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].direccion, "/composition/layers/2/clips/3/connect");
        assert_eq!(
            r[0].resultado,
            Ok(Some(OrdenDeControl::Clip {
                capa: 1,
                columna: 2
            }))
        );
    }

    #[test]
    fn un_bundle_trae_varios_mensajes_incluso_anidados() {
        let servidor = ServidorOsc::iniciar(0).unwrap();
        let interno = OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 1,
            },
            content: vec![mensaje("/hypnotica/columna/2", vec![])],
        });
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 1,
            },
            content: vec![
                mensaje("/hypnotica/capa/1/opacidad", vec![OscType::Float(0.5)]),
                interno,
            ],
        });
        mandar(servidor.puerto(), &bundle);
        let r = esperar(&servidor, 2);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].resultado,
            Ok(Some(OrdenDeControl::Opacidad {
                capa: 0,
                valor: 0.5
            }))
        );
        assert_eq!(r[1].resultado, Ok(Some(OrdenDeControl::Columna(1))));
    }

    #[test]
    fn lo_que_no_es_osc_o_no_se_entiende_no_tira_abajo_el_servidor() {
        let servidor = ServidorOsc::iniciar(0).unwrap();
        // Basura que no es OSC: se ignora sin más.
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .send_to(b"esto no es osc", ("127.0.0.1", servidor.puerto()))
            .unwrap();
        // Un mensaje de una dirección desconocida: llega con su error.
        mandar(servidor.puerto(), &mensaje("/otra/cosa", vec![]));
        // Y el servidor sigue andando.
        mandar(servidor.puerto(), &mensaje("/hypnotica/detener", vec![]));
        let r = esperar(&servidor, 2);
        assert_eq!(r.len(), 2);
        assert!(r[0].resultado.is_err());
        assert_eq!(r[1].resultado, Ok(Some(OrdenDeControl::DetenerTodo)));
    }

    #[test]
    fn un_puerto_ocupado_da_un_error_claro() {
        let primero = ServidorOsc::iniciar(0).unwrap();
        let error = ServidorOsc::iniciar(primero.puerto()).err().unwrap();
        assert!(error.contains("puerto"), "{error}");
    }

    #[test]
    fn los_argumentos_de_osc_se_traducen() {
        assert_eq!(argumento_de(&OscType::Int(3)), Some(Argumento::Entero(3)));
        assert_eq!(
            argumento_de(&OscType::Float(0.5)),
            Some(Argumento::Real(0.5))
        );
        assert_eq!(
            argumento_de(&OscType::Bool(true)),
            Some(Argumento::Booleano(true))
        );
        assert_eq!(argumento_de(&OscType::Nil), None);
        // Un entero largo que no entra en 32 bits se descarta en vez de truncarse.
        assert_eq!(argumento_de(&OscType::Long(i64::MAX)), None);
    }
}
