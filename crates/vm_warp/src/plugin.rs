use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use vm_composicion::{TexturaComposicion, TexturasDeCapas, textura_de_origen};
use vm_core::{ConfigSuperficie, Malla, Proyecto, Punto};
use vm_output::{PatronesActivos, Salida};
use vm_proyecto::{ProyectoActual, TecladoOcupado};

use crate::deformacion;
use crate::homografia::{Homografia, es_convexo};
use crate::material::{
    MascaraBlanca, MaterialSuperficie, MaterialSuperficiePlugin, ParametrosSuperficie,
    imagen_mascara,
};
use crate::patrones::ImagenesDePatron;

/// Cuántos tramos se dibujan entre dos puntos de control de la malla. Más tramos siguen
/// mejor la curva (y cuestan más vértices); con 8 la curva se ve lisa.
const SUBDIVISIONES: usize = 8;

/// Separación en profundidad (`z`) entre superficies: la de más adelante en la lista se
/// dibuja encima de las anteriores.
const PASO_Z: f32 = 0.01;

/// Profundidad a partir de la cual se dibujan los marcadores: encima de cualquier superficie.
const Z_MARCADORES: f32 = 10.0;

pub struct WarpPlugin;

impl Plugin for WarpPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialSuperficiePlugin)
            .init_resource::<Seleccion>()
            .init_resource::<MostrarGuias>()
            .init_resource::<ImagenesDePatron>()
            .add_systems(
                Update,
                (
                    sincronizar_superficies,
                    actualizar_origen_de_materiales,
                    limpiar_superficies,
                    mover_esquinas,
                    sincronizar_mallas,
                    crate::mascaras::sincronizar_mascaras,
                    crate::patrones::sincronizar_patrones,
                    actualizar_marcadores,
                )
                    .chain(),
            );
    }
}

/// La entidad que dibuja una superficie. Sus esquinas **no** se guardan acá:
/// viven en el `ProyectoActual`; `id` (el monitor) e `indice` (la posición en la lista
/// de superficies de esa salida) dicen cuál es la suya.
#[derive(Component)]
struct Superficie {
    /// La salida (cámara) donde se dibuja. Si esa entidad desaparece, esta también.
    salida: Entity,
    id: String,
    indice: usize,
    malla: Handle<Mesh>,
    material: Handle<MaterialSuperficie>,
    /// La forma y los huecos que se aplicaron a la máscara la última vez; sirven para saber
    /// cuándo cambiaron.
    forma: Option<Vec<Punto>>,
    huecos: Vec<Vec<Punto>>,
    /// La imagen de máscara que usa su material.
    mascara: Handle<Image>,
    /// Cuántos puntos de control (columnas, filas) tenía la malla la última vez; sirve para
    /// saber cuándo hay que rehacer la malla dibujable (cambia la cantidad de vértices).
    dimensiones_malla: Option<(usize, usize)>,
}

/// Cuadradito que marca una esquina de una superficie.
#[derive(Component)]
struct Marcador {
    salida: Entity,
    id: String,
    indice: usize,
    esquina: usize,
}

/// Qué salida (por id de monitor), qué superficie de esa salida y qué esquina se están
/// editando. La comparten el teclado y el editor con mouse de la interfaz.
#[derive(Resource, Default)]
pub struct Seleccion {
    pub id: String,
    pub superficie: usize,
    pub esquina: usize,
}

/// Opciones de visualización de las salidas.
///
/// `Default` de un `bool` es `false`: lo que se ve en una salida se proyecta, así que las
/// guías empiezan ocultas. Las esquinas se editan sobre la vista previa de la interfaz.
#[derive(Resource, Default)]
pub struct MostrarGuias {
    /// Dibujar los marcadores de las esquinas en las salidas (los proyectores).
    /// Sirven de guía al montar, pero se proyectan: tienen que poder ocultarse.
    pub en_salidas: bool,
}

/// Configuración de la superficie `indice` de la salida del monitor `id`, si existe.
/// `Option` es el tipo de Rust para "puede haber un valor o no".
fn superficie_de<'a>(
    proyecto: &'a Proyecto,
    id: &str,
    indice: usize,
) -> Option<&'a ConfigSuperficie> {
    proyecto
        .salidas
        .iter()
        .find(|s| s.monitor == id)?
        .superficies
        .get(indice)
}

/// Los recursos donde se guardan las mallas y los materiales de las superficies.
#[derive(SystemParam)]
struct ActivosDeSuperficie<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    materiales: ResMut<'w, Assets<MaterialSuperficie>>,
    imagenes: ResMut<'w, Assets<Image>>,
    blanca: Res<'w, MascaraBlanca>,
    /// La textura de cada capa (para las superficies cuyo origen es una capa).
    capas: Res<'w, TexturasDeCapas>,
}

/// Mantiene una entidad (con sus 4 marcadores) por cada superficie del proyecto.
///
/// Si la cantidad de superficies de una salida no coincide con las que están dibujadas
/// (la salida recién se abrió, o se agregó, duplicó o borró una), se **reconstruyen todas**
/// las de esa salida: es raro y barato. Reordenar no cambia la cantidad: las entidades se
/// quedan y simplemente leen su nueva configuración (`sincronizar_mallas`).
fn sincronizar_superficies(
    mut commands: Commands,
    salidas: Query<(Entity, &Salida)>,
    superficies: Query<(Entity, &Superficie)>,
    marcadores: Query<(Entity, &Marcador)>,
    proyecto: Res<ProyectoActual>,
    composicion: Res<TexturaComposicion>,
    mut activos: ActivosDeSuperficie,
) {
    for (entidad, salida) in &salidas {
        let Some(config) = proyecto.0.salidas.iter().find(|c| c.monitor == salida.id) else {
            warn!("la salida {} no está en el proyecto", salida.id);
            continue;
        };
        let dibujadas = superficies
            .iter()
            .filter(|(_, s)| s.salida == entidad)
            .count();
        if dibujadas == config.superficies.len() {
            continue;
        }

        // Se borra lo que había de esta salida y se vuelve a crear.
        for (e, s) in &superficies {
            if s.salida == entidad {
                commands.entity(e).despawn();
            }
        }
        for (e, m) in &marcadores {
            if m.salida == entidad {
                commands.entity(e).despawn();
            }
        }

        let capa = RenderLayers::layer(salida.capa);
        for (indice, sup) in config.superficies.iter().enumerate() {
            let h = Homografia::desde_esquinas(sup.esquinas);
            let malla = activos.meshes.add(construir_malla(&h, sup.malla.as_ref()));
            // Sin forma, la superficie usa la máscara blanca compartida (se dibuja entera).
            let mascara = match (&sup.forma, sup.huecos.is_empty()) {
                (None, true) => activos.blanca.0.clone(),
                _ => activos
                    .imagenes
                    .add(imagen_mascara(sup.forma.as_deref(), &sup.huecos)),
            };
            let material = activos.materiales.add(MaterialSuperficie {
                parametros: ParametrosSuperficie::nuevos(
                    &h,
                    sup.entrada,
                    &sup.borde,
                    sup.malla.is_some(),
                    &config.color,
                ),
                composicion: textura_de_origen(sup.origen, &composicion, &activos.capas).imagen,
                mascara: mascara.clone(),
            });

            commands.spawn((
                Superficie {
                    salida: entidad,
                    id: salida.id.clone(),
                    indice,
                    malla: malla.clone(),
                    material: material.clone(),
                    forma: sup.forma.clone(),
                    huecos: sup.huecos.clone(),
                    mascara,
                    dimensiones_malla: sup.malla.as_ref().map(|m| (m.columnas, m.filas)),
                },
                Mesh2d(malla),
                MeshMaterial2d(material),
                Transform::from_xyz(0.0, 0.0, indice as f32 * PASO_Z),
                capa.clone(),
            ));

            for (esquina, punto) in sup.esquinas.iter().enumerate() {
                commands.spawn((
                    Marcador {
                        salida: entidad,
                        id: salida.id.clone(),
                        indice,
                        esquina,
                    },
                    Sprite::from_color(Color::WHITE, Vec2::splat(20.0)),
                    Transform::from_xyz(punto[0], punto[1], Z_MARCADORES + indice as f32 * PASO_Z),
                    // En su propia capa: la vista previa de la interfaz no los ve.
                    RenderLayers::from_layers(&[salida.capa_guias]),
                ));
            }
        }
    }
}

/// Mantiene cada superficie leyendo la textura de su origen (la composición o una capa).
///
/// Se revisa en cada frame porque cualquiera de esas texturas puede cambiar de handle (una
/// nueva resolución de la composición, o un video que cambia de tamaño en una capa): el
/// handle viejo ya no apunta a nada. Se compara antes de escribir para no marcar el
/// material como modificado sin necesidad.
fn actualizar_origen_de_materiales(
    proyecto: Res<ProyectoActual>,
    composicion: Res<TexturaComposicion>,
    capas: Res<TexturasDeCapas>,
    activos: Res<PatronesActivos>,
    patrones: Res<ImagenesDePatron>,
    superficies: Query<&Superficie>,
    mut materiales: ResMut<Assets<MaterialSuperficie>>,
) {
    for sup in &superficies {
        let Some(config) = superficie_de(&proyecto.0, &sup.id, sup.indice) else {
            continue;
        };
        // Con un patrón «en la superficie», la imagen de entrada es el patrón (si ya se creó).
        let de_patron = activos
            .0
            .get(&sup.id)
            .filter(|a| a.en_superficie)
            .and_then(|a| {
                patrones
                    .0
                    .get(&(a.patron, composicion.ancho, composicion.alto))
            })
            .cloned();
        let deseada = de_patron
            .unwrap_or_else(|| textura_de_origen(config.origen, &composicion, &capas).imagen);
        let difiere = materiales
            .get(&sup.material)
            .is_some_and(|m| m.composicion != deseada);
        if difiere && let Some(mut material) = materiales.get_mut(&sup.material) {
            material.composicion = deseada;
        }
    }
}

/// Cuando se cierra una salida, se borran sus superficies y sus marcadores.
fn limpiar_superficies(
    mut commands: Commands,
    salidas: Query<(), With<Salida>>,
    superficies: Query<(Entity, &Superficie)>,
    marcadores: Query<(Entity, &Marcador)>,
) {
    for (entidad, sup) in &superficies {
        if !salidas.contains(sup.salida) {
            commands.entity(entidad).despawn();
        }
    }
    for (entidad, marcador) in &marcadores {
        if !salidas.contains(marcador.salida) {
            commands.entity(entidad).despawn();
        }
    }
}

/// Cuántos tramos tiene la malla dibujable a lo ancho y a lo alto.
///
/// Sin malla de control alcanza con **un solo rectángulo** (4 vértices): el shader hace la
/// deformación exacta por píxel. Con malla hay una grilla densa de triángulos, porque la
/// deformación ya no es una homografía y hay que dibujarla vértice por vértice.
fn tramos_de(malla: Option<&Malla>) -> (usize, usize) {
    match malla {
        None => (1, 1),
        Some(m) => (
            (m.columnas - 1) * SUBDIVISIONES,
            (m.filas - 1) * SUBDIVISIONES,
        ),
    }
}

/// La posición en pantalla de cada vértice de la malla dibujable.
fn posiciones(h: &Homografia, malla: Option<&Malla>) -> Vec<[f32; 3]> {
    let (nx, ny) = tramos_de(malla);
    let mut pos = Vec::with_capacity((nx + 1) * (ny + 1));
    for fila in 0..=ny {
        for col in 0..=nx {
            let u = col as f32 / nx as f32;
            let v = fila as f32 / ny as f32;
            let [x, y] = deformacion::proyectar(h, malla, u, v);
            pos.push([x, y, 0.0]);
        }
    }
    pos
}

fn construir_malla(h: &Homografia, malla: Option<&Malla>) -> Mesh {
    let (nx, ny) = tramos_de(malla);

    // Las coordenadas (u, v) de cada vértice: una grilla regular de 0 a 1.
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((nx + 1) * (ny + 1));
    for fila in 0..=ny {
        for col in 0..=nx {
            uvs.push([col as f32 / nx as f32, fila as f32 / ny as f32]);
        }
    }

    // Dos triángulos por celda.
    let mut indices: Vec<u32> = Vec::with_capacity(nx * ny * 6);
    for fila in 0..ny {
        for col in 0..nx {
            let i0 = (fila * (nx + 1) + col) as u32;
            let i1 = i0 + 1;
            let i2 = i0 + (nx as u32 + 1);
            let i3 = i2 + 1;
            indices.extend([i0, i2, i3, i0, i3, i1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, posiciones(h, malla))
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn mover_esquinas(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    ocupado: Res<TecladoOcupado>,
    mut seleccion: ResMut<Seleccion>,
    mut proyecto: ResMut<ProyectoActual>,
    salidas: Query<&Salida>,
) {
    // Si la interfaz está usando el teclado (se está escribiendo un nombre), no se reacciona.
    if ocupado.0 {
        return;
    }

    // Todas las superficies de las salidas abiertas, como pares (salida, superficie), en
    // un orden fijo para que Tab sea predecible.
    let mut pares: Vec<(String, usize)> = Vec::new();
    for salida in &salidas {
        if let Some(config) = proyecto.0.salidas.iter().find(|c| c.monitor == salida.id) {
            pares.extend((0..config.superficies.len()).map(|i| (salida.id.clone(), i)));
        }
    }
    pares.sort();
    if pares.is_empty() {
        return;
    }

    // Si la superficie elegida ya no existe (se cerró la salida o se borró), se pasa a la primera.
    let actual = match pares
        .iter()
        .position(|(id, i)| *id == seleccion.id && *i == seleccion.superficie)
    {
        Some(actual) => actual,
        None => {
            seleccion.id = pares[0].0.clone();
            seleccion.superficie = pares[0].1;
            0
        }
    };

    // Tab pasa a la siguiente superficie (el `%` vuelve a la primera al llegar al final).
    if keys.just_pressed(KeyCode::Tab) {
        let (id, superficie) = &pares[(actual + 1) % pares.len()];
        seleccion.id = id.clone();
        seleccion.superficie = *superficie;
    }

    let teclas = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ];
    for (i, tecla) in teclas.iter().enumerate() {
        if keys.just_pressed(*tecla) {
            seleccion.esquina = i;
        }
    }

    let paso = 200.0 * time.delta_secs();
    let mut dx = 0.0;
    let mut dy = 0.0;
    if keys.pressed(KeyCode::ArrowLeft) {
        dx -= paso;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dx += paso;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        dy += paso;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        dy -= paso;
    }

    // Recién acá se pide acceso de escritura: tocar `proyecto` como mutable lo marca
    // como "modificado", y eso dispara la actualización de las mallas.
    if dx != 0.0 || dy != 0.0 {
        let Some(sup) = proyecto
            .0
            .salidas
            .iter_mut()
            .find(|s| s.monitor == seleccion.id)
            .and_then(|s| s.superficies.get_mut(seleccion.superficie))
        else {
            return;
        };
        let mut nuevas = sup.esquinas;
        nuevas[seleccion.esquina][0] += dx;
        nuevas[seleccion.esquina][1] += dy;
        // Un cuadrilátero no convexo rompe la homografía: ese movimiento se ignora.
        if es_convexo(nuevas) {
            sup.esquinas = nuevas;
        }
    }
}

/// Actualiza las mallas, los parámetros del shader y la visibilidad de cada superficie
/// cuando el proyecto cambió (por mover una esquina, reordenar o cargar un archivo).
fn sincronizar_mallas(
    proyecto: Res<ProyectoActual>,
    mut superficies: Query<(&mut Superficie, &mut Visibility)>,
    mut activos: ActivosDeSuperficie,
) {
    if !proyecto.is_changed() {
        return;
    }
    for (mut sup, mut visibilidad) in &mut superficies {
        let Some(config) = superficie_de(&proyecto.0, &sup.id, sup.indice) else {
            continue;
        };
        // La calibración de color es de la salida: la comparten todas sus superficies.
        let color = proyecto
            .0
            .salidas
            .iter()
            .find(|s| s.monitor == sup.id)
            .map(|s| s.color)
            .unwrap_or_default();
        let color = &color;
        let h = Homografia::desde_esquinas(config.esquinas);
        let dimensiones = config.malla.as_ref().map(|m| (m.columnas, m.filas));
        if let Some(mut mesh) = activos.meshes.get_mut(&sup.malla) {
            if dimensiones != sup.dimensiones_malla {
                // Cambió la cantidad de puntos de control (o se creó o quitó la malla):
                // cambia la cantidad de vértices, así que se rehace la malla dibujable.
                *mesh = construir_malla(&h, config.malla.as_ref());
            } else {
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    posiciones(&h, config.malla.as_ref()),
                );
            }
        }
        sup.dimensiones_malla = dimensiones;
        // El shader necesita la matriz inversa de la nueva homografía, la entrada, el borde
        // y saber si hay malla (ahí usa las coordenadas de los vértices).
        if let Some(mut material) = activos.materiales.get_mut(&sup.material) {
            material.parametros.actualizar(
                &h,
                config.entrada,
                &config.borde,
                config.malla.is_some(),
                color,
            );
        }

        // La forma y los huecos: solo se vuelve a rellenar la máscara si cambiaron (rellenar
        // cuesta algo).
        if sup.forma != config.forma || sup.huecos != config.huecos {
            let tenia = sup.forma.is_some() || !sup.huecos.is_empty();
            let tiene = config.forma.is_some() || !config.huecos.is_empty();
            match (tenia, tiene) {
                // Ya tenía una máscara y sigue teniendo: se reescriben los píxeles de la imagen.
                (true, true) => {
                    if let Some(mut imagen) = activos.imagenes.get_mut(&sup.mascara) {
                        *imagen = imagen_mascara(config.forma.as_deref(), &config.huecos);
                    }
                }
                // Empezó a tener una: imagen nueva (un handle nuevo; ver nota en `Reproductor`).
                (false, true) => {
                    sup.mascara = activos
                        .imagenes
                        .add(imagen_mascara(config.forma.as_deref(), &config.huecos));
                }
                // Dejó de tener: vuelve a la máscara blanca compartida.
                (_, false) => sup.mascara = activos.blanca.0.clone(),
            }
            sup.forma = config.forma.clone();
            sup.huecos = config.huecos.clone();
            if let Some(mut material) = activos.materiales.get_mut(&sup.material) {
                material.mascara = sup.mascara.clone();
            }
        }

        // Una superficie oculta sigue existiendo, pero no se dibuja.
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

/// Coloca cada marcador sobre su esquina; el de la esquina seleccionada va en rojo.
fn actualizar_marcadores(
    seleccion: Res<Seleccion>,
    proyecto: Res<ProyectoActual>,
    guias: Res<MostrarGuias>,
    mut marcadores: Query<(&Marcador, &mut Transform, &mut Sprite, &mut Visibility)>,
) {
    for (marcador, mut transform, mut sprite, mut visible) in &mut marcadores {
        let Some(config) = superficie_de(&proyecto.0, &marcador.id, marcador.indice) else {
            continue;
        };

        // Las guías de una superficie oculta tampoco se muestran.
        let deseada = if guias.en_salidas && config.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        // Se compara antes de asignar para no marcar el componente como modificado sin necesidad.
        if *visible != deseada {
            *visible = deseada;
        }

        let [x, y] = config.esquinas[marcador.esquina];
        transform.translation.x = x;
        transform.translation.y = y;

        let elegido = marcador.id == seleccion.id
            && marcador.indice == seleccion.superficie
            && marcador.esquina == seleccion.esquina;
        sprite.color = if elegido {
            Color::srgb(1.0, 0.2, 0.2)
        } else {
            Color::WHITE
        };
    }
}
