//! Envío de estado por OSC: le cuenta al controlador (un celular con TouchOSC, por ejemplo)
//! qué está pasando, para que sus botones y deslizadores reflejen el estado real.
//!
//! Solo se manda lo que CAMBIÓ desde la última vez (y todo de nuevo al cambiar el destino
//! o cargar otro proyecto), así no se llena la red de mensajes repetidos.
//!
//! Direcciones (todo desde 1, igual que las de entrada):
//! - `/hypnotica/capa/N/activo i`: la columna que suena en la capa N (0 = detenida).
//! - `/hypnotica/capa/N/pausa i`: 1 si está en pausa.
//! - `/hypnotica/capa/N/opacidad f`: su opacidad de 0 a 1.
//! - `/hypnotica/capa/N/visible i`: 1 si está visible.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;

use bevy::prelude::*;
use rosc::{OscMessage, OscPacket, OscType};
use vm_composicion::Reproduccion;
use vm_proyecto::ProyectoActual;

/// A dónde se manda el estado (`ip:puerto`). `None` es no mandar. Se recuerda entre sesiones,
/// aparte del proyecto: es del equipo.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DestinoOsc {
    pub direccion: Option<SocketAddr>,
}

impl DestinoOsc {
    pub fn a_texto(&self) -> String {
        match self.direccion {
            Some(d) => format!("destino={d}\n"),
            None => String::new(),
        }
    }

    /// Lo inverso. Algo que no es `ip:puerto` queda en «no mandar».
    pub fn desde_texto(texto: &str) -> Self {
        let direccion = texto
            .lines()
            .find_map(|l| l.strip_prefix("destino="))
            .and_then(|d| d.trim().parse().ok());
        DestinoOsc { direccion }
    }
}

/// Los mensajes que describen el estado de las capas. Es una función pura (no toca la red):
/// el sistema compara con lo último mandado y envía la diferencia.
pub fn mensajes_de_estado(
    activo: &[Option<usize>],
    pausada: &[bool],
    opacidades: &[f32],
    visibles: &[bool],
) -> Vec<(String, OscType)> {
    let mut m = Vec::new();
    for (i, opacidad) in opacidades.iter().enumerate() {
        let n = i + 1;
        let columna = activo.get(i).copied().flatten().map_or(0, |c| c as i32 + 1);
        let en_pausa = pausada.get(i).copied().unwrap_or(false);
        m.push((format!("/hypnotica/capa/{n}/activo"), OscType::Int(columna)));
        m.push((
            format!("/hypnotica/capa/{n}/pausa"),
            OscType::Int(i32::from(en_pausa)),
        ));
        m.push((
            format!("/hypnotica/capa/{n}/opacidad"),
            OscType::Float(*opacidad),
        ));
        m.push((
            format!("/hypnotica/capa/{n}/visible"),
            OscType::Int(i32::from(visibles.get(i).copied().unwrap_or(true))),
        ));
    }
    m
}

/// Manda por UDP a `destino` los mensajes de `nuevos` que difieren de `enviados`, y los anota.
fn enviar_diferencias(
    socket: &UdpSocket,
    destino: SocketAddr,
    nuevos: Vec<(String, OscType)>,
    enviados: &mut HashMap<String, OscType>,
) {
    for (addr, valor) in nuevos {
        if enviados.get(&addr) == Some(&valor) {
            continue;
        }
        let paquete = OscPacket::Message(OscMessage {
            addr: addr.clone(),
            args: vec![valor.clone()],
        });
        if let Ok(bytes) = rosc::encoder::encode(&paquete) {
            // UDP no avisa si el otro lado no está: un error acá no es grave.
            let _ = socket.send_to(&bytes, destino);
        }
        enviados.insert(addr, valor);
    }
}

pub(crate) struct EnvioPlugin;

impl Plugin for EnvioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(leer_configuracion().unwrap_or_default())
            .add_systems(Update, (enviar_estado, guardar_la_configuracion).chain());
    }
}

fn enviar_estado(
    destino: Res<DestinoOsc>,
    proyecto: Res<ProyectoActual>,
    reproduccion: Res<Reproduccion>,
    mut socket: Local<Option<UdpSocket>>,
    mut enviados: Local<HashMap<String, OscType>>,
    mut destino_anterior: Local<Option<SocketAddr>>,
) {
    let Some(direccion) = destino.direccion else {
        *destino_anterior = None;
        return;
    };
    // Cambió el destino: el nuevo no sabe nada, así que se manda todo de nuevo.
    if *destino_anterior != Some(direccion) {
        *destino_anterior = Some(direccion);
        enviados.clear();
        *socket = None;
    }
    if socket.is_none() {
        let local = if direccion.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        *socket = UdpSocket::bind(local).ok();
    }
    let Some(socket) = socket.as_ref() else {
        return;
    };
    let capas = &proyecto.0.composicion.capas;
    let opacidades: Vec<f32> = capas.iter().map(|c| c.opacidad).collect();
    let visibles: Vec<bool> = capas.iter().map(|c| c.visible).collect();
    let nuevos = mensajes_de_estado(
        &reproduccion.0.activo,
        &reproduccion.0.pausada,
        &opacidades,
        &visibles,
    );
    enviar_diferencias(socket, direccion, nuevos, &mut enviados);
}

fn guardar_la_configuracion(destino: Res<DestinoOsc>, mut primera_vez: Local<bool>) {
    if !*primera_vez {
        *primera_vez = true;
        return;
    }
    if destino.is_changed() {
        guardar_configuracion(&destino);
    }
}

fn ruta_de_la_configuracion() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hypnotica/osc_envio.txt"))
}

fn leer_configuracion() -> Option<DestinoOsc> {
    let texto = std::fs::read_to_string(ruta_de_la_configuracion()?).ok()?;
    Some(DestinoOsc::desde_texto(&texto))
}

fn guardar_configuracion(destino: &DestinoOsc) {
    let Some(ruta) = ruta_de_la_configuracion() else {
        return;
    };
    if let Some(carpeta) = ruta.parent() {
        let _ = std::fs::create_dir_all(carpeta);
    }
    if let Err(e) = std::fs::write(&ruta, destino.a_texto()) {
        warn!(
            "no se pudo guardar el destino de OSC en {}: {e}",
            ruta.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn el_destino_se_guarda_y_se_lee_en_texto() {
        let d = DestinoOsc {
            direccion: Some("192.168.1.20:9000".parse().unwrap()),
        };
        assert_eq!(DestinoOsc::desde_texto(&d.a_texto()), d);
        assert_eq!(
            DestinoOsc::desde_texto("destino=basura"),
            DestinoOsc::default()
        );
        assert_eq!(DestinoOsc::desde_texto(""), DestinoOsc::default());
    }

    #[test]
    fn el_estado_usa_numeros_desde_uno() {
        let m = mensajes_de_estado(
            &[Some(2), None],
            &[false, true],
            &[0.5, 1.0],
            &[true, false],
        );
        let buscar = |a: &str| m.iter().find(|(d, _)| d == a).map(|(_, v)| v.clone());
        assert_eq!(buscar("/hypnotica/capa/1/activo"), Some(OscType::Int(3)));
        assert_eq!(buscar("/hypnotica/capa/2/activo"), Some(OscType::Int(0)));
        assert_eq!(buscar("/hypnotica/capa/2/pausa"), Some(OscType::Int(1)));
        assert_eq!(
            buscar("/hypnotica/capa/1/opacidad"),
            Some(OscType::Float(0.5))
        );
        assert_eq!(buscar("/hypnotica/capa/2/visible"), Some(OscType::Int(0)));
    }

    /// Un receptor UDP de verdad: recibe todo la primera vez y solo lo que cambió después.
    #[test]
    fn solo_se_manda_lo_que_cambio() {
        let receptor = UdpSocket::bind("127.0.0.1:0").unwrap();
        receptor
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let destino = receptor.local_addr().unwrap();
        let emisor = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut enviados = HashMap::new();

        let contar = |receptor: &UdpSocket| {
            let mut buffer = [0u8; 1024];
            let mut n = 0;
            while receptor.recv_from(&mut buffer).is_ok() {
                n += 1;
            }
            n
        };

        let uno = mensajes_de_estado(&[Some(0)], &[false], &[1.0], &[true]);
        enviar_diferencias(&emisor, destino, uno.clone(), &mut enviados);
        assert_eq!(contar(&receptor), 4);

        // Nada cambió: nada se manda.
        enviar_diferencias(&emisor, destino, uno, &mut enviados);
        assert_eq!(contar(&receptor), 0);

        // Cambia solo la opacidad: llega un mensaje.
        let dos = mensajes_de_estado(&[Some(0)], &[false], &[0.25], &[true]);
        enviar_diferencias(&emisor, destino, dos, &mut enviados);
        assert_eq!(contar(&receptor), 1);
    }
}
