//! Audio de las capturas de pantalla: el portal solo entrega imagen, así que el sonido se
//! toma aparte de PipeWire, o del escritorio entero o de una aplicación.
//!
//! Cada aplicación que suena es un *nodo* de PipeWire de clase `Stream/Output/Audio`.
//! Capturar un nodo concreto (por su número de serie) trae solo lo que esa aplicación
//! emite, sin el sonido de este programa.

use std::process::Command;

/// De dónde sale el sonido de una captura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioDeCaptura {
    /// Todo lo que suena en el escritorio (incluido este programa).
    Escritorio,
    /// Una aplicación, por su nombre (`application.name`).
    Aplicacion(String),
}

/// Una aplicación que está emitiendo sonido ahora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AplicacionDeAudio {
    pub nombre: String,
    /// El identificador del nodo en PipeWire. Cambia cada vez que la aplicación abre un
    /// flujo de sonido, por eso se guarda el nombre y no esto.
    pub serial: u64,
}

/// Binario de este programa: no se ofrece capturarse a sí mismo.
const NOMBRE_PROPIO: &str = "hypnotica";

/// Las aplicaciones que suenan ahora. Puede tardar un instante (ejecuta `pw-dump`): llamarla
/// desde un hilo aparte. Si PipeWire no está o `pw-dump` falla, devuelve una lista vacía.
pub fn listar_aplicaciones_de_audio() -> Vec<AplicacionDeAudio> {
    let Ok(salida) = Command::new("pw-dump").output() else {
        return Vec::new();
    };
    if !salida.status.success() {
        return Vec::new();
    }
    aplicaciones_en(&String::from_utf8_lossy(&salida.stdout))
}

/// El número de serie del flujo de la aplicación `nombre`, si está sonando. Si hay varios (un
/// navegador con varias pestañas), toma el primero.
pub fn serial_de_aplicacion(nombre: &str) -> Option<u64> {
    listar_aplicaciones_de_audio()
        .into_iter()
        .find(|a| a.nombre == nombre)
        .map(|a| a.serial)
}

/// Lee la salida de `pw-dump` (un arreglo JSON de objetos de PipeWire) y saca las
/// aplicaciones que emiten sonido, sin repetir nombres y sin este programa. Es lógica pura:
/// no ejecuta nada, así se prueba con texto de ejemplo.
pub fn aplicaciones_en(json: &str) -> Vec<AplicacionDeAudio> {
    let Ok(valor) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(objetos) = valor.as_array() else {
        return Vec::new();
    };
    let mut apps: Vec<AplicacionDeAudio> = Vec::new();
    for objeto in objetos {
        let props = &objeto["info"]["props"];
        if props["media.class"].as_str() != Some("Stream/Output/Audio") {
            continue;
        }
        let nombre = props["application.name"]
            .as_str()
            .or_else(|| props["node.name"].as_str());
        let binario = props["application.process.binary"].as_str();
        let Some(serial) = props["object.serial"].as_u64() else {
            continue;
        };
        let Some(nombre) = nombre else { continue };
        let es_propio = binario.is_some_and(|b| b.eq_ignore_ascii_case(NOMBRE_PROPIO))
            || nombre.eq_ignore_ascii_case(NOMBRE_PROPIO);
        if es_propio || apps.iter().any(|a| a.nombre == nombre) {
            continue;
        }
        apps.push(AplicacionDeAudio {
            nombre: nombre.to_string(),
            serial,
        });
    }
    apps.sort_by(|a, b| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un recorte de lo que imprime `pw-dump`: dos aplicaciones, una repetida, este programa,
    /// y objetos que no son flujos de sonido.
    const EJEMPLO: &str = r#"[
      {"id": 50, "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output"}}},
      {"id": 101, "info": {"props": {"media.class": "Stream/Output/Audio",
        "application.name": "Firefox", "object.serial": 607}}},
      {"id": 102, "info": {"props": {"media.class": "Stream/Output/Audio",
        "application.name": "Firefox", "object.serial": 650}}},
      {"id": 103, "info": {"props": {"media.class": "Stream/Output/Audio",
        "application.name": "mpv", "object.serial": 700}}},
      {"id": 104, "info": {"props": {"media.class": "Stream/Output/Audio",
        "application.name": "Hypnotica", "application.process.binary": "hypnotica",
        "object.serial": 701}}},
      {"id": 105, "info": {"props": {"media.class": "Stream/Input/Audio",
        "application.name": "Grabador", "object.serial": 702}}},
      {"id": 106, "info": {"props": {"media.class": "Stream/Output/Audio",
        "node.name": "sin_nombre_de_app", "object.serial": 703}}}
    ]"#;

    #[test]
    fn lista_las_aplicaciones_que_suenan_sin_repetir_y_sin_este_programa() {
        let nombres: Vec<_> = aplicaciones_en(EJEMPLO)
            .into_iter()
            .map(|a| (a.nombre, a.serial))
            .collect();
        assert_eq!(
            nombres,
            vec![
                ("Firefox".to_string(), 607), // el primero de los dos
                ("mpv".to_string(), 700),
                ("sin_nombre_de_app".to_string(), 703), // sin `application.name`: usa el nodo
            ]
        );
    }

    #[test]
    fn un_texto_que_no_es_json_da_lista_vacia() {
        assert!(aplicaciones_en("no es json").is_empty());
        assert!(aplicaciones_en("{}").is_empty());
        assert!(aplicaciones_en("[]").is_empty());
    }
}
