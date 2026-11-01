//! Reproductores de video como componentes de Bevy: cada uno lleva los frames de su
//! video a una textura de la GPU.

use std::error::Error;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use std::sync::mpsc::TryRecvError;

use crate::audio::AudioDeCaptura;
use crate::captura::SesionDeCaptura;
use crate::decodificador::{Ajustes, Frame, Origen, Video};
use crate::dmabuf::CapturaDmaBuf;
use crate::importar::{ImportadorPlugin, RanurasDmaBuf, dmabuf_deshabilitado, formato_de_imagen};

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ImportadorPlugin)
            .add_systems(Update, actualizar_reproductores);
    }
}

/// Un reproductor de video: abre un archivo y deja sus frames en `textura`.
///
/// Se agrega como componente a una entidad (por ejemplo, una capa de la composición).
/// Quien lo use debe mirar `textura` cada frame: **cuando cambia la resolución del video
/// el handle cambia** (se crea una imagen nueva; una imagen que cambia de tamaño puede
/// dejar cachés de Bevy apuntando a la textura vieja).
#[derive(Component)]
pub struct Reproductor {
    /// El video que se está mostrando.
    actual: Option<Video>,
    /// El video que se está abriendo. Hasta que entregue su primer frame, el `actual` sigue
    /// mostrándose: así el cambio de clip no deja un parpadeo en negro.
    pendiente: Option<Video>,
    /// Pedido de dejar la textura transparente (se atiende en el próximo frame).
    limpiar: bool,
    /// Un error que todavía nadie leyó (ver `tomar_error`).
    error: Option<String>,
    /// Algo que salió mal pero **no detiene** lo que ya se estaba mostrando: por ejemplo, se
    /// canceló el selector al elegir otra captura y la anterior sigue (ver `tomar_aviso`).
    aviso: Option<String>,
    /// Los últimos ajustes de reproducción que se le dieron al video (para saber si cambiaron).
    ajustes: Option<Ajustes>,
    /// Una captura de pantalla que espera la respuesta del portal (el usuario puede estar
    /// eligiendo qué compartir).
    captura_en_curso: Option<EsperaDeCaptura>,
    /// Un token de restauración nuevo del portal que todavía nadie guardó.
    token_nuevo: Option<String>,
    /// El clip que se está yendo: sigue reproduciéndose, con su propia textura, mientras dura
    /// el fundido hacia el nuevo (ver `fijar_transicion`).
    saliente: Option<Saliente>,
    /// Segundos que dura el fundido al cambiar de clip. `0` es un corte seco.
    duracion_de_transicion: f32,
    /// Cuándo empezó el fundido (reloj de Bevy, en segundos).
    inicio_de_transicion: f64,
    /// Cuánto del clip nuevo se ve (de 0 a 1). 1 cuando no hay fundido.
    avance: f32,
    /// La captura por DMA-BUF en marcha, si la hay: los cuadros llegan como buffers de GPU y
    /// no pasan por la CPU. Es una alternativa a `actual` (nunca están las dos).
    dma: Option<CapturaDmaBuf>,
    /// Con qué se creó la imagen destino de la captura DMA-BUF (ancho, alto y formato DRM),
    /// para saber cuándo hay que recrearla. `None` hasta que llega el primer cuadro.
    dma_imagen: Option<(u32, u32, String)>,
    /// `false` después de un fallo del DMA-BUF: las capturas usan siempre el camino por CPU.
    permitir_dmabuf: bool,
    /// Cómo se pidió la última captura, para volver a abrirla por CPU si falla el DMA-BUF.
    ultima_captura: Option<UltimaCaptura>,
    /// La textura con el último frame. Empieza como un píxel transparente.
    pub textura: Handle<Image>,
}

/// Cómo se abrió una captura (se guarda para poder reabrirla por otro camino).
#[derive(Clone)]
struct UltimaCaptura {
    token: Option<String>,
    ajustes: Ajustes,
    limite: Option<(u32, u32)>,
    audio: Option<AudioDeCaptura>,
    ocultar_cursor: bool,
}

/// Un clip que sale durante un fundido.
struct Saliente {
    video: Video,
    textura: Handle<Image>,
}

impl Reproductor {
    /// Un reproductor detenido, con una textura transparente de 1×1.
    pub fn nuevo(imagenes: &mut Assets<Image>) -> Self {
        Reproductor {
            actual: None,
            pendiente: None,
            limpiar: false,
            error: None,
            aviso: None,
            ajustes: None,
            captura_en_curso: None,
            token_nuevo: None,
            saliente: None,
            duracion_de_transicion: 0.0,
            inicio_de_transicion: 0.0,
            avance: 1.0,
            dma: None,
            dma_imagen: None,
            permitir_dmabuf: true,
            ultima_captura: None,
            textura: imagenes.add(imagen_transparente()),
        }
    }

    /// ¿La imagen llega por DMA-BUF (sin pasar por la CPU)?
    pub fn usa_dmabuf(&self) -> bool {
        self.dma.is_some()
    }

    /// ¿Hay que ignorar el alfa de lo que se ve? Los cuadros DMA-BUF son `XR24`: el cuarto byte
    /// no se usa y en las pruebas iba de 0 a 255, así que no vale como transparencia.
    pub fn ignora_alfa(&self) -> bool {
        self.dma_imagen.is_some()
    }

    /// Cuánto dura el fundido al cambiar de clip (`0`: corte seco). Mirar `transicion` antes
    /// de escribir: pedir acceso de escritura marca el componente como modificado.
    pub fn fijar_transicion(&mut self, segundos: f32) {
        self.duracion_de_transicion = segundos;
    }

    pub fn transicion(&self) -> f32 {
        self.duracion_de_transicion
    }

    /// Cuántos segundos van del fundido en marcha, o `None` si no hay ninguno. `ahora` es el
    /// reloj de Bevy. Quien conoce la duración y la curva (`vm_core`) con esto calcula el
    /// avance y lo devuelve con `avanzar_transicion`.
    pub fn segundos_en_transicion(&self, ahora: f64) -> Option<f32> {
        self.saliente
            .as_ref()
            .map(|_| (ahora - self.inicio_de_transicion) as f32)
    }

    /// Anota cuánto del clip nuevo se ve. Con 1 el fundido terminó: el clip viejo se cierra.
    /// Mientras dura, el viejo baja su volumen en la misma proporción.
    pub fn avanzar_transicion(&mut self, avance: f32) {
        self.avance = avance.clamp(0.0, 1.0);
        if self.avance >= 1.0 {
            self.avance = 1.0;
            if let Some(saliente) = self.saliente.take() {
                cerrar_en_segundo_plano(Some(saliente.video));
            }
        } else if let (Some(saliente), Some(base)) = (self.saliente.as_ref(), self.ajustes) {
            saliente.video.ajustar(Ajustes {
                volumen: base.volumen * (1.0 - self.avance),
                ..base
            });
        }
    }

    /// Cuánto del clip nuevo se ve (de 0 a 1; 1 si no hay fundido).
    pub fn avance_de_entrada(&self) -> f32 {
        self.avance
    }

    /// La textura del clip que está saliendo, si hay un fundido en marcha.
    pub fn textura_saliente(&self) -> Option<&Handle<Image>> {
        self.saliente.as_ref().map(|s| &s.textura)
    }

    /// Empieza a abrir un video. Devuelve error si el archivo no existe o GStreamer no
    /// puede armar la tubería. Si ya había uno abriéndose, se descarta.
    pub fn abrir(&mut self, ruta: &Path) -> Result<(), Box<dyn Error>> {
        self.abrir_con(ruta, Ajustes::default(), None)
    }

    /// Lo mismo que `abrir`, con ajustes de reproducción (bucle, velocidad, tramo, audio) y un
    /// tamaño máximo para los cuadros (ver `Video::abrir_con`).
    pub fn abrir_con(
        &mut self,
        ruta: &Path,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
    ) -> Result<(), Box<dyn Error>> {
        self.abrir_origen(&Origen::Archivo(ruta.to_path_buf()), ajustes, limite)
    }

    /// Abre cualquier origen: un archivo, una cámara o una transmisión de red.
    pub fn abrir_origen(
        &mut self,
        origen: &Origen,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
    ) -> Result<(), Box<dyn Error>> {
        let nuevo = Video::abrir_origen(origen, ajustes, limite)?;
        self.ajustes = Some(ajustes);
        cerrar_en_segundo_plano(self.pendiente.replace(nuevo));
        Ok(())
    }

    /// Empieza una captura de pantalla o ventana por el portal. El portal puede mostrar su
    /// selector y esperar al usuario, así que la sesión se pide en un hilo aparte: el video
    /// aparece cuando el portal responde. Con `token`, se reabre la fuente guardada sin
    /// preguntar. El token nuevo (si lo hay) se recoge con `tomar_token_nuevo`.
    pub fn abrir_captura(
        &mut self,
        token: Option<String>,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
        audio: Option<AudioDeCaptura>,
        ocultar_cursor: bool,
    ) {
        // Cada vez que se dispara una captura se vuelve a intentar por DMA-BUF: un fallo
        // pasajero de la vez anterior no la deja en el camino por CPU para siempre.
        self.permitir_dmabuf = true;
        self.pedir_captura(token, ajustes, limite, audio, ocultar_cursor);
    }

    /// Pide la captura al portal sin tocar `permitir_dmabuf` (lo usa el respaldo por CPU).
    fn pedir_captura(
        &mut self,
        token: Option<String>,
        ajustes: Ajustes,
        limite: Option<(u32, u32)>,
        audio: Option<AudioDeCaptura>,
        ocultar_cursor: bool,
    ) {
        info!(
            "pidiendo una captura al portal ({})",
            if token.is_some() {
                "con el token guardado"
            } else {
                "se va a mostrar el selector"
            }
        );
        let (emisor, receptor) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = emisor.send(crate::captura::iniciar(token.as_deref(), ocultar_cursor));
        });
        self.ajustes = Some(ajustes);
        self.captura_en_curso = Some(EsperaDeCaptura {
            respuesta: std::sync::Mutex::new(receptor),
            ajustes,
            limite,
            audio,
            ocultar_cursor,
        });
    }

    /// ¿Se está esperando la respuesta del portal?
    pub fn esperando_captura(&self) -> bool {
        self.captura_en_curso.is_some()
    }

    /// ¿Hay un token de restauración sin leer? (Se mira con acceso de solo lectura.)
    pub fn hay_token_nuevo(&self) -> bool {
        self.token_nuevo.is_some()
    }

    /// Entrega el token de restauración nuevo del portal y lo borra.
    pub fn tomar_token_nuevo(&mut self) -> Option<String> {
        self.token_nuevo.take()
    }

    /// Los ajustes que tiene puestos el video que suena (o se está abriendo).
    pub fn ajustes(&self) -> Option<Ajustes> {
        self.ajustes
    }

    /// Cambia los ajustes del video en marcha (ver `Video::ajustar`).
    pub fn ajustar(&mut self, ajustes: Ajustes) {
        for video in [self.actual.as_ref(), self.pendiente.as_ref()]
            .into_iter()
            .flatten()
        {
            video.ajustar(ajustes);
        }
        self.ajustes = Some(ajustes);
    }

    /// Pausa el video (`true`) o lo hace seguir (`false`).
    pub fn pausar(&self, en_pausa: bool) {
        for video in [
            self.actual.as_ref(),
            self.pendiente.as_ref(),
            self.saliente.as_ref().map(|s| &s.video),
        ]
        .into_iter()
        .flatten()
        {
            video.pausar(en_pausa);
        }
    }

    /// Salta a un punto del video: `fraccion` de 0 a 1 sobre su duración.
    pub fn buscar(&self, fraccion: f32) {
        if let Some(video) = self.actual.as_ref().or(self.pendiente.as_ref()) {
            video.buscar(fraccion);
        }
    }

    /// Las muestras de audio (mono, 48 kHz) del clip que suena, para analizar su sonido. Vacío
    /// si no suena nada o el clip no tiene audio. Se pide con `&self`: no marca el componente.
    pub fn tomar_muestras_de_audio(&self) -> Vec<f32> {
        self.actual
            .as_ref()
            .map(Video::tomar_muestras_de_audio)
            .unwrap_or_default()
    }

    /// Dónde va el video que se ve y cuánto dura, en segundos.
    pub fn posicion(&self) -> Option<(f64, f64)> {
        self.actual.as_ref()?.posicion()
    }

    /// ¿Hay un error sin leer? (Se mira con acceso de solo lectura para no marcar el
    /// componente como modificado en cada frame.)
    pub fn hay_error(&self) -> bool {
        self.error.is_some()
    }

    /// ¿Hay un aviso sin leer? (Solo lectura, igual que `hay_error`.)
    pub fn hay_aviso(&self) -> bool {
        self.aviso.is_some()
    }

    /// Entrega el aviso pendiente y lo borra.
    pub fn tomar_aviso(&mut self) -> Option<String> {
        self.aviso.take()
    }

    /// Entrega el error pendiente (por ejemplo, un archivo dañado) y lo borra.
    pub fn tomar_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// Detiene la reproducción y deja la textura transparente.
    pub fn detener(&mut self) {
        self.ajustes = None;
        self.captura_en_curso = None;
        cerrar_en_segundo_plano(self.actual.take());
        cerrar_en_segundo_plano(self.pendiente.take());
        cerrar_en_segundo_plano(self.saliente.take().map(|s| s.video));
        self.dma = None;
        self.dma_imagen = None;
        self.avance = 1.0;
        self.limpiar = true;
    }
}

/// Una captura esperando la respuesta del portal.
struct EsperaDeCaptura {
    respuesta: std::sync::Mutex<std::sync::mpsc::Receiver<Result<SesionDeCaptura, String>>>,
    ajustes: Ajustes,
    limite: Option<(u32, u32)>,
    audio: Option<AudioDeCaptura>,
    ocultar_cursor: bool,
}

fn imagen_transparente() -> Image {
    Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Cerrar un video espera a que termine su hilo y apaga la tubería, y puede tardar un
/// momento. Se hace en otro hilo para no trabar la interfaz al cambiar de clip.
fn cerrar_en_segundo_plano(video: Option<Video>) {
    if let Some(video) = video {
        std::thread::spawn(move || drop(video));
    }
}

fn actualizar_reproductores(
    mut reproductores: Query<&mut Reproductor>,
    mut imagenes: ResMut<Assets<Image>>,
    reloj: Res<Time>,
    ranuras: Res<RanurasDmaBuf>,
) {
    // Las capturas DMA-BUF vivas, para que el mundo de render sepa a qué imagen importar.
    let mut para_importar = Vec::new();
    for mut r in &mut reproductores {
        // Una captura que esperaba al portal: si ya respondió, se arma el video. Se mira
        // primero sin pedir acceso de escritura.
        //
        // Si el portal responde antes de que el mundo de render sepa si la GPU puede importar
        // DMA-BUF (pasa con una captura que arranca junto con el proyecto), se espera unos
        // cuadros: la respuesta queda en el canal. Si no, caería al camino por CPU sin motivo.
        let sabe_si_hay_dmabuf =
            !r.permitir_dmabuf || dmabuf_deshabilitado() || ranuras.disponible().is_some();
        let respuesta = r
            .captura_en_curso
            .as_ref()
            .filter(|_| sabe_si_hay_dmabuf)
            .map(|e| {
                e.respuesta
                    .lock()
                    .map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv())
            });
        match respuesta {
            Some(Ok(resultado)) => {
                if let Some(espera) = r.captura_en_curso.take() {
                    match resultado {
                        Ok(sesion) => {
                            info!(
                                "el portal entregó la captura (nodo {}, token {})",
                                sesion.nodo,
                                if sesion.token.is_some() { "sí" } else { "no" }
                            );
                            r.token_nuevo = sesion.token.clone();
                            r.ultima_captura = Some(UltimaCaptura {
                                token: sesion.token.clone(),
                                ajustes: espera.ajustes,
                                limite: espera.limite,
                                audio: espera.audio.clone(),
                                ocultar_cursor: espera.ocultar_cursor,
                            });
                            // Por DMA-BUF si la GPU puede y nada lo impide; si no, por CPU.
                            let por_gpu = r.permitir_dmabuf
                                && !dmabuf_deshabilitado()
                                && espera.audio.is_none()
                                && ranuras.disponible() == Some(true);
                            if por_gpu {
                                match CapturaDmaBuf::abrir(sesion) {
                                    Ok(captura) => {
                                        info!("captura abierta por DMA-BUF (sin pasar por la CPU)");
                                        // Lo que hubiera sonando se cierra: la imagen ahora
                                        // viene de la GPU. La textura vieja se ve hasta el
                                        // primer cuadro nuevo.
                                        cerrar_en_segundo_plano(r.actual.take());
                                        cerrar_en_segundo_plano(r.pendiente.take());
                                        cerrar_en_segundo_plano(r.saliente.take().map(|s| s.video));
                                        r.avance = 1.0;
                                        r.dma = Some(captura);
                                        r.dma_imagen = None;
                                    }
                                    Err(motivo) => {
                                        warn!("DMA-BUF no disponible ({motivo}): se usa la CPU");
                                        r.permitir_dmabuf = false;
                                        reabrir_por_cpu(&mut r);
                                    }
                                }
                            } else {
                                match Video::abrir_captura(
                                    sesion,
                                    espera.ajustes,
                                    espera.limite,
                                    espera.audio,
                                ) {
                                    Ok(video) => {
                                        cerrar_en_segundo_plano(r.pendiente.replace(video))
                                    }
                                    Err(e) => r.error = Some(e.to_string()),
                                }
                            }
                        }
                        Err(motivo) => {
                            warn!("el portal no entregó la captura: {motivo}");
                            // Si ya había una captura mostrándose (se estaba eligiendo otra y
                            // se canceló), sigue: es un aviso. Si no, la capa se queda sin nada.
                            if r.dma.is_some() || r.actual.is_some() {
                                r.aviso = Some(motivo);
                            } else {
                                r.error = Some(motivo);
                            }
                        }
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => r.captura_en_curso = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        if r.dma.is_some() {
            gestionar_dmabuf(&mut r, &mut imagenes);
            if let (Some(dma), Some(_)) = (r.dma.as_ref(), r.dma_imagen.as_ref()) {
                para_importar.push((r.textura.id(), dma.ranura.clone()));
            }
        }

        // Un video que falla después de arrancar (archivo dañado, sin pista de video...) se
        // descarta y se guarda el motivo. Si falla el que se estaba abriendo, el clip que
        // ya sonaba sigue sonando.
        let fallo_pendiente = r.pendiente.as_ref().and_then(|v| v.error());
        if let Some(motivo) = fallo_pendiente {
            cerrar_en_segundo_plano(r.pendiente.take());
            r.error = Some(motivo);
        }
        let fallo_actual = r.actual.as_ref().and_then(|v| v.error());
        if let Some(motivo) = fallo_actual {
            cerrar_en_segundo_plano(r.actual.take());
            r.error = Some(motivo);
            r.limpiar = true;
        }

        // Se mira primero con acceso de solo lectura: pedir acceso de escritura marcaría
        // el componente como modificado en cada frame aunque no pase nada.
        let primer_frame = r.pendiente.as_ref().and_then(|v| v.ultimo_frame());

        let frame = if let Some(frame) = primer_frame {
            // Si había una captura por DMA-BUF, se cierra: la textura pasa a ser la de un video.
            if r.dma.take().is_some() {
                r.dma_imagen = None;
                r.textura = imagenes.add(imagen_transparente());
            }
            // El video nuevo ya entregó su primer frame: reemplaza al anterior.
            let viejo = r.actual.take();
            r.actual = r.pendiente.take();
            match viejo {
                // Con fundido, el viejo se queda (con su textura) hasta que termine; el nuevo
                // empieza en una textura propia.
                Some(video) if r.duracion_de_transicion > 0.0 => {
                    let textura_vieja =
                        std::mem::replace(&mut r.textura, imagenes.add(imagen_transparente()));
                    let anterior = r.saliente.replace(Saliente {
                        video,
                        textura: textura_vieja,
                    });
                    // Un fundido que todavía no terminaba cede el lugar al nuevo.
                    cerrar_en_segundo_plano(anterior.map(|s| s.video));
                    r.inicio_de_transicion = reloj.elapsed_secs_f64();
                    r.avance = 0.0;
                }
                otro => cerrar_en_segundo_plano(otro),
            }
            Some(frame)
        } else {
            r.actual.as_ref().and_then(|v| v.ultimo_frame())
        };

        if let Some(frame) = frame {
            escribir_frame(&mut r.textura, frame, &mut imagenes);
        } else if r.limpiar {
            r.limpiar = false;
            r.textura = imagenes.add(imagen_transparente());
        }

        // El clip que se está yendo sigue reproduciéndose: sus cuadros van a su textura.
        // (Se mira con acceso de solo lectura para no marcar el componente cada frame.)
        let frame_saliente = r.saliente.as_ref().and_then(|s| s.video.ultimo_frame());
        if let Some(frame) = frame_saliente
            && let Some(saliente) = r.saliente.as_mut()
        {
            escribir_frame(&mut saliente.textura, frame, &mut imagenes);
        }
    }
    ranuras.publicar(para_importar);
}

/// Mantiene al día una captura por DMA-BUF: crea la imagen destino cuando llega el primer
/// cuadro (o cambia el tamaño o el formato) y, si la importación falla, vuelve a la CPU.
fn gestionar_dmabuf(r: &mut Reproductor, imagenes: &mut Assets<Image>) {
    let Some(dma) = r.dma.as_ref() else {
        return;
    };
    // Un fallo de verdad (del bus o de la importación), o cuadros que llegan pero no sirven
    // (no son DMA-BUF) y nunca hubo uno bueno. Un cuadro raro suelto no cuenta.
    let fallo = dma
        .ranura
        .fallo()
        .or_else(|| dma.ranura.sin_salida(dma.abierta_hace()));
    if let Some(motivo) = fallo {
        warn!("la captura por DMA-BUF falló ({motivo}): se vuelve a la CPU");
        r.dma = None;
        r.dma_imagen = None;
        r.permitir_dmabuf = false;
        reabrir_por_cpu(r);
        return;
    }
    // La imagen destino: una `Image` común, creada una vez y sin datos propios (la GPU la
    // llena copiando cada cuadro). Si cambia el tamaño o el formato se crea otra: el handle
    // cambia, igual que con un video que cambia de resolución.
    let Some(datos) = dma.ranura.visto() else {
        return;
    };
    let actual = (datos.ancho, datos.alto, datos.fourcc.clone());
    if r.dma_imagen.as_ref() == Some(&actual) {
        return;
    }
    let Some(formato) = formato_de_imagen(&datos.fourcc) else {
        dma.ranura
            .informar_fallo(format!("formato {} sin soporte", datos.fourcc));
        return;
    };
    r.textura = imagenes.add(Image::new_uninit(
        Extent3d {
            width: datos.ancho,
            height: datos.alto,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        formato,
        RenderAssetUsages::RENDER_WORLD,
    ));
    r.dma_imagen = Some(actual);
}

/// Vuelve a pedir la última captura, ahora por el camino de CPU. La sesión del portal que se
/// usó para el DMA-BUF ya se cerró; con el token se reabre la misma fuente sin preguntar.
fn reabrir_por_cpu(r: &mut Reproductor) {
    if let Some(u) = r.ultima_captura.clone() {
        r.pedir_captura(u.token, u.ajustes, u.limite, u.audio, u.ocultar_cursor);
    }
}

/// Copia un frame a la textura. Si cambió la resolución, crea una imagen **nueva** y
/// reemplaza el handle (ver la nota de `Reproductor`).
fn escribir_frame(textura: &mut Handle<Image>, frame: Frame, imagenes: &mut Assets<Image>) {
    let tamano = Extent3d {
        width: frame.ancho,
        height: frame.alto,
        depth_or_array_layers: 1,
    };
    let mismo_tamano = imagenes
        .get(&*textura)
        .is_some_and(|i| i.texture_descriptor.size == tamano);

    if mismo_tamano {
        if let Some(mut imagen) = imagenes.get_mut(&*textura) {
            imagen.data = Some(frame.datos);
        }
    } else {
        *textura = imagenes.add(Image::new(
            tamano,
            TextureDimension::D2,
            frame.datos,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
    }
}
