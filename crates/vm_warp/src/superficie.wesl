// Shader de una superficie: deforma la composición con una homografía exacta, por píxel.
//
// En lugar de llevar la imagen a la pantalla, hacemos el camino inverso: para cada píxel
// del proyector preguntamos "¿qué punto de la imagen cae acá?". La matriz `inversa` (que
// calcula la CPU cada vez que cambian las esquinas) responde esa pregunta.

import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;

struct Parametros {
    // Lleva un punto de la pantalla (x, y, 1) a coordenadas de la imagen (u·s, v·s, s).
    inversa: mat3x3<f32>,
    // Rectángulo de la composición que muestra esta superficie, en coordenadas 0..1.
    // (0,0)-(1,1) es la composición entera.
    entrada_min: vec2<f32>,
    entrada_max: vec2<f32>,
    // Borde suave (edge blending): ancho de cada lado como fracción de la superficie, en el
    // orden (izquierda, derecha, arriba, abajo).
    borde: vec4<f32>,
    // Ajusta la curva del borde (1 = neutra).
    potencia: f32,
    // 1 si la superficie tiene malla (mesh warp): las coordenadas de la superficie salen de
    // los vértices; 0 si no: salen de la homografía inversa, exacta por píxel.
    usar_uv_vertice: u32,
    // Calibración de color de la salida (ver `vm_core::color`): ganancia de rojo, verde y
    // azul, y brillo; y, aparte, contraste y gamma.
    color_ganancia: vec4<f32>,
    color_ajuste: vec4<f32>,
}

// La rampa de un borde: 0 en el extremo, 1 a `ancho` de distancia, con curva suave. Con
// esta curva, `rampa(x) + rampa(1 - x) = 1`: dos proyectores con rampas espejadas suman
// una luz constante en la zona donde se solapan. Un ancho nulo significa "sin borde".
fn rampa(distancia: f32, ancho: f32) -> f32 {
    if ancho <= 0.0 {
        return 1.0;
    }
    let t = clamp(distancia / ancho, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Entre lineal y sRGB (el espacio de los valores que se ven y se miden). La calibración se
// hace en sRGB: es la misma cuenta que `CorreccionColor::aplicar` en Rust.
fn a_srgb(lineal: vec3<f32>) -> vec3<f32> {
    let c = clamp(lineal, vec3<f32>(0.0), vec3<f32>(1.0));
    let bajo = c * 12.92;
    let alto = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(alto, bajo, c <= vec3<f32>(0.0031308));
}

fn a_lineal(srgb: vec3<f32>) -> vec3<f32> {
    let bajo = srgb / 12.92;
    let alto = pow((srgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(alto, bajo, srgb <= vec3<f32>(0.04045));
}

// Ganancia por canal, contraste y brillo, y gamma, en ese orden.
fn calibrar(lineal: vec3<f32>) -> vec3<f32> {
    let ganancia = parametros.color_ganancia.xyz;
    let brillo = parametros.color_ganancia.w;
    let contraste = parametros.color_ajuste.x;
    let gamma = parametros.color_ajuste.y;
    var c = a_srgb(lineal) * ganancia;
    c = (c - vec3<f32>(0.5)) * contraste + vec3<f32>(0.5) + vec3<f32>(brillo);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    c = pow(c, vec3<f32>(1.0 / gamma));
    return a_lineal(c);
}

@group(2) @binding(0) var<uniform> parametros: Parametros;
@group(2) @binding(1) var composicion: texture_2d<f32>;
@group(2) @binding(2) var muestreador: sampler;
// Forma de la superficie: blanco donde se dibuja, negro donde no (en coordenadas de la
// superficie, así que se deforma junto con ella).
@group(2) @binding(3) var mascara: texture_2d<f32>;
@group(2) @binding(4) var muestreador_mascara: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Coordenadas (u, v) de este píxel sobre la superficie, de 0 a 1.
    var uv: vec2<f32>;
    if parametros.usar_uv_vertice == 1u {
        // Con malla, la superficie es una grilla densa de triángulos y cada vértice trae
        // sus coordenadas: la GPU las interpola entre vértices.
        uv = in.uv;
    } else {
        // Sin malla, hay un solo rectángulo y se hace el camino inverso por píxel: la matriz
        // `inversa` lleva la posición del píxel en el mundo 2D (el mismo sistema que las
        // esquinas del proyecto) a coordenadas de la imagen. La división por la coordenada
        // homogénea es lo que produce la perspectiva.
        let punto = parametros.inversa * vec3<f32>(in.world_position.xy, 1.0);
        uv = punto.xy / punto.z;
    }

    // De "0..1 sobre la superficie" a "qué parte de la composición".
    let uv_composicion = mix(parametros.entrada_min, parametros.entrada_max, uv);

    let color = textureSample(composicion, muestreador, uv_composicion);

    // Lo que queda fuera de la forma se vuelve transparente (deja ver lo que hay debajo).
    let dentro = textureSample(mascara, muestreador_mascara, uv).r;

    // El borde suave se aplica en las coordenadas de la superficie: se deforma con ella.
    let producto = rampa(uv.x, parametros.borde.x)
        * rampa(1.0 - uv.x, parametros.borde.y)
        * rampa(uv.y, parametros.borde.z)
        * rampa(1.0 - uv.y, parametros.borde.w);
    let atenuacion = pow(producto, parametros.potencia);

    return vec4<f32>(calibrar(color.rgb), color.a * dentro * atenuacion);
}
