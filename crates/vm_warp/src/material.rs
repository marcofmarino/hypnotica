//! Material propio de las superficies: un shader que aplica la homografía por píxel.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::math::{Mat3, Vec2, Vec4};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

use vm_core::{BordeSuave, CorreccionColor, Entrada, Punto, rasterizar_con_huecos};

use crate::homografia::Homografia;

/// Agrega el material de las superficies y embebe su shader dentro del programa.
pub struct MaterialSuperficiePlugin;

impl Plugin for MaterialSuperficiePlugin {
    fn build(&self, app: &mut App) {
        // `embedded_asset!` mete el archivo `.wesl` dentro del ejecutable: el crate no
        // depende de que exista una carpeta `assets/` con el shader.
        embedded_asset!(app, "superficie.wesl");
        app.add_plugins(Material2dPlugin::<MaterialSuperficie>::default())
            .add_systems(Startup, crear_mascara_blanca);
    }
}

/// Lado (en píxeles) de la imagen donde se rellena la forma de una superficie. La imagen
/// cubre la superficie entera, sea cual sea su proporción; al leerla el shader con
/// interpolación, y con los bordes ya suavizados al rellenarla (ver `rasterizar`), no se
/// ven escalones.
pub const LADO_MASCARA: usize = 1024;

/// Una máscara de un solo píxel blanco: "se dibuja todo". La comparten las superficies que
/// no tienen forma, en vez de gastar una imagen entera cada una.
#[derive(Resource)]
pub struct MascaraBlanca(pub Handle<Image>);

fn crear_mascara_blanca(mut commands: Commands, mut imagenes: ResMut<Assets<Image>>) {
    commands.insert_resource(MascaraBlanca(imagenes.add(imagen_de_mascara(
        1,
        1,
        vec![255],
    ))));
}

fn imagen_de_mascara(ancho: usize, alto: usize, datos: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: ancho as u32,
            height: alto as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        datos,
        // Un solo canal de 8 bits: el shader lee `.r`.
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    )
}

/// La imagen de máscara de una superficie: la forma (o la superficie entera si no tiene)
/// rellena de blanco, menos sus huecos, sobre negro.
pub fn imagen_mascara(forma: Option<&[Punto]>, huecos: &[Vec<Punto>]) -> Image {
    imagen_de_mascara(
        LADO_MASCARA,
        LADO_MASCARA,
        rasterizar_con_huecos(forma, huecos, LADO_MASCARA, LADO_MASCARA),
    )
}

/// Los números que se le pasan al shader. `ShaderType` le enseña a Bevy a copiarlos
/// a la GPU con el formato (alineación y relleno) que el shader espera.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct ParametrosSuperficie {
    /// Matriz que lleva un punto de la pantalla a coordenadas de la imagen.
    pub inversa: Mat3,
    /// Rectángulo de la composición que se muestra, de 0 a 1.
    pub entrada_min: Vec2,
    pub entrada_max: Vec2,
    /// Borde suave: izquierda, derecha, arriba, abajo (fracciones de la superficie).
    pub borde: Vec4,
    /// Ajuste de la curva del borde.
    pub potencia: f32,
    /// 1 si la superficie tiene malla (las coordenadas salen de los vértices), 0 si no.
    pub usar_uv_vertice: u32,
    /// Calibración de color de la salida: ganancia de rojo, verde y azul, y brillo.
    pub color_ganancia: Vec4,
    /// Calibración de color: contraste y gamma (los otros dos valores no se usan).
    pub color_ajuste: Vec4,
}

impl ParametrosSuperficie {
    /// Parámetros para una homografía y un rectángulo de entrada.
    pub fn nuevos(
        h: &Homografia,
        entrada: Entrada,
        borde: &BordeSuave,
        con_malla: bool,
        color: &CorreccionColor,
    ) -> Self {
        ParametrosSuperficie {
            inversa: mat3_desde_filas(h.matriz_inversa()),
            entrada_min: Vec2::new(entrada[0], entrada[1]),
            entrada_max: Vec2::new(entrada[2], entrada[3]),
            borde: Vec4::new(borde.izquierda, borde.derecha, borde.arriba, borde.abajo),
            potencia: borde.potencia,
            usar_uv_vertice: u32::from(con_malla),
            color_ganancia: Vec4::new(color.rojo, color.verde, color.azul, color.brillo),
            color_ajuste: Vec4::new(color.contraste, color.gamma, 0.0, 0.0),
        }
    }
}

impl ParametrosSuperficie {
    /// Actualiza los parámetros cuando cambiaron las esquinas, la entrada o el borde.
    pub fn actualizar(
        &mut self,
        h: &Homografia,
        entrada: Entrada,
        borde: &BordeSuave,
        con_malla: bool,
        color: &CorreccionColor,
    ) {
        *self = Self::nuevos(h, entrada, borde, con_malla, color);
    }
}

/// `Mat3` de Bevy (y la `mat3x3` de WGSL) guarda sus datos por **columnas**; nuestra
/// matriz está escrita por filas. Construir desde columnas y trasponer las reconcilia.
fn mat3_desde_filas(filas: [[f32; 3]; 3]) -> Mat3 {
    Mat3::from_cols_array_2d(&filas).transpose()
}

/// Un material 2D con shader propio. `AsBindGroup` describe cómo cada campo se conecta
/// al shader: `#[uniform(0)]` es el binding 0, la textura es el 1 y su muestreador el 2
/// (coinciden con los `@binding` de `superficie.wesl`, dentro del grupo 2).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct MaterialSuperficie {
    #[uniform(0)]
    pub parametros: ParametrosSuperficie,
    #[texture(1)]
    #[sampler(2)]
    pub composicion: Handle<Image>,
    /// La forma de la superficie: una imagen de un canal (blanco = se dibuja).
    #[texture(3)]
    #[sampler(4)]
    pub mascara: Handle<Image>,
}

impl Material2d for MaterialSuperficie {
    /// Mezcla por transparencia: lo que la forma recorta deja ver lo que hay detrás.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("superficie.wesl")).with_source("embedded"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec3;

    #[test]
    fn la_matriz_por_columnas_aplica_lo_mismo_que_por_filas() {
        // Una matriz fácil de reconocer: suma 10 a x y 20 a y (traslación homogénea).
        let filas = [[1.0, 0.0, 10.0], [0.0, 1.0, 20.0], [0.0, 0.0, 1.0]];
        let m = mat3_desde_filas(filas);
        let r = m * Vec3::new(1.0, 2.0, 1.0);
        assert!((r.x - 11.0).abs() < 1e-6);
        assert!((r.y - 22.0).abs() < 1e-6);
        assert!((r.z - 1.0).abs() < 1e-6);
    }

    #[test]
    fn la_entrada_llega_a_los_parametros() {
        let esq = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];
        let h = Homografia::desde_esquinas(esq);
        let p = ParametrosSuperficie::nuevos(
            &h,
            [0.25, 0.1, 0.75, 0.9],
            &BordeSuave::default(),
            false,
            &CorreccionColor::default(),
        );
        assert_eq!(p.entrada_min, Vec2::new(0.25, 0.1));
        assert_eq!(p.entrada_max, Vec2::new(0.75, 0.9));
    }

    #[test]
    fn con_malla_el_shader_usa_las_coordenadas_de_los_vertices() {
        let esq = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];
        let h = Homografia::desde_esquinas(esq);
        let sin = ParametrosSuperficie::nuevos(
            &h,
            vm_core::ENTRADA_COMPLETA,
            &BordeSuave::default(),
            false,
            &CorreccionColor::default(),
        );
        let con = ParametrosSuperficie::nuevos(
            &h,
            vm_core::ENTRADA_COMPLETA,
            &BordeSuave::default(),
            true,
            &CorreccionColor::default(),
        );
        assert_eq!(sin.usar_uv_vertice, 0);
        assert_eq!(con.usar_uv_vertice, 1);
    }

    #[test]
    fn el_borde_llega_a_los_parametros() {
        let esq = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];
        let h = Homografia::desde_esquinas(esq);
        let borde = BordeSuave {
            izquierda: 0.1,
            derecha: 0.2,
            arriba: 0.3,
            abajo: 0.4,
            potencia: 1.5,
        };
        let p = ParametrosSuperficie::nuevos(
            &h,
            vm_core::ENTRADA_COMPLETA,
            &borde,
            false,
            &CorreccionColor::default(),
        );
        assert_eq!(p.borde, Vec4::new(0.1, 0.2, 0.3, 0.4));
        assert_eq!(p.potencia, 1.5);
    }

    #[test]
    fn los_parametros_llevan_una_esquina_a_su_uv() {
        let esq = [
            [-300.0, 200.0],
            [320.0, 180.0],
            [310.0, -190.0],
            [-290.0, -210.0],
        ];
        let h = Homografia::desde_esquinas(esq);
        let p = ParametrosSuperficie::nuevos(
            &h,
            vm_core::ENTRADA_COMPLETA,
            &BordeSuave::default(),
            false,
            &CorreccionColor::default(),
        );
        // La esquina inferior derecha de la pantalla debe caer en (u, v) = (1, 1).
        let r = p.inversa * Vec3::new(esq[2][0], esq[2][1], 1.0);
        assert!((r.x / r.z - 1.0).abs() < 1e-3);
        assert!((r.y / r.z - 1.0).abs() < 1e-3);
    }
}
