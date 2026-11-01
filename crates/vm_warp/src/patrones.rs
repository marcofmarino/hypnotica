//! Muestra un patrón de prueba (ver `vm_core::patrones`) en una salida, encima de todo lo
//! demás. Se dibuja con el mismo material que las superficies, así que le llega la
//! **calibración de color** de la salida: sirve para calibrar de verdad.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;
use vm_core::{BordeSuave, CorreccionColor, ENTRADA_COMPLETA, PatronPrueba, generar};

use vm_composicion::TexturaComposicion;
use vm_output::{PatronesActivos, Salida};
use vm_proyecto::ProyectoActual;

use crate::homografia::Homografia;
use crate::material::{MascaraBlanca, MaterialSuperficie, ParametrosSuperficie};

/// La imagen de un patrón, puesta sobre una salida. Guarda con qué se creó para saber
/// cuándo hay que rehacerla (cambió el patrón o el tamaño de la salida).
#[derive(Component)]
pub(crate) struct PatronDeSalida {
    id: String,
    patron: PatronPrueba,
    tamano: UVec2,
    material: Handle<MaterialSuperficie>,
}

/// Las imágenes de los patrones que se muestran **dentro de las superficies**, con la
/// resolución de la composición (así el recorte de la entrada y la proporción se comportan
/// como con cualquier otra imagen). Se hace una por patrón y resolución, no una por superficie.
#[derive(Resource, Default)]
pub(crate) struct ImagenesDePatron(pub(crate) HashMap<(PatronPrueba, u32, u32), Handle<Image>>);

fn imagen_de_patron(patron: PatronPrueba, ancho: u32, alto: u32) -> Image {
    Image::new(
        Extent3d {
            width: ancho,
            height: alto,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        generar(patron, ancho as usize, alto as usize),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Profundidad del patrón: por encima de cualquier superficie (que van de 0 hacia arriba
/// de a 0,01) y de la composición.
const PROFUNDIDAD: f32 = 500.0;

fn tamano_de(salida: &Salida) -> UVec2 {
    UVec2::new(
        salida.tamano.x.round() as u32,
        salida.tamano.y.round() as u32,
    )
}

/// Los parámetros del material para una imagen que cubre la salida entera, sin deformar.
fn parametros_de_pantalla(tamano: Vec2, color: &CorreccionColor) -> ParametrosSuperficie {
    let (mx, my) = (tamano.x / 2.0, tamano.y / 2.0);
    let h = Homografia::desde_esquinas([[-mx, my], [mx, my], [mx, -my], [-mx, -my]]);
    ParametrosSuperficie::nuevos(&h, ENTRADA_COMPLETA, &BordeSuave::default(), false, color)
}

fn color_de(proyecto: &ProyectoActual, id: &str) -> CorreccionColor {
    proyecto
        .0
        .salidas
        .iter()
        .find(|s| s.monitor == id)
        .map(|s| s.color)
        .unwrap_or_default()
}

/// Crea, cambia o quita el patrón de cada salida según lo pedido en `PatronesActivos`, y
/// mantiene al día su calibración de color.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sincronizar_patrones(
    mut commands: Commands,
    activos: Res<PatronesActivos>,
    proyecto: Res<ProyectoActual>,
    salidas: Query<&Salida>,
    existentes: Query<(Entity, &PatronDeSalida)>,
    mut imagenes: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materiales: ResMut<Assets<MaterialSuperficie>>,
    blanca: Res<MascaraBlanca>,
    composicion: Res<TexturaComposicion>,
    mut de_superficie: ResMut<ImagenesDePatron>,
) {
    // Los patrones que van dentro de las superficies: una imagen por patrón y resolución.
    let en_uso: Vec<(PatronPrueba, u32, u32)> = activos
        .0
        .values()
        .filter(|a| a.en_superficie)
        .map(|a| (a.patron, composicion.ancho, composicion.alto))
        .collect();
    for clave in &en_uso {
        if !de_superficie.0.contains_key(clave) {
            let imagen = imagenes.add(imagen_de_patron(clave.0, clave.1, clave.2));
            de_superficie.0.insert(*clave, imagen);
        }
    }
    // Las que ya no se usan se sueltan (su memoria se libera sola).
    if de_superficie.0.len() > en_uso.len() {
        de_superficie.0.retain(|k, _| en_uso.contains(k));
    }

    // Se quitan los que ya no corresponden: la salida se cerró, ya no se pidió ese patrón
    // o cambió el tamaño de la salida.
    for (entidad, patron) in &existentes {
        let vigente = salidas.iter().any(|s| {
            s.id == patron.id
                && tamano_de(s) == patron.tamano
                && activos
                    .0
                    .get(&s.id)
                    .is_some_and(|a| !a.en_superficie && a.patron == patron.patron)
        });
        if !vigente {
            commands.entity(entidad).despawn();
        }
    }

    // La calibración se refleja en los que siguen.
    if proyecto.is_changed() {
        for (_, patron) in &existentes {
            if let Some(mut material) = materiales.get_mut(&patron.material) {
                let tamano = patron.tamano.as_vec2();
                material.parametros =
                    parametros_de_pantalla(tamano, &color_de(&proyecto, &patron.id));
            }
        }
    }

    // Se crean los que faltan.
    for salida in &salidas {
        let Some(&activo) = activos.0.get(&salida.id) else {
            continue;
        };
        if activo.en_superficie {
            continue;
        }
        let patron = activo.patron;
        let tamano = tamano_de(salida);
        let ya_esta = existentes
            .iter()
            .any(|(_, p)| p.id == salida.id && p.patron == patron && p.tamano == tamano);
        if ya_esta || tamano.x == 0 || tamano.y == 0 {
            continue;
        }
        let imagen = imagenes.add(Image::new(
            Extent3d {
                width: tamano.x,
                height: tamano.y,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            generar(patron, tamano.x as usize, tamano.y as usize),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
        let material = materiales.add(MaterialSuperficie {
            parametros: parametros_de_pantalla(salida.tamano, &color_de(&proyecto, &salida.id)),
            composicion: imagen,
            mascara: blanca.0.clone(),
        });
        let capa = RenderLayers::layer(salida.capa);
        let mut entidad = commands.spawn((
            PatronDeSalida {
                id: salida.id.clone(),
                patron,
                tamano,
                material: material.clone(),
            },
            Mesh2d(meshes.add(Rectangle::new(salida.tamano.x, salida.tamano.y))),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, 0.0, PROFUNDIDAD),
            capa.clone(),
        ));
        if patron.lleva_rotulo() {
            entidad.with_child((
                Text2d::new(format!("{}   {} x {}", salida.id, tamano.x, tamano.y)),
                TextFont {
                    font_size: bevy::text::FontSize::Px(56.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextBackgroundColor(Color::BLACK),
                Transform::from_xyz(0.0, 0.0, 1.0),
                capa,
            ));
        }
    }
}
