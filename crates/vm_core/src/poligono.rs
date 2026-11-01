//! Geometría de polígonos 2D. Rust puro: solo arrays, sin Bevy ni egui.
//!
//! Se usa para las **formas** de las superficies: un polígono (en coordenadas de la
//! superficie, de 0 a 1) que recorta lo que se dibuja. Se parte siempre de un rectángulo
//! (`rectangulo_unitario`) y el editor solo permite mover puntos y agregarlos sobre los
//! lados, rechazando todo movimiento que deje el polígono con lados cruzados
//! (`es_simple`). Así la forma siempre es válida y se puede rellenar sin ambigüedad.

pub type Punto = [f32; 2];

/// Margen para decidir si tres puntos están alineados. Las coordenadas de una forma van de
/// 0 a 1, así que esto es un área muy chica.
const TOLERANCIA: f32 = 1e-7;

/// Cuánto se separan, como mínimo, dos puntos consecutivos de una forma.
const SEPARACION_MINIMA: f32 = 1e-4;

/// El rectángulo que ocupa toda la superficie: la forma de partida.
/// Orden: sup-izq, sup-der, inf-der, inf-izq (igual que las esquinas de una superficie).
pub fn rectangulo_unitario() -> Vec<Punto> {
    vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
}

/// ¿El punto está dentro del polígono?
///
/// Regla par-impar: se traza un rayo desde el punto hacia la derecha y se cuentan los lados
/// que cruza. Si es impar, el punto está adentro.
pub fn contiene(poligono: &[Punto], p: Punto) -> bool {
    let n = poligono.len();
    if n < 3 {
        return false;
    }
    let mut dentro = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poligono[i], poligono[j]);
        // ¿El lado cruza la altura del punto, y el cruce queda a su derecha?
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            dentro = !dentro;
        }
        j = i;
    }
    dentro
}

/// Producto vectorial (*cruz*) de `ob` × `oc`: positivo si `c` queda a la izquierda de la
/// recta `o → b`, negativo si queda a la derecha, y cero si los tres están alineados.
fn cruz(o: Punto, b: Punto, c: Punto) -> f32 {
    (b[0] - o[0]) * (c[1] - o[1]) - (b[1] - o[1]) * (c[0] - o[0])
}

/// Hacia qué lado de la recta `a → b` queda `c`: `1`, `-1`, o `0` si están alineados.
fn orientacion(a: Punto, b: Punto, c: Punto) -> i32 {
    let v = cruz(a, b, c);
    if v.abs() < TOLERANCIA {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// Dado que `p` está alineado con el segmento `a–b`, ¿cae dentro de él?
fn sobre_segmento(a: Punto, b: Punto, p: Punto) -> bool {
    p[0] >= a[0].min(b[0]) - TOLERANCIA
        && p[0] <= a[0].max(b[0]) + TOLERANCIA
        && p[1] >= a[1].min(b[1]) - TOLERANCIA
        && p[1] <= a[1].max(b[1]) + TOLERANCIA
}

/// ¿Se cruzan (o se tocan) los segmentos `a–b` y `c–d`?
fn se_cruzan(a: Punto, b: Punto, c: Punto, d: Punto) -> bool {
    let (o1, o2) = (orientacion(a, b, c), orientacion(a, b, d));
    let (o3, o4) = (orientacion(c, d, a), orientacion(c, d, b));
    // Caso general: cada segmento deja los extremos del otro a lados opuestos.
    if o1 != o2 && o3 != o4 {
        return true;
    }
    // Casos especiales: un extremo cae sobre el otro segmento.
    (o1 == 0 && sobre_segmento(a, b, c))
        || (o2 == 0 && sobre_segmento(a, b, d))
        || (o3 == 0 && sobre_segmento(c, d, a))
        || (o4 == 0 && sobre_segmento(c, d, b))
}

/// ¿Es un polígono válido para usar de forma? Tiene al menos 3 puntos, ningún lado de
/// largo casi nulo, ningún lado que se doble sobre el anterior, y **ningún par de lados
/// se cruza ni se toca** (salvo los vecinos, en el punto que comparten).
pub fn es_simple(poligono: &[Punto]) -> bool {
    let n = poligono.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let (a, b) = (poligono[i], poligono[(i + 1) % n]);
        if (a[0] - b[0]).hypot(a[1] - b[1]) < SEPARACION_MINIMA {
            return false;
        }
    }
    for i in 0..n {
        let (a, b) = (poligono[i], poligono[(i + 1) % n]);
        for j in (i + 1)..n {
            let (c, d) = (poligono[j], poligono[(j + 1) % n]);
            let vecinos = j == i + 1 || (i == 0 && j == n - 1);
            if vecinos {
                // Comparten un vértice; el otro extremo de cada uno no puede caer sobre el
                // lado del vecino (sería un lado que se dobla hacia atrás sobre sí mismo).
                let (compartido, otro_i, otro_j) = if j == i + 1 { (b, a, d) } else { (a, b, c) };
                if (orientacion(compartido, otro_i, otro_j) == 0)
                    && (sobre_segmento(compartido, otro_i, otro_j)
                        || sobre_segmento(compartido, otro_j, otro_i))
                {
                    return false;
                }
            } else if se_cruzan(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// El lado del polígono más cercano a `p`: `(índice del lado, posición a lo largo del lado
/// de 0 a 1, distancia)`. El lado `i` va del punto `i` al punto `i + 1` (el último vuelve
/// al primero). Sirve para insertar un punto nuevo donde el usuario hizo doble clic.
pub fn lado_mas_cercano(poligono: &[Punto], p: Punto) -> Option<(usize, f32, f32)> {
    let n = poligono.len();
    if n < 2 {
        return None;
    }
    let mut mejor: Option<(usize, f32, f32)> = None;
    for i in 0..n {
        let (a, b) = (poligono[i], poligono[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let largo2 = dx * dx + dy * dy;
        let t = if largo2 < f32::EPSILON {
            0.0
        } else {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / largo2).clamp(0.0, 1.0)
        };
        let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
        let distancia = (p[0] - qx).hypot(p[1] - qy);
        if mejor.is_none_or(|(_, _, d)| distancia < d) {
            mejor = Some((i, t, distancia));
        }
    }
    mejor
}

/// Inserta un punto en el lado `lado`, a la fracción `t` (de 0 a 1) de su recorrido, y
/// devuelve el índice del punto nuevo. El lado `i` va del punto `i` al `i + 1`.
pub fn insertar_en_lado(poligono: &mut Vec<Punto>, lado: usize, t: f32) -> Option<usize> {
    let n = poligono.len();
    if lado >= n {
        return None;
    }
    let (a, b) = (poligono[lado], poligono[(lado + 1) % n]);
    let nuevo = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
    poligono.insert(lado + 1, nuevo);
    Some(lado + 1)
}

/// Cuántas líneas de muestreo se usan por fila de píxeles para suavizar los bordes.
const MUESTRAS_VERTICALES: usize = 8;

/// Rellena el polígono en una imagen de `ancho` × `alto` píxeles de un canal: `255` adentro
/// y `0` afuera, y valores intermedios en los bordes (cuánto del píxel cubre el polígono).
/// Esos grises hacen que el borde se vea suave en vez de escalonado ("dientes de sierra")
/// cuando el shader estira la imagen sobre la superficie.
///
/// Las coordenadas del polígono van de 0 a 1 (la imagen es la superficie entera). Cada fila
/// se recorre en `MUESTRAS_VERTICALES` alturas; en cada una se buscan los cruces con los
/// lados, se ordenan y se rellena entre cada par (regla par-impar) sumando, por píxel, la
/// fracción exacta que cubre el tramo en horizontal.
pub fn rasterizar(poligono: &[Punto], ancho: usize, alto: usize) -> Vec<u8> {
    let mut imagen = vec![0u8; ancho * alto];
    let n = poligono.len();
    if n < 3 || ancho == 0 || alto == 0 {
        return imagen;
    }
    let mut cruces: Vec<f32> = Vec::new();
    let mut cobertura = vec![0.0f32; ancho];
    for fila in 0..alto {
        cobertura.fill(0.0);
        for muestra in 0..MUESTRAS_VERTICALES {
            // Altura (de 0 a 1) del centro de esta línea de muestreo.
            let y =
                (fila as f32 + (muestra as f32 + 0.5) / MUESTRAS_VERTICALES as f32) / alto as f32;
            cruces.clear();
            for i in 0..n {
                let (a, b) = (poligono[i], poligono[(i + 1) % n]);
                // Semiabierto (`<=` de un lado, `>` del otro): un vértice exactamente a la
                // altura de la línea se cuenta una sola vez.
                if (a[1] <= y) != (b[1] <= y) {
                    cruces.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            cruces.sort_by(f32::total_cmp);
            for par in cruces.as_chunks::<2>().0 {
                // El tramo, en unidades de píxel y dentro de la imagen.
                let desde = (par[0] * ancho as f32).clamp(0.0, ancho as f32);
                let hasta = (par[1] * ancho as f32).clamp(0.0, ancho as f32);
                if hasta <= desde {
                    continue;
                }
                let primera = desde.floor() as usize;
                let ultima = (hasta.ceil() as usize).min(ancho);
                for (columna, c) in cobertura[primera..ultima].iter_mut().enumerate() {
                    let columna = (primera + columna) as f32;
                    *c += (hasta.min(columna + 1.0) - desde.max(columna)).max(0.0);
                }
            }
        }
        for (columna, c) in cobertura.iter().enumerate() {
            let fraccion = (c / MUESTRAS_VERTICALES as f32).clamp(0.0, 1.0);
            imagen[fila * ancho + columna] = (fraccion * 255.0).round() as u8;
        }
    }
    imagen
}

/// Rasteriza una figura con **huecos**: la `forma` (o la superficie entera si no hay) menos
/// cada uno de los `huecos`. Como en `rasterizar`, cada píxel vale 255 si está adentro, 0 si
/// está afuera y un valor intermedio en los bordes (cobertura): en un borde de hueco, lo que
/// el hueco cubre se resta de lo que había.
pub fn rasterizar_con_huecos(
    forma: Option<&[Punto]>,
    huecos: &[Vec<Punto>],
    ancho: usize,
    alto: usize,
) -> Vec<u8> {
    let mut imagen = match forma {
        Some(f) => rasterizar(f, ancho, alto),
        None => vec![255u8; ancho * alto],
    };
    for hueco in huecos {
        let cobertura = rasterizar(hueco, ancho, alto);
        for (pixel, c) in imagen.iter_mut().zip(cobertura) {
            // 255 · (1 − c/255): lo que el hueco deja sin cubrir.
            *pixel = (*pixel as u32 * (255 - c as u32) / 255) as u8;
        }
    }
    imagen
}

/// El área con signo del polígono (positiva si da la vuelta en sentido antihorario).
pub fn area_con_signo(poligono: &[Punto]) -> f32 {
    let n = poligono.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poligono[i], poligono[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        / 2.0
}

/// Divide un polígono **simple** (sin lados que se crucen), convexo o no, en triángulos, por
/// el método de las "orejas": se va recortando un vértice cuyo triángulo con sus vecinos
/// queda adentro y no contiene a otro vértice. Devuelve los índices de los vértices de cada
/// triángulo (`n − 2` triángulos). Si el polígono no es simple o tiene menos de 3 puntos,
/// devuelve una lista vacía.
pub fn triangular(poligono: &[Punto]) -> Vec<[usize; 3]> {
    let n = poligono.len();
    if n < 3 || !es_simple(poligono) {
        return Vec::new();
    }
    // Se recorre siempre en sentido antihorario.
    let mut indices: Vec<usize> = (0..n).collect();
    if area_con_signo(poligono) < 0.0 {
        indices.reverse();
    }
    let mut triangulos = Vec::with_capacity(n - 2);
    let punto = |i: usize| poligono[i];
    let dentro_del_triangulo = |p: Punto, a: Punto, b: Punto, c: Punto| {
        cruz(a, b, p) >= 0.0 && cruz(b, c, p) >= 0.0 && cruz(c, a, p) >= 0.0
    };

    while indices.len() > 3 {
        let m = indices.len();
        let mut recortada = false;
        for k in 0..m {
            let (ia, ib, ic) = (indices[(k + m - 1) % m], indices[k], indices[(k + 1) % m]);
            let (a, b, c) = (punto(ia), punto(ib), punto(ic));
            // Un vértice "hacia afuera" (reflejo) no puede ser una oreja.
            if cruz(a, b, c) <= 0.0 {
                continue;
            }
            // Ningún otro vértice puede estar dentro de ese triángulo.
            let hay_otro = indices
                .iter()
                .any(|&j| j != ia && j != ib && j != ic && dentro_del_triangulo(punto(j), a, b, c));
            if hay_otro {
                continue;
            }
            triangulos.push([ia, ib, ic]);
            indices.remove(k);
            recortada = true;
            break;
        }
        if !recortada {
            // Un polígono raro (casi degenerado): se corta por donde se pueda.
            let k = 0;
            let (ia, ib, ic) = (indices[m - 1], indices[k], indices[1]);
            triangulos.push([ia, ib, ic]);
            indices.remove(k);
        }
    }
    triangulos.push([indices[0], indices[1], indices[2]]);
    triangulos
}

/// El centro de un conjunto de puntos: el promedio de sus coordenadas.
pub fn centroide(puntos: &[Punto]) -> Punto {
    if puntos.is_empty() {
        return [0.0, 0.0];
    }
    let n = puntos.len() as f32;
    let suma = puntos
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [suma[0] / n, suma[1] / n]
}

/// Cambia el tamaño de un conjunto de puntos, `factor` veces, alrededor de `centro` (que
/// queda quieto). Con `factor` 2 todo se aleja al doble; con 0,5 se acerca a la mitad.
/// Es una escala uniforme: las proporciones no cambian.
pub fn escalar(puntos: &[Punto], centro: Punto, factor: f32) -> Vec<Punto> {
    puntos
        .iter()
        .map(|p| {
            [
                centro[0] + (p[0] - centro[0]) * factor,
                centro[1] + (p[1] - centro[1]) * factor,
            ]
        })
        .collect()
}

/// Desplaza todos los puntos en `delta`.
pub fn trasladar(puntos: &[Punto], delta: Punto) -> Vec<Punto> {
    puntos
        .iter()
        .map(|p| [p[0] + delta[0], p[1] + delta[1]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forma_en_l() -> Vec<Punto> {
        // Una "L": cóncava, pero válida.
        vec![
            [0.0, 0.0],
            [0.5, 0.0],
            [0.5, 0.5],
            [1.0, 0.5],
            [1.0, 1.0],
            [0.0, 1.0],
        ]
    }

    #[test]
    fn el_rectangulo_unitario_es_valido_y_contiene_el_centro() {
        let r = rectangulo_unitario();
        assert!(es_simple(&r));
        assert!(contiene(&r, [0.5, 0.5]));
        assert!(!contiene(&r, [1.5, 0.5]));
    }

    #[test]
    fn contiene_funciona_con_una_forma_concava() {
        let l = forma_en_l();
        assert!(contiene(&l, [0.25, 0.25])); // brazo vertical
        assert!(contiene(&l, [0.75, 0.75])); // brazo horizontal
        assert!(!contiene(&l, [0.75, 0.25])); // el hueco de la L
    }

    #[test]
    fn una_forma_en_l_es_simple() {
        assert!(es_simple(&forma_en_l()));
    }

    #[test]
    fn un_mono_de_lados_cruzados_no_es_simple() {
        // Esquinas 3 y 4 intercambiadas: los lados se cruzan formando un "moño".
        let mono = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        assert!(!es_simple(&mono));
    }

    #[test]
    fn un_punto_empujado_a_traves_del_poligono_lo_invalida() {
        // La esquina inferior derecha del rectángulo, arrastrada hacia la izquierda más
        // allá de la inferior izquierda: el lado de abajo cruza el de la izquierda.
        let r = vec![[0.0, 0.0], [1.0, 0.0], [-0.5, 1.0], [0.0, 1.0]];
        assert!(!es_simple(&r));
    }

    #[test]
    fn menos_de_tres_puntos_no_es_valido() {
        assert!(!es_simple(&[[0.0, 0.0], [1.0, 1.0]]));
        assert!(!es_simple(&[]));
    }

    #[test]
    fn dos_puntos_casi_iguales_no_son_validos() {
        let p = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.00001], [0.0, 1.0]];
        assert!(!es_simple(&p));
    }

    #[test]
    fn un_lado_que_se_dobla_hacia_atras_no_es_valido() {
        // El punto 2 vuelve sobre el lado anterior (0,0)-(1,0): un "pico" de ancho cero.
        let p = vec![[0.0, 0.0], [1.0, 0.0], [0.5, 0.0], [0.5, 1.0]];
        assert!(!es_simple(&p));
    }

    #[test]
    fn tres_puntos_alineados_en_el_mismo_sentido_si_son_validos() {
        // Un punto agregado justo sobre un lado no cambia la forma.
        let p = vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!(es_simple(&p));
    }

    #[test]
    fn el_lado_mas_cercano_y_su_posicion() {
        let r = rectangulo_unitario();
        // Cerca del lado de arriba (lado 0: de (0,0) a (1,0)), a un cuarto de su recorrido.
        let (lado, t, distancia) = lado_mas_cercano(&r, [0.25, 0.05]).unwrap();
        assert_eq!(lado, 0);
        assert!((t - 0.25).abs() < 1e-6);
        assert!((distancia - 0.05).abs() < 1e-6);
        // Cerca del lado que cierra la forma (lado 3: de (0,1) a (0,0)).
        let (lado, _, _) = lado_mas_cercano(&r, [0.02, 0.5]).unwrap();
        assert_eq!(lado, 3);
    }

    #[test]
    fn insertar_en_un_lado_agrega_el_punto_en_el_lugar_correcto() {
        let mut r = rectangulo_unitario();
        let i = insertar_en_lado(&mut r, 1, 0.5).unwrap();
        assert_eq!(i, 2);
        assert_eq!(r.len(), 5);
        assert_eq!(r[2], [1.0, 0.5]); // a mitad del lado derecho
        assert!(es_simple(&r));
        // Insertar en el último lado (el que vuelve al primer punto).
        let mut r = rectangulo_unitario();
        assert_eq!(insertar_en_lado(&mut r, 3, 0.5), Some(4));
        assert_eq!(r[4], [0.0, 0.5]);
        assert_eq!(insertar_en_lado(&mut r, 99, 0.5), None);
    }

    #[test]
    fn rasterizar_el_rectangulo_unitario_llena_todo() {
        let img = rasterizar(&rectangulo_unitario(), 8, 8);
        assert!(img.iter().all(|&p| p == 255));
    }

    #[test]
    fn rasterizar_la_mitad_izquierda() {
        let mitad = vec![[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]];
        let img = rasterizar(&mitad, 8, 8);
        for fila in 0..8 {
            for columna in 0..8 {
                let esperado = if columna < 4 { 255 } else { 0 };
                assert_eq!(
                    img[fila * 8 + columna],
                    esperado,
                    "fila {fila}, col {columna}"
                );
            }
        }
    }

    #[test]
    fn rasterizar_un_triangulo_llena_cerca_de_la_mitad() {
        let triangulo = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let img = rasterizar(&triangulo, 64, 64);
        let llenos = img.iter().filter(|&&p| p == 255).count();
        let fraccion = llenos as f32 / (64.0 * 64.0);
        assert!((fraccion - 0.5).abs() < 0.03, "llenó {fraccion}");
        // La esquina de la diagonal opuesta queda vacía.
        assert_eq!(img[63 * 64 + 63], 0);
        assert_eq!(img[0], 255);
    }

    #[test]
    fn rasterizar_respeta_el_hueco_de_la_l() {
        let img = rasterizar(&forma_en_l(), 8, 8);
        assert_eq!(img[2 * 8 + 2], 255); // brazo vertical
        assert_eq!(img[2 * 8 + 6], 0); // el hueco: arriba a la derecha
        assert_eq!(img[6 * 8 + 6], 255); // brazo horizontal
    }

    #[test]
    fn rasterizar_algo_invalido_deja_todo_vacio() {
        assert!(
            rasterizar(&[[0.0, 0.0], [1.0, 1.0]], 4, 4)
                .iter()
                .all(|&p| p == 0)
        );
    }

    #[test]
    fn el_centroide_es_el_promedio() {
        assert_eq!(centroide(&rectangulo_unitario()), [0.5, 0.5]);
        assert_eq!(centroide(&[[0.0, 0.0], [4.0, 2.0]]), [2.0, 1.0]);
        assert_eq!(centroide(&[]), [0.0, 0.0]);
    }

    #[test]
    fn escalar_deja_quieto_el_centro_y_agranda_o_achica_el_resto() {
        let cuadrado = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];
        let doble = escalar(&cuadrado, [0.0, 0.0], 2.0);
        assert_eq!(doble[1], [2.0, 2.0]);
        let mitad = escalar(&cuadrado, [0.0, 0.0], 0.5);
        assert_eq!(mitad[3], [-0.5, -0.5]);
        // Un centro que no es el origen: ese punto no se mueve.
        let desde_la_esquina = escalar(&cuadrado, [-1.0, -1.0], 2.0);
        assert_eq!(desde_la_esquina[3], [-1.0, -1.0]);
        assert_eq!(desde_la_esquina[1], [3.0, 3.0]);
    }

    #[test]
    fn escalar_conserva_la_forma() {
        // Las proporciones y la convexidad no cambian con una escala uniforme.
        let trapecio = [[-3.0, 2.0], [4.0, 1.0], [3.0, -2.0], [-4.0, -3.0]];
        let centro = centroide(&trapecio);
        let grande = escalar(&trapecio, centro, 1.7);
        assert!(es_simple(&grande));
        // El cociente entre dos lados cualesquiera se mantiene.
        let lado = |p: &[Punto], i: usize| {
            let (a, b) = (p[i], p[(i + 1) % 4]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        };
        let antes = lado(&trapecio, 0) / lado(&trapecio, 1);
        let despues = lado(&grande, 0) / lado(&grande, 1);
        assert!((antes - despues).abs() < 1e-4);
        // Y el centro sigue siendo el mismo.
        let nuevo_centro = centroide(&grande);
        assert!((nuevo_centro[0] - centro[0]).abs() < 1e-4);
        assert!((nuevo_centro[1] - centro[1]).abs() < 1e-4);
    }

    #[test]
    fn trasladar_desplaza_todos_los_puntos() {
        let t = trasladar(&[[0.0, 0.0], [1.0, 2.0]], [0.5, -1.0]);
        assert_eq!(t, vec![[0.5, -1.0], [1.5, 1.0]]);
    }

    #[test]
    fn rasterizar_suaviza_los_bordes_en_diagonal() {
        // Un triángulo con una diagonal: en el borde tiene que haber grises, no solo 0 y 255.
        let triangulo = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let img = rasterizar(&triangulo, 32, 32);
        let grises = img.iter().filter(|p| **p > 0 && **p < 255).count();
        assert!(grises >= 32, "pocos píxeles de borde suavizado: {grises}");
        // La suma de coberturas es el área: la mitad del cuadrado.
        let area: f32 = img.iter().map(|p| *p as f32 / 255.0).sum::<f32>() / (32.0 * 32.0);
        assert!((area - 0.5).abs() < 0.01, "área {area}");
    }

    #[test]
    fn rasterizar_un_borde_a_mitad_de_pixel_da_gris_medio() {
        // La mitad izquierda hasta x = 0,5625 en una imagen de 8 de ancho: la columna 4
        // queda cubierta en un 50 %.
        let forma = vec![[0.0, 0.0], [0.5625, 0.0], [0.5625, 1.0], [0.0, 1.0]];
        let img = rasterizar(&forma, 8, 8);
        assert_eq!(img[3], 255);
        assert!((img[4] as i32 - 128).abs() <= 1, "{}", img[4]);
        assert_eq!(img[5], 0);
    }

    fn area_de_triangulos(p: &[Punto], t: &[[usize; 3]]) -> f32 {
        t.iter()
            .map(|[a, b, c]| area_con_signo(&[p[*a], p[*b], p[*c]]).abs())
            .sum()
    }

    #[test]
    fn un_cuadrado_se_divide_en_dos_triangulos_que_suman_su_area() {
        let cuadrado = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let t = triangular(&cuadrado);
        assert_eq!(t.len(), 2);
        assert!((area_de_triangulos(&cuadrado, &t) - 16.0).abs() < 1e-4);
    }

    #[test]
    fn un_poligono_concavo_en_l_se_triangula_sin_salirse() {
        // Una L: área = 3 cuadrados de 1×1.
        let l = vec![
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ];
        let t = triangular(&l);
        assert_eq!(t.len(), 4); // n − 2
        assert!((area_de_triangulos(&l, &t) - 3.0).abs() < 1e-4);
        // El centro de cada triángulo cae dentro de la L.
        for [a, b, c] in &t {
            let centro = centroide(&[l[*a], l[*b], l[*c]]);
            assert!(contiene(&l, centro), "triángulo fuera: {centro:?}");
        }
    }

    #[test]
    fn el_sentido_de_giro_no_importa() {
        let horario = vec![[0.0, 0.0], [0.0, 3.0], [3.0, 3.0], [3.0, 0.0]];
        let t = triangular(&horario);
        assert_eq!(t.len(), 2);
        assert!((area_de_triangulos(&horario, &t) - 9.0).abs() < 1e-4);
        assert!(area_con_signo(&horario) < 0.0);
    }

    #[test]
    fn triangular_algo_invalido_da_una_lista_vacia() {
        assert!(triangular(&[[0.0, 0.0], [1.0, 1.0]]).is_empty());
        // Un moño (lados que se cruzan).
        let mono = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]];
        assert!(triangular(&mono).is_empty());
    }

    #[test]
    fn un_hueco_se_resta_de_la_figura() {
        // Superficie entera menos un hueco central de la mitad del ancho y alto.
        let hueco = vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]];
        let img = rasterizar_con_huecos(None, &[hueco], 8, 8);
        assert_eq!(img[0], 255); // esquina: afuera del hueco
        assert_eq!(img[3 * 8 + 3], 0); // centro: adentro del hueco
        let cubierto = img.iter().filter(|p| **p == 0).count();
        assert_eq!(cubierto, 16); // 4×4 píxeles
    }

    #[test]
    fn un_hueco_no_agrega_nada_afuera_de_la_forma() {
        // La forma es la mitad izquierda; el hueco cae en la mitad derecha (donde ya no hay nada).
        let mitad = vec![[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]];
        let hueco = vec![[0.6, 0.2], [0.9, 0.2], [0.9, 0.8], [0.6, 0.8]];
        let con = rasterizar_con_huecos(Some(&mitad), &[hueco], 8, 8);
        let sin = rasterizar(&mitad, 8, 8);
        assert_eq!(con, sin);
    }

    #[test]
    fn sin_forma_ni_huecos_se_dibuja_todo() {
        let img = rasterizar_con_huecos(None, &[], 4, 4);
        assert!(img.iter().all(|p| *p == 255));
    }
}
