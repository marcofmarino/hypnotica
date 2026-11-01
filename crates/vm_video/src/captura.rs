//! Captura de pantalla o de una ventana con el **portal XDG** (`ScreenCast`) y PipeWire.
//!
//! Los portales son la forma estándar, en los escritorios modernos, de pedirle permiso al
//! usuario para ver su pantalla: el escritorio (Hyprland, GNOME, KDE, Sway...) muestra su
//! propio selector y devuelve un "nodo" de PipeWire con la imagen. Este módulo hace ese
//! intercambio y deja listo lo que necesita GStreamer (`pipewiresrc`).
//!
//! El portal puede devolver un *token de restauración*: con él, la próxima vez la misma
//! fuente se vuelve a abrir sin preguntar. Cada sesión puede devolver un token nuevo, que
//! hay que guardar para la vez siguiente.

use std::os::fd::{AsRawFd, OwnedFd, RawFd};

use ashpd::desktop::PersistMode;
use ashpd::desktop::Session;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};

/// Una captura en marcha. Mientras exista este valor, la sesión del portal sigue abierta y
/// el escritorio muestra que se está compartiendo la pantalla; al soltarlo se cierra.
pub struct SesionDeCaptura {
    fd: OwnedFd,
    /// El nodo de PipeWire donde llega la imagen.
    pub nodo: u32,
    /// El token para volver a abrir esta misma fuente sin preguntar (si el portal lo da).
    pub token: Option<String>,
    /// El tamaño de lo capturado, si el portal lo informa.
    pub tamano: Option<(i32, i32)>,
    sesion: Option<Session<Screencast>>,
}

impl SesionDeCaptura {
    /// El descriptor de archivo de la conexión a PipeWire (lo usa `pipewiresrc`).
    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Drop for SesionDeCaptura {
    fn drop(&mut self) {
        // Cerrar la sesión es asíncrono: se hace en otro hilo para no trabar a quien la suelta.
        if let Some(sesion) = self.sesion.take() {
            std::thread::spawn(move || {
                let _ = pollster::block_on(sesion.close());
            });
        }
    }
}

/// Abre una sesión de captura. Con `token`, intenta reabrir la fuente guardada sin
/// preguntar; si no hay token (o ya no vale), el escritorio muestra su selector de pantalla
/// o ventana y **espera a que el usuario elija**: bloquea hasta entonces, así que hay que
/// llamarla desde un hilo aparte. El error es un texto para mostrar.
///
/// Con `ocultar_cursor` el cursor del mouse no se dibuja dentro de la imagen.
pub fn iniciar(token: Option<&str>, ocultar_cursor: bool) -> Result<SesionDeCaptura, String> {
    pollster::block_on(async {
        let proxy = Screencast::new().await.map_err(explicar)?;
        let sesion = proxy
            .create_session(Default::default())
            .await
            .map_err(explicar)?;
        proxy
            .select_sources(
                &sesion,
                SelectSourcesOptions::default()
                    // El cursor, o se dibuja dentro de la imagen o no se dibuja.
                    .set_cursor_mode(if ocultar_cursor {
                        CursorMode::Hidden
                    } else {
                        CursorMode::Embedded
                    })
                    .set_sources(SourceType::Monitor | SourceType::Window)
                    .set_multiple(false)
                    .set_restore_token(token)
                    // Que la elección se recuerde hasta que el usuario la revoque.
                    .set_persist_mode(PersistMode::ExplicitlyRevoked),
            )
            .await
            .map_err(explicar)?;
        let respuesta = proxy
            .start(&sesion, None, Default::default())
            .await
            .map_err(explicar)?
            .response()
            .map_err(explicar)?;
        let flujo = respuesta
            .streams()
            .first()
            .ok_or("no se eligió ninguna pantalla ni ventana")?;
        let fd = proxy
            .open_pipe_wire_remote(&sesion, Default::default())
            .await
            .map_err(explicar)?;
        Ok(SesionDeCaptura {
            fd,
            nodo: flujo.pipe_wire_node_id(),
            token: respuesta.restore_token().map(str::to_owned),
            tamano: flujo.size(),
            sesion: Some(sesion),
        })
    })
}

/// Un error del portal, en palabras.
fn explicar(e: ashpd::Error) -> String {
    let texto = e.to_string();
    if texto.to_lowercase().contains("cancel") {
        "se canceló la captura".into()
    } else if texto.contains("ServiceUnknown") || texto.contains("not provided") {
        "este escritorio no tiene el portal de captura de pantalla (xdg-desktop-portal)".into()
    } else {
        texto
    }
}
