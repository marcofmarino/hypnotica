//! Importa a la GPU los cuadros DMA-BUF que recibe `dmabuf.rs`. Corre en el **mundo de
//! render** de Bevy, que es donde vive el dispositivo de `wgpu`.
//!
//! Cada cuadro se importa como una textura temporal de Vulkan (`texture_from_dmabuf_fd`), se
//! **copia dentro de la GPU** a la textura de la capa (una `Image` común de Bevy, ya creada
//! por el mundo principal) y se suelta. La textura de la capa nunca cambia de handle por el
//! cuadro: así el resto del programa la usa igual que la de un video.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use wgpu::hal::api::Vulkan;

use crate::dmabuf::{CuadroDmaBuf, Ranura};

/// `HYPNOTICA_SIN_DMABUF=1` fuerza el camino por CPU (para comparar o si algo falla).
pub fn dmabuf_deshabilitado() -> bool {
    use std::sync::OnceLock;
    static VALOR: OnceLock<bool> = OnceLock::new();
    *VALOR.get_or_init(|| std::env::var_os("HYPNOTICA_SIN_DMABUF").is_some())
}

/// `HYPNOTICA_DIAGNOSTICO_DMABUF=1` lee de vuelta la textura destino de vez en cuando y
/// registra si el contenido llegó (no se puede sacar una captura de pantalla en todos los
/// escritorios). Es lento: solo para diagnosticar.
fn diagnostico_activo() -> bool {
    use std::sync::OnceLock;
    static VALOR: OnceLock<bool> = OnceLock::new();
    *VALOR.get_or_init(|| std::env::var_os("HYPNOTICA_DIAGNOSTICO_DMABUF").is_some())
}

/// Se sabe si la GPU puede importar DMA-BUF recién cuando el mundo de render corre por
/// primera vez.
const DESCONOCIDO: u8 = 0;
const SI: u8 = 1;
const NO: u8 = 2;

struct Compartido {
    /// Las capturas DMA-BUF vivas: a qué imagen va cada una. La publica el mundo principal
    /// en cada frame y la lee el de render.
    lista: Mutex<Vec<(AssetId<Image>, Arc<Ranura>)>>,
    disponible: AtomicU8,
    importados: AtomicU64,
}

/// Lo que comparten el mundo principal y el de render. Es el mismo `Arc` en los dos: no hace
/// falta extraer nada entre mundos.
#[derive(Resource, Clone)]
pub struct RanurasDmaBuf(Arc<Compartido>);

impl Default for RanurasDmaBuf {
    fn default() -> Self {
        RanurasDmaBuf(Arc::new(Compartido {
            lista: Mutex::new(Vec::new()),
            disponible: AtomicU8::new(DESCONOCIDO),
            importados: AtomicU64::new(0),
        }))
    }
}

impl RanurasDmaBuf {
    /// Reemplaza la lista de capturas DMA-BUF que hay que importar.
    pub fn publicar(&self, lista: Vec<(AssetId<Image>, Arc<Ranura>)>) {
        if let Ok(mut actual) = self.0.lista.lock() {
            *actual = lista;
        }
    }

    /// ¿Puede esta GPU importar DMA-BUF? `None` mientras no se sabe todavía (el mundo de
    /// render no corrió ni una vez).
    pub fn disponible(&self) -> Option<bool> {
        match self.0.disponible.load(Ordering::Relaxed) {
            SI => Some(true),
            NO => Some(false),
            _ => None,
        }
    }

    /// Cuántos cuadros se importaron en total (sirve para diagnosticar).
    pub fn importados(&self) -> u64 {
        self.0.importados.load(Ordering::Relaxed)
    }
}

pub struct ImportadorPlugin;

impl Plugin for ImportadorPlugin {
    fn build(&self, app: &mut App) {
        let ranuras = RanurasDmaBuf::default();
        app.insert_resource(ranuras.clone());
        // Sin renderizador (por ejemplo, en los tests con `MinimalPlugins`) no hay nada que
        // importar: `disponible()` queda en `None` y las capturas usan el camino por CPU.
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(ranuras)
                .add_systems(Render, importar_cuadros.in_set(RenderSystems::Prepare));
        }
    }
}

/// ¿Tiene el dispositivo lo necesario para importar DMA-BUF? Bevy ya pide por su cuenta todas
/// las features que soporta la GPU, así que alcanza con preguntar.
fn puede_importar(dispositivo: &RenderDevice) -> bool {
    dispositivo
        .features()
        .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        // SAFETY: solo se pregunta si el dispositivo es de Vulkan; no se usa lo que devuelve.
        && unsafe { dispositivo.wgpu_device().as_hal::<Vulkan>() }.is_some()
}

fn importar_cuadros(
    ranuras: Res<RanurasDmaBuf>,
    dispositivo: Res<RenderDevice>,
    cola: Res<RenderQueue>,
    imagenes: Res<RenderAssets<GpuImage>>,
) {
    let compartido = &ranuras.0;
    if compartido.disponible.load(Ordering::Relaxed) == DESCONOCIDO {
        let puede = puede_importar(&dispositivo);
        info!(
            "importación de DMA-BUF {}",
            if puede {
                "disponible en esta GPU"
            } else {
                "NO disponible: las capturas usan la CPU"
            }
        );
        compartido
            .disponible
            .store(if puede { SI } else { NO }, Ordering::Relaxed);
    }
    if compartido.disponible.load(Ordering::Relaxed) != SI {
        return;
    }

    // Una copia de la lista (son pocas capturas): así no se mantiene bloqueada mientras se
    // importa.
    let lista = match compartido.lista.lock() {
        Ok(l) => l.clone(),
        Err(_) => return,
    };
    for (id, ranura) in lista {
        let Some(cuadro) = ranura.tomar() else {
            continue;
        };
        // Si la imagen todavía no existe en la GPU (se acaba de crear), el cuadro se descarta:
        // llega otro enseguida.
        let Some(destino) = imagenes.get(id) else {
            continue;
        };
        match importar_y_copiar(&dispositivo, &cola, destino, cuadro) {
            Ok(true) => {
                let total = compartido.importados.fetch_add(1, Ordering::Relaxed) + 1;
                if total == 1 || total.is_multiple_of(600) {
                    info!("captura por DMA-BUF: {total} cuadros importados");
                }
            }
            Ok(false) => {}
            Err(motivo) => ranura.informar_fallo(motivo),
        }
    }
}

/// Importa un cuadro y lo copia a `destino`. Devuelve `Ok(false)` si el cuadro no encaja con
/// la imagen (cambió el tamaño o el formato y el mundo principal todavía no la recreó).
fn importar_y_copiar(
    dispositivo: &RenderDevice,
    cola: &RenderQueue,
    destino: &GpuImage,
    cuadro: CuadroDmaBuf,
) -> Result<bool, String> {
    let CuadroDmaBuf { fd, datos, muestra } = cuadro;
    let formato = match formato_de(&datos.fourcc) {
        Some(f) => f,
        None => return Err(format!("formato {} sin soporte", datos.fourcc)),
    };
    let tamano = wgpu::Extent3d {
        width: datos.ancho,
        height: datos.alto,
        depth_or_array_layers: 1,
    };
    // Las texturas del mismo formato (o que solo difieren en sRGB) son compatibles para copiar.
    let mismo_formato = destino.texture_descriptor.format.remove_srgb_suffix() == formato;
    if destino.texture.size() != tamano || !mismo_formato {
        return Ok(false);
    }

    let descripcion = wgpu::TextureDescriptor {
        label: Some("dmabuf importado"),
        size: tamano,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: formato,
        usage: wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let descripcion_hal = wgpu::hal::TextureDescriptor {
        label: Some("dmabuf importado"),
        size: tamano,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: formato,
        usage: wgpu::TextureUses::COPY_SRC,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: vec![],
    };

    let dispositivo = dispositivo.wgpu_device();
    // SAFETY: `fd` es un DMA-BUF válido que describe `datos` (viene del buffer de PipeWire,
    // que `muestra` mantiene vivo); wgpu se queda con el descriptor. Se pide un solo plano y
    // el buffer es compacto (stride y offset de su `VideoMeta`).
    let importada = unsafe {
        let hal = dispositivo
            .as_hal::<Vulkan>()
            .ok_or("el dispositivo no es de Vulkan")?;
        let textura = hal
            .texture_from_dmabuf_fd(
                fd,
                &descripcion_hal,
                datos.modificador,
                datos.stride,
                datos.offset,
            )
            .map_err(|e| format!("texture_from_dmabuf_fd: {e:?}"))?;
        dispositivo.create_texture_from_hal::<Vulkan>(
            textura,
            &descripcion,
            wgpu::TextureUses::COPY_SRC,
        )
    };

    let mut codificador = dispositivo.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("copiar dmabuf"),
    });
    codificador.copy_texture_to_texture(
        importada.as_image_copy(),
        destino.texture.as_image_copy(),
        tamano,
    );
    // Solo para diagnosticar: se lee de vuelta la textura destino (no la importada).
    let lectura = (diagnostico_activo() && leer_este_cuadro()).then(|| {
        let por_fila = (datos.ancho * 4).div_ceil(256) * 256;
        let buffer = dispositivo.create_buffer(&wgpu::BufferDescriptor {
            label: Some("diagnóstico dmabuf"),
            size: por_fila as u64 * datos.alto as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        codificador.copy_texture_to_buffer(
            destino.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(por_fila),
                    rows_per_image: Some(datos.alto),
                },
            },
            tamano,
        );
        (buffer, por_fila)
    });
    cola.submit([codificador.finish()]);
    if let Some((buffer, por_fila)) = lectura {
        diagnosticar(dispositivo, &buffer, por_fila, datos.ancho, datos.alto);
    }
    // El buffer de PipeWire se suelta **cuando la GPU termine de copiar**, no antes: si no,
    // el productor lo podría volver a pisar a mitad de la copia.
    cola.on_submitted_work_done(move || drop(muestra));
    Ok(true)
}

/// Cada cuántos cuadros se lee la textura de vuelta en el modo de diagnóstico.
fn leer_este_cuadro() -> bool {
    use std::sync::atomic::AtomicU64;
    static CUENTA: AtomicU64 = AtomicU64::new(0);
    CUENTA.fetch_add(1, Ordering::Relaxed).is_multiple_of(120)
}

/// Lee el buffer y registra cuántos píxeles no son negros y el color promedio.
fn diagnosticar(
    dispositivo: &wgpu::Device,
    buffer: &wgpu::Buffer,
    por_fila: u32,
    ancho: u32,
    alto: u32,
) {
    let (emisor, receptor) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = emisor.send(r);
    });
    let _ = dispositivo.poll(wgpu::PollType::wait_indefinitely());
    if !matches!(receptor.recv(), Ok(Ok(()))) {
        warn!("diagnóstico DMA-BUF: no se pudo leer el buffer");
        return;
    }
    let Ok(vista) = buffer.slice(..).get_mapped_range() else {
        return;
    };
    let (mut no_negros, mut suma) = (0u64, [0u64; 3]);
    for fila in 0..alto as usize {
        let inicio = fila * por_fila as usize;
        for px in vista[inicio..inicio + ancho as usize * 4].chunks_exact(4) {
            if px[0] != 0 || px[1] != 0 || px[2] != 0 {
                no_negros += 1;
            }
            for (acumulado, valor) in suma.iter_mut().zip(px) {
                *acumulado += u64::from(*valor);
            }
        }
    }
    let total = ancho as u64 * alto as u64;
    info!(
        "diagnóstico DMA-BUF {ancho}x{alto}: {:.1} % de píxeles no negros, color medio (canales de memoria 0,1,2) = {}, {}, {}",
        no_negros as f64 / total as f64 * 100.0,
        suma[0] / total,
        suma[1] / total,
        suma[2] / total
    );
}

/// El formato de `wgpu` para un formato DRM. DRM es «little endian»: `XR24` es B,G,R,X en
/// memoria. El byte `X` no se usa y **no hay que confiar en él como alfa** (en las pruebas iba
/// de 0 a 255): las capas con DMA-BUF ignoran el alfa.
fn formato_de(fourcc: &str) -> Option<wgpu::TextureFormat> {
    match fourcc {
        "XR24" | "AR24" | "BGRx" | "BGRA" => Some(wgpu::TextureFormat::Bgra8Unorm),
        "XB24" | "AB24" | "RGBx" | "RGBA" => Some(wgpu::TextureFormat::Rgba8Unorm),
        _ => None,
    }
}

/// Lo que `plugin.rs` necesita para crear la imagen destino de una captura: el formato de
/// Bevy (con sRGB, como las demás texturas de video) según el formato DRM.
pub fn formato_de_imagen(fourcc: &str) -> Option<wgpu::TextureFormat> {
    match formato_de(fourcc)? {
        wgpu::TextureFormat::Bgra8Unorm => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        wgpu::TextureFormat::Rgba8Unorm => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        _ => None,
    }
}
