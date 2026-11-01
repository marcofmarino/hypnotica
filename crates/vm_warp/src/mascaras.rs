//! Las máscaras de salida: polígonos negros dibujados **encima de todo** en un proyector,
//! para tapar lo que no debe recibir luz. Cada salida tiene como mucho una entidad con una
//! sola malla que junta todos sus polígonos.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use vm_core::{Punto, triangular};
use vm_output::Salida;
use vm_proyecto::ProyectoActual;

/// Profundidad de las máscaras: encima de las superficies (que van de 0 hacia arriba de a
/// 0,01) y debajo de los patrones de prueba.
const PROFUNDIDAD: f32 = 300.0;

/// La entidad que dibuja las máscaras de una salida. Guarda los polígonos con los que se
/// armó para saber cuándo hay que rehacer la malla.
#[derive(Component)]
pub(crate) struct MascarasDeSalida {
    salida: Entity,
    poligonos: Vec<Vec<Punto>>,
    malla: Handle<Mesh>,
}

/// Una malla con todos los polígonos (cada uno triangulado).
pub(crate) fn malla_de_mascaras(poligonos: &[Vec<Punto>]) -> Mesh {
    let mut posiciones: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for poligono in poligonos {
        let base = posiciones.len() as u32;
        posiciones.extend(poligono.iter().map(|p| [p[0], p[1], 0.0]));
        for [a, b, c] in triangular(poligono) {
            indices.extend([base + a as u32, base + b as u32, base + c as u32]);
        }
    }
    let uvs = vec![[0.0_f32, 0.0]; posiciones.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, posiciones)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Crea, actualiza o quita las máscaras de cada salida según el proyecto.
pub(crate) fn sincronizar_mascaras(
    mut commands: Commands,
    proyecto: Res<ProyectoActual>,
    salidas: Query<(Entity, &Salida)>,
    mut existentes: Query<(Entity, &mut MascarasDeSalida)>,
    mut mallas: ResMut<Assets<Mesh>>,
    mut materiales: ResMut<Assets<ColorMaterial>>,
) {
    // Las de una salida que se cerró.
    for (entidad, m) in &existentes {
        if !salidas.contains(m.salida) {
            commands.entity(entidad).despawn();
        }
    }

    for (entidad_salida, salida) in &salidas {
        let deseadas: &[Vec<Punto>] = proyecto
            .0
            .salidas
            .iter()
            .find(|c| c.monitor == salida.id)
            .map_or(&[], |c| c.mascaras.as_slice());
        let actual = existentes
            .iter_mut()
            .find(|(_, m)| m.salida == entidad_salida);
        match actual {
            Some((entidad, mut m)) => {
                if deseadas.is_empty() {
                    commands.entity(entidad).despawn();
                } else if m.poligonos != deseadas {
                    if let Some(mut malla) = mallas.get_mut(&m.malla) {
                        *malla = malla_de_mascaras(deseadas);
                    }
                    m.poligonos = deseadas.to_vec();
                }
            }
            None if !deseadas.is_empty() => {
                let malla = mallas.add(malla_de_mascaras(deseadas));
                commands.spawn((
                    MascarasDeSalida {
                        salida: entidad_salida,
                        poligonos: deseadas.to_vec(),
                        malla: malla.clone(),
                    },
                    Mesh2d(malla),
                    MeshMaterial2d(materiales.add(ColorMaterial::from(Color::BLACK))),
                    Transform::from_xyz(0.0, 0.0, PROFUNDIDAD),
                    RenderLayers::layer(salida.capa),
                ));
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_malla_junta_los_poligonos_con_sus_triangulos() {
        let cuadrado = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let triangulo = vec![[5.0, 5.0], [7.0, 5.0], [6.0, 7.0]];
        let malla = malla_de_mascaras(&[cuadrado, triangulo]);
        assert_eq!(malla.count_vertices(), 7);
        // 2 + 1 triángulos = 9 índices.
        assert_eq!(malla.indices().map(|i| i.len()), Some(9));
    }

    #[test]
    fn sin_poligonos_la_malla_queda_vacia() {
        let malla = malla_de_mascaras(&[]);
        assert_eq!(malla.count_vertices(), 0);
    }
}
