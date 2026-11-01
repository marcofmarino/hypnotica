// Shader de una capa de la composición: muestra el cuadro del video con su opacidad.
//
// La GPU combina lo que devuelve este shader con lo que ya está dibujado debajo, con una
// función de mezcla fija que se elige según el modo (ver `material_capa.rs`). Por eso acá
// la opacidad se "pliega" en el color de la manera que cada mezcla necesita:
//
//   0 Normal      devuelve (color, alfa·opacidad)      la GPU hace  S·α + D·(1−α)
//   1 Suma        devuelve  color·a                    la GPU hace  S + D
//   2 Multiplicar devuelve  1 − a + a·color            la GPU hace  S · D
//   3 Pantalla    devuelve  color·a                    la GPU hace  S + D·(1−S)
//
// donde a = alfa del cuadro · opacidad de la capa.

import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;

struct ParametrosCapa {
    opacidad: f32,
    modo: u32,
    // 1: el cuadro se trata como opaco (el alfa de una captura por DMA-BUF no vale).
    ignorar_alfa: u32,
}

@group(2) @binding(0) var<uniform> parametros: ParametrosCapa;
@group(2) @binding(1) var textura: texture_2d<f32>;
@group(2) @binding(2) var muestreador: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let cuadro = textureSample(textura, muestreador, in.uv);
    let alfa_del_cuadro = select(cuadro.a, 1.0, parametros.ignorar_alfa != 0u);
    let a = alfa_del_cuadro * parametros.opacidad;

    if parametros.modo == 0u {
        return vec4<f32>(cuadro.rgb, a);
    }
    if parametros.modo == 2u {
        return vec4<f32>(mix(vec3<f32>(1.0), cuadro.rgb, a), 1.0);
    }
    // Suma (1) y Pantalla (3): el color ya multiplicado por su opacidad.
    return vec4<f32>(cuadro.rgb * a, 1.0);
}
