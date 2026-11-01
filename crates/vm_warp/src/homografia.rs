#[derive(Debug, Clone, Copy)]
pub struct Homografia {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
    g: f32,
    h: f32,
}

impl Homografia {
    pub fn desde_esquinas(p: [[f32; 2]; 4]) -> Self {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = p;

        let dx1 = x1 - x2;
        let dx2 = x3 - x2;
        let dx3 = x0 - x1 + x2 - x3;
        let dy1 = y1 - y2;
        let dy2 = y3 - y2;
        let dy3 = y0 - y1 + y2 - y3;

        if dx3.abs() < 1e-6 && dy3.abs() < 1e-6 {
            Self {
                a: x1 - x0,
                b: x3 - x0,
                c: x0,
                d: y1 - y0,
                e: y3 - y0,
                f: y0,
                g: 0.0,
                h: 0.0,
            }
        } else {
            let den = dx1 * dy2 - dy1 * dx2;
            let g = (dx3 * dy2 - dy3 * dx2) / den;
            let h = (dx1 * dy3 - dy1 * dx3) / den;
            Self {
                a: x1 - x0 + g * x1,
                b: x3 - x0 + h * x3,
                c: x0,
                d: y1 - y0 + g * y1,
                e: y3 - y0 + h * y3,
                f: y0,
                g,
                h,
            }
        }
    }

    /// La matriz **inversa** de la homografía, en filas (`[[f32; 3]; 3]`).
    ///
    /// La homografía es una matriz 3×3 que lleva `(u, v, 1)` a `(x·w, y·w, w)`. Su inversa
    /// hace el camino contrario: dado un punto de la pantalla `(x, y, 1)`, devuelve
    /// `(u·s, v·s, s)`, y dividiendo por `s` se obtiene el punto `(u, v)` de la imagen.
    /// El shader la usa para saber, en cada píxel, qué parte de la imagen le toca.
    ///
    /// Se calcula con la matriz *adjunta* (la traspuesta de los cofactores) dividida por
    /// el determinante.
    pub fn matriz_inversa(&self) -> [[f32; 3]; 3] {
        let (a, b, c) = (self.a, self.b, self.c);
        let (d, e, f) = (self.d, self.e, self.f);
        let (g, h, i) = (self.g, self.h, 1.0);

        // Cofactores de cada elemento.
        let ca = e * i - f * h;
        let cb = -(d * i - f * g);
        let cc = d * h - e * g;
        let cd = -(b * i - c * h);
        let ce = a * i - c * g;
        let cf = -(a * h - b * g);
        let cg = b * f - c * e;
        let ch = -(a * f - c * d);
        let ci = a * e - b * d;

        let det = a * ca + b * cb + c * cc;
        [
            [ca / det, cd / det, cg / det],
            [cb / det, ce / det, ch / det],
            [cc / det, cf / det, ci / det],
        ]
    }

    /// Lo inverso de `proyectar`: dado un punto de la pantalla, devuelve su `(u, v)` en la
    /// imagen. El editor lo usa para saber qué punto de la superficie está bajo el mouse.
    pub fn desproyectar(&self, x: f32, y: f32) -> [f32; 2] {
        let m = self.matriz_inversa();
        let u = m[0][0] * x + m[0][1] * y + m[0][2];
        let v = m[1][0] * x + m[1][1] * y + m[1][2];
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        [u / w, v / w]
    }

    pub fn proyectar(&self, u: f32, v: f32) -> [f32; 2] {
        // Calcular la proyección de un punto en la
        // pantalla.
        let w = self.g * u + self.h * v + 1.0;
        [
            (self.a * u + self.b * v + self.c) / w,
            (self.d * u + self.e * v + self.f) / w,
        ]
    }
}

/// Dice si el cuadrilátero es **convexo** (sin ángulos "hacia adentro" ni lados cruzados).
///
/// La homografía solo es válida para cuadriláteros convexos: si no lo es, el divisor
/// `w = g·u + h·v + 1` se anula dentro de la imagen y la malla se rompe. Se comprueba
/// con el producto vectorial (*cruz*) de cada par de lados consecutivos: su signo dice
/// hacia qué lado gira el contorno en esa esquina. Si las cuatro esquinas giran
/// hacia el mismo lado, es convexo.
pub fn es_convexo(p: [[f32; 2]; 4]) -> bool {
    let mut positivos = 0;
    let mut negativos = 0;
    for i in 0..4 {
        let a = p[i];
        let b = p[(i + 1) % 4];
        let c = p[(i + 2) % 4];
        let cruz = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        // Un giro casi nulo (tres puntos casi alineados) tampoco sirve: es degenerado.
        if cruz > 1e-3 {
            positivos += 1;
        } else if cruz < -1e-3 {
            negativos += 1;
        } else {
            return false;
        }
    }
    positivos == 4 || negativos == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aplica una matriz 3×3 (en filas) a un punto 2D y divide por la coordenada homogénea.
    fn aplicar(m: [[f32; 3]; 3], x: f32, y: f32) -> [f32; 2] {
        let px = m[0][0] * x + m[0][1] * y + m[0][2];
        let py = m[1][0] * x + m[1][1] * y + m[1][2];
        let pz = m[2][0] * x + m[2][1] * y + m[2][2];
        [px / pz, py / pz]
    }

    #[test]
    fn la_inversa_deshace_la_proyeccion() {
        // Un cuadrilátero con perspectiva fuerte, en píxeles de una pantalla grande.
        let esq = [
            [-700.0, 400.0],
            [900.0, 250.0],
            [650.0, -420.0],
            [-820.0, -300.0],
        ];
        let h = Homografia::desde_esquinas(esq);
        let inversa = h.matriz_inversa();
        for (u, v) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (0.3, 0.7),
            (0.5, 0.5),
        ] {
            let [x, y] = h.proyectar(u, v);
            let [ru, rv] = aplicar(inversa, x, y);
            assert!((ru - u).abs() < 1e-3, "u: esperado {u}, salió {ru}");
            assert!((rv - v).abs() < 1e-3, "v: esperado {v}, salió {rv}");
        }
    }

    #[test]
    fn desproyectar_es_lo_inverso_de_proyectar() {
        let esq = [
            [-700.0, 400.0],
            [900.0, 250.0],
            [650.0, -420.0],
            [-820.0, -300.0],
        ];
        let h = Homografia::desde_esquinas(esq);
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.25, 0.8), (0.5, 0.5), (0.9, 0.1)] {
            let [x, y] = h.proyectar(u, v);
            let [ru, rv] = h.desproyectar(x, y);
            assert!(
                (ru - u).abs() < 1e-3 && (rv - v).abs() < 1e-3,
                "({u},{v}) -> ({ru},{rv})"
            );
        }
    }

    #[test]
    fn la_inversa_funciona_con_un_paralelogramo() {
        // Sin perspectiva (g = h = 0): es la rama especial de `desde_esquinas`.
        let esq = [[-4.0, 2.0], [4.0, 2.0], [4.0, -2.0], [-4.0, -2.0]];
        let h = Homografia::desde_esquinas(esq);
        let [u, v] = aplicar(h.matriz_inversa(), 2.0, 1.0);
        assert!((u - 0.75).abs() < 1e-5);
        assert!((v - 0.25).abs() < 1e-5);
    }

    #[test]
    fn un_rectangulo_es_convexo() {
        assert!(es_convexo([
            [-1.0, 1.0],
            [1.0, 1.0],
            [1.0, -1.0],
            [-1.0, -1.0]
        ]));
    }

    #[test]
    fn un_trapecio_torcido_es_convexo() {
        assert!(es_convexo([
            [-3.0, 2.0],
            [4.0, 1.0],
            [3.0, -2.0],
            [-4.0, -3.0]
        ]));
    }

    #[test]
    fn una_esquina_hacia_adentro_no_es_convexo() {
        // La esquina inferior derecha se metió dentro del cuadrilátero.
        assert!(!es_convexo([
            [-1.0, 1.0],
            [1.0, 1.0],
            [-0.5, 0.5],
            [-1.0, -1.0]
        ]));
    }

    #[test]
    fn lados_cruzados_no_es_convexo() {
        // Esquinas 2 y 3 intercambiadas: el contorno se cruza en forma de "moño".
        assert!(!es_convexo([
            [-1.0, 1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, -1.0]
        ]));
    }

    #[test]
    fn tres_puntos_alineados_no_es_convexo() {
        assert!(!es_convexo([
            [-1.0, 1.0],
            [0.0, 1.0],
            [1.0, 1.0],
            [0.0, -1.0]
        ]));
    }

    #[test]
    fn las_esquinas_caen_donde_deben() {
        let esq = [[-3.0, 2.0], [4.0, 1.0], [3.0, -2.0], [-4.0, -3.0]];
        let h = Homografia::desde_esquinas(esq);
        let casos = [
            (0.0, 0.0, esq[0]),
            (1.0, 0.0, esq[1]),
            (1.0, 1.0, esq[2]),
            (0.0, 1.0, esq[3]),
        ];
        for (u, v, esperado) in casos {
            let r = h.proyectar(u, v);
            assert!((r[0] - esperado[0]).abs() < 1e-4);
            assert!((r[1] - esperado[1]).abs() < 1e-4);
        }
    }
}
