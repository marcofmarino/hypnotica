//! Material de las capas de la composición: opacidad y modo de mezcla.
//!
//! Cada capa es un rectángulo con este material. El **modo de mezcla** se logra con la
//! función de mezcla fija de la GPU, que se configura al armar la tubería de dibujo: no
//! hace falta leer lo ya dibujado. Cada modo es una variante de la tubería (una
//! "especialización"), elegida por la clave `ClaveMaterialCapa`.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{
    AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin,
};
use vm_core::ModoMezcla;

pub struct MaterialCapaPlugin;

impl Plugin for MaterialCapaPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "capa.wesl");
        app.add_plugins(Material2dPlugin::<MaterialCapa>::default())
            .add_systems(Startup, crear_malla_de_capa);
    }
}

/// El rectángulo de 1×1 que usan todas las capas; cada una lo estira con su `Transform`.
#[derive(Resource)]
pub struct MallaDeCapa(pub Handle<Mesh>);

fn crear_malla_de_capa(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(MallaDeCapa(meshes.add(Rectangle::new(1.0, 1.0))));
}

/// Los números que se le pasan al shader.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct ParametrosCapa {
    pub opacidad: f32,
    /// El número del modo (ver `ModoMezcla::numero`).
    pub modo: u32,
    /// 1 si hay que tratar el cuadro como opaco (ignorar su canal alfa): las capturas por
    /// DMA-BUF traen un byte «sin usar» donde Bevy espera el alfa.
    pub ignorar_alfa: u32,
}

impl ParametrosCapa {
    pub fn nuevos(opacidad: f32, modo: ModoMezcla, ignorar_alfa: bool) -> Self {
        ParametrosCapa {
            opacidad,
            modo: modo.numero(),
            ignorar_alfa: u32::from(ignorar_alfa),
        }
    }
}

/// Lo que distingue una variante de la tubería de otra: el modo de mezcla. Tiene que ser
/// chica y barata de comparar, porque Bevy la usa de clave de un caché de tuberías.
#[repr(C)]
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct ClaveMaterialCapa {
    modo: u32,
}

impl From<&MaterialCapa> for ClaveMaterialCapa {
    fn from(material: &MaterialCapa) -> Self {
        ClaveMaterialCapa {
            modo: material.parametros.modo,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ClaveMaterialCapa)]
pub struct MaterialCapa {
    #[uniform(0)]
    pub parametros: ParametrosCapa,
    /// El último cuadro del video de la capa.
    #[texture(1)]
    #[sampler(2)]
    pub textura: Handle<Image>,
}

/// La función de mezcla de la GPU para cada modo: `resultado = S·a + D·b`, donde `S` es lo
/// que devuelve el shader, `D` lo que ya estaba dibujado, y `a` y `b` se eligen de una
/// lista corta de factores.
fn mezcla_de(modo: u32) -> BlendState {
    // La transparencia de la composición se conserva: el canal alfa de lo de abajo no cambia.
    let conservar_alfa = BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
    let color = |src_factor, dst_factor| BlendComponent {
        src_factor,
        dst_factor,
        operation: BlendOperation::Add,
    };
    match modo {
        // Normal: S·α + D·(1−α).
        0 => BlendState::ALPHA_BLENDING,
        // Suma: S + D.
        1 => BlendState {
            color: color(BlendFactor::One, BlendFactor::One),
            alpha: conservar_alfa,
        },
        // Multiplicar: S·D.
        2 => BlendState {
            color: color(BlendFactor::Dst, BlendFactor::Zero),
            alpha: conservar_alfa,
        },
        // Pantalla: S + D·(1−S).
        _ => BlendState {
            color: color(BlendFactor::One, BlendFactor::OneMinusSrc),
            alpha: conservar_alfa,
        },
    }
}

impl Material2d for MaterialCapa {
    /// Con `Blend` la capa se dibuja en la fase de transparencias, ordenada por profundidad
    /// (`z`): así la capa 0 queda arriba de todas.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("capa.wesl")).with_source("embedded"),
        )
    }

    /// Cambia la función de mezcla de la tubería según el modo de la capa.
    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        clave: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragmento) = descriptor.fragment.as_mut()
            && let Some(Some(objetivo)) = fragmento.targets.first_mut()
        {
            objetivo.blend = Some(mezcla_de(clave.bind_group_data.modo));
        }
        Ok(())
    }
}
