//! La malla de puntos de control de una superficie (*mesh warp*). Rust puro.
//!
//! Una superficie se dibuja deformada por un corner pin (sus 4 esquinas). La malla agrega,
//! encima, una grilla de **puntos de control**, cada uno con un *desplazamiento*: cuánto se
//! corre ese punto respecto de donde lo pondría el corner pin. Entre los puntos se
//! interpola con una curva cúbica **monótona** (bicúbica; ver `hermite_monotona`) que pasa
//! exactamente por cada punto de control, es suave entre ellos y nunca se pasa de los
//! puntos vecinos: da superficies curvas (una columna, un domo) en lugar de tramos rectos,
//! sin rebotes.
//!
//! Los 4 puntos de las **esquinas** de la malla siempre valen cero: las esquinas de la
//! superficie son las del corner pin, y se editan allí.

use serde::{Deserialize, Serialize};

use crate::poligono::Punto;

/// La menor y la mayor cantidad de puntos de control por lado.
pub const PUNTOS_MINIMOS: usize = 2;
pub const PUNTOS_MAXIMOS: usize = 16;

/// La grilla de desplazamientos. `desplazamientos` guarda `columnas × filas` puntos,
/// fila por fila (primero toda la fila de arriba).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Malla {
    pub columnas: usize,
    pub filas: usize,
    pub desplazamientos: Vec<Punto>,
}

/// Interpolación cúbica de Hermite **monótona** entre `p1` y `p2` (con `p0` y `p3` de
/// vecinos), con `t` de 0 a 1. Vale `p1` en `t = 0` y `p2` en `t = 1`, es suave (la
/// pendiente no salta al pasar de un tramo al siguiente) y **nunca se pasa**: entre `p1` y
/// `p2` el valor queda siempre entre los dos.
///
/// Se usa en lugar de Catmull-Rom porque esa curva sí se pasa: junto a un punto muy
/// desplazado "rebota" al otro lado, y cerca de las esquinas (que no se mueven) la
/// superficie se metía hacia adentro en vez de seguir los puntos. Acá la pendiente en
/// cada punto es el promedio armónico de las pendientes de los dos tramos que lo tocan, y
/// vale 0 si el punto es un máximo o un mínimo local (así no hay rebote).
fn hermite_monotona(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    // Pendiente de cada tramo (los puntos están a distancia 1 entre sí).
    let (d0, d1, d2) = (p1 - p0, p2 - p1, p3 - p2);
    let pendiente = |antes: f32, despues: f32| {
        if antes * despues <= 0.0 {
            0.0 // máximo, mínimo o tramo plano: la curva se aplana ahí
        } else {
            2.0 * antes * despues / (antes + despues)
        }
    };
    let (m1, m2) = (pendiente(d0, d1), pendiente(d1, d2));
    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * p1
        + (t3 - 2.0 * t2 + t) * m1
        + (-2.0 * t3 + 3.0 * t2) * p2
        + (t3 - t2) * m2
}

impl Malla {
    /// Una malla sin deformar, con esa cantidad de puntos por lado (limitada a lo permitido).
    pub fn nueva(columnas: usize, filas: usize) -> Self {
        let columnas = columnas.clamp(PUNTOS_MINIMOS, PUNTOS_MAXIMOS);
        let filas = filas.clamp(PUNTOS_MINIMOS, PUNTOS_MAXIMOS);
        Malla {
            columnas,
            filas,
            desplazamientos: vec![[0.0, 0.0]; columnas * filas],
        }
    }

    fn indice(&self, columna: usize, fila: usize) -> usize {
        fila * self.columnas + columna
    }

    /// El desplazamiento del punto de control `(columna, fila)`.
    pub fn desplazamiento(&self, columna: usize, fila: usize) -> Punto {
        self.desplazamientos[self.indice(columna, fila)]
    }

    /// ¿Ese punto es una de las 4 esquinas de la malla?
    pub fn es_esquina(&self, columna: usize, fila: usize) -> bool {
        (columna == 0 || columna == self.columnas - 1) && (fila == 0 || fila == self.filas - 1)
    }

    /// Cambia el desplazamiento de un punto. Las esquinas no se pueden mover: devuelve
    /// `false` (y no hace nada) para ellas o para un punto que no existe.
    pub fn poner(&mut self, columna: usize, fila: usize, desplazamiento: Punto) -> bool {
        if columna >= self.columnas || fila >= self.filas || self.es_esquina(columna, fila) {
            return false;
        }
        let i = self.indice(columna, fila);
        self.desplazamientos[i] = desplazamiento;
        true
    }

    /// Las coordenadas `(u, v)` de la superficie donde está el punto de control (de 0 a 1).
    pub fn uv_de_nodo(&self, columna: usize, fila: usize) -> Punto {
        [
            columna as f32 / (self.columnas - 1) as f32,
            fila as f32 / (self.filas - 1) as f32,
        ]
    }

    /// El desplazamiento interpolado en cualquier punto `(u, v)` de la superficie.
    pub fn desplazamiento_en(&self, u: f32, v: f32) -> Punto {
        let x = u.clamp(0.0, 1.0) * (self.columnas - 1) as f32;
        let y = v.clamp(0.0, 1.0) * (self.filas - 1) as f32;
        // La celda donde cae el punto, y la posición (0 a 1) dentro de ella.
        let i = (x.floor() as usize).min(self.columnas - 2);
        let j = (y.floor() as usize).min(self.filas - 2);
        let (tx, ty) = (x - i as f32, y - j as f32);

        // Los 4 puntos vecinos en cada dirección. Pasado el borde de la grilla no hay punto:
        // se continúa la línea que traían los dos últimos (así la curva sale del borde en
        // la dirección del primer tramo, en vez de aplanarse).
        let mut resultado = [0.0, 0.0];
        for (canal, salida) in resultado.iter_mut().enumerate() {
            // Primero se interpola a lo largo de cada una de las 4 filas vecinas...
            let mut por_fila = [0.0; 4];
            for (n, valor) in por_fila.iter_mut().enumerate() {
                let f = j as isize + n as isize - 1;
                let p = |dc: isize| self.valor_extendido(i as isize + dc, f, canal);
                *valor = hermite_monotona(p(-1), p(0), p(1), p(2), tx);
            }
            // ...y después entre esos 4 resultados, hacia abajo.
            *salida = hermite_monotona(por_fila[0], por_fila[1], por_fila[2], por_fila[3], ty);
        }
        resultado
    }

    /// El desplazamiento (un canal: 0 = x, 1 = y) del punto `(columna, fila)`, aun si cae
    /// fuera de la grilla: ahí se extiende linealmente a partir de los dos puntos más cercanos.
    fn valor_extendido(&self, columna: isize, fila: isize, canal: usize) -> f32 {
        let (columnas, filas) = (self.columnas as isize, self.filas as isize);
        if columna < 0 {
            return 2.0 * self.valor_extendido(0, fila, canal)
                - self.valor_extendido(1, fila, canal);
        }
        if columna >= columnas {
            return 2.0 * self.valor_extendido(columnas - 1, fila, canal)
                - self.valor_extendido(columnas - 2, fila, canal);
        }
        if fila < 0 {
            return 2.0 * self.valor_extendido(columna, 0, canal)
                - self.valor_extendido(columna, 1, canal);
        }
        if fila >= filas {
            return 2.0 * self.valor_extendido(columna, filas - 1, canal)
                - self.valor_extendido(columna, filas - 2, canal);
        }
        self.desplazamiento(columna as usize, fila as usize)[canal]
    }

    /// ¿Ningún punto está desplazado? (La malla no cambia nada.)
    pub fn es_neutra(&self) -> bool {
        self.desplazamientos.iter().all(|d| *d == [0.0, 0.0])
    }

    /// Deja una malla leída de un archivo en un estado válido: los desplazamientos que no
    /// sean números se vuelven 0 y las esquinas se ponen en 0. Devuelve `false` si no tiene
    /// arreglo (cantidad de puntos fuera de lo permitido o que no coincide con la grilla).
    pub fn normalizar(&mut self) -> bool {
        let en_rango = |n: usize| (PUNTOS_MINIMOS..=PUNTOS_MAXIMOS).contains(&n);
        if !en_rango(self.columnas)
            || !en_rango(self.filas)
            || self.desplazamientos.len() != self.columnas * self.filas
        {
            return false;
        }
        for d in &mut self.desplazamientos {
            for valor in d.iter_mut() {
                if !valor.is_finite() {
                    *valor = 0.0;
                }
            }
        }
        for (columna, fila) in [
            (0, 0),
            (self.columnas - 1, 0),
            (0, self.filas - 1),
            (self.columnas - 1, self.filas - 1),
        ] {
            let i = self.indice(columna, fila);
            self.desplazamientos[i] = [0.0, 0.0];
        }
        true
    }

    /// La misma deformación con otra cantidad de puntos: cada punto nuevo toma el
    /// desplazamiento interpolado de la malla actual en su posición.
    pub fn con_dimensiones(&self, columnas: usize, filas: usize) -> Malla {
        let mut nueva = Malla::nueva(columnas, filas);
        for fila in 0..nueva.filas {
            for columna in 0..nueva.columnas {
                let [u, v] = nueva.uv_de_nodo(columna, fila);
                let d = self.desplazamiento_en(u, v);
                nueva.poner(columna, fila, d);
            }
        }
        nueva
    }

    /// Todos los desplazamientos multiplicados por `factor`. Se usa al cambiar el tamaño de
    /// la superficie: la deformación crece o se achica con ella.
    pub fn escalada(&self, factor: f32) -> Malla {
        Malla {
            columnas: self.columnas,
            filas: self.filas,
            desplazamientos: self
                .desplazamientos
                .iter()
                .map(|d| [d[0] * factor, d[1] * factor])
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn casi(a: Punto, b: Punto, tolerancia: f32) -> bool {
        (a[0] - b[0]).abs() < tolerancia && (a[1] - b[1]).abs() < tolerancia
    }

    #[test]
    fn una_malla_nueva_esta_sin_deformar_y_respeta_los_limites() {
        let m = Malla::nueva(4, 3);
        assert_eq!((m.columnas, m.filas), (4, 3));
        assert_eq!(m.desplazamientos.len(), 12);
        assert!(m.es_neutra());
        assert_eq!(Malla::nueva(1, 99).columnas, PUNTOS_MINIMOS);
        assert_eq!(Malla::nueva(1, 99).filas, PUNTOS_MAXIMOS);
    }

    #[test]
    fn sin_deformar_el_desplazamiento_es_cero_en_todas_partes() {
        let m = Malla::nueva(5, 5);
        for (u, v) in [(0.0, 0.0), (0.3, 0.7), (0.5, 0.5), (1.0, 1.0), (0.99, 0.01)] {
            assert_eq!(m.desplazamiento_en(u, v), [0.0, 0.0]);
        }
    }

    #[test]
    fn la_curva_pasa_exactamente_por_cada_punto_de_control() {
        let mut m = Malla::nueva(5, 4);
        assert!(m.poner(2, 1, [30.0, -12.0]));
        assert!(m.poner(1, 2, [-7.5, 20.0]));
        for (c, f) in [(2, 1), (1, 2), (3, 2), (0, 1)] {
            let [u, v] = m.uv_de_nodo(c, f);
            let esperado = m.desplazamiento(c, f);
            assert!(
                casi(m.desplazamiento_en(u, v), esperado, 1e-4),
                "nodo ({c},{f}): {:?} != {esperado:?}",
                m.desplazamiento_en(u, v)
            );
        }
    }

    #[test]
    fn un_campo_constante_se_reproduce_en_toda_la_superficie() {
        // Construida a mano (sin pasar por `poner`, que protege las esquinas).
        let m = Malla {
            columnas: 4,
            filas: 4,
            desplazamientos: vec![[5.0, -3.0]; 16],
        };
        for (u, v) in [
            (0.0, 0.0),
            (0.21, 0.64),
            (0.5, 0.5),
            (1.0, 0.3),
            (0.77, 1.0),
        ] {
            assert!(
                casi(m.desplazamiento_en(u, v), [5.0, -3.0], 1e-4),
                "({u},{v})"
            );
        }
    }

    #[test]
    fn un_campo_lineal_se_reproduce_en_el_interior() {
        // El desplazamiento en x crece linealmente con la columna: Catmull-Rom reproduce
        // exactamente las funciones lineales entre puntos que tienen vecinos a ambos lados.
        let (columnas, filas) = (7, 4);
        let desplazamientos = (0..filas)
            .flat_map(|_| (0..columnas).map(|c| [10.0 * c as f32, 0.0]))
            .collect();
        let m = Malla {
            columnas,
            filas,
            desplazamientos,
        };
        for u in [0.2, 0.4, 0.5, 0.6, 0.8] {
            let esperado = 10.0 * u * (columnas - 1) as f32;
            assert!(
                (m.desplazamiento_en(u, 0.5)[0] - esperado).abs() < 1e-3,
                "u={u}"
            );
        }
    }

    #[test]
    fn no_hay_saltos_al_cruzar_el_limite_entre_dos_celdas() {
        let mut m = Malla::nueva(5, 5);
        m.poner(2, 2, [40.0, 25.0]);
        m.poner(1, 2, [-15.0, 5.0]);
        // Alrededor del nodo (u = 0,5), que es un limite de celda.
        let antes = m.desplazamiento_en(0.5 - 1e-4, 0.5);
        let despues = m.desplazamiento_en(0.5 + 1e-4, 0.5);
        assert!(casi(antes, despues, 0.05), "{antes:?} vs {despues:?}");
    }

    #[test]
    fn las_esquinas_no_se_pueden_mover() {
        let mut m = Malla::nueva(4, 4);
        for (c, f) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
            assert!(m.es_esquina(c, f));
            assert!(!m.poner(c, f, [9.0, 9.0]));
        }
        assert!(m.es_neutra());
        assert!(!m.es_esquina(1, 0)); // un punto del borde que no es esquina
        assert!(m.poner(1, 0, [1.0, 2.0]));
        // Un punto que no existe tampoco.
        assert!(!m.poner(9, 9, [1.0, 1.0]));
    }

    #[test]
    fn normalizar_arregla_o_rechaza_una_malla_leida() {
        let mut m = Malla::nueva(3, 3);
        m.desplazamientos[0] = [5.0, 5.0]; // una esquina con valor
        m.desplazamientos[4] = [f32::NAN, 2.0]; // el centro, con un NaN
        assert!(m.normalizar());
        assert_eq!(m.desplazamientos[0], [0.0, 0.0]);
        assert_eq!(m.desplazamientos[4], [0.0, 2.0]);

        // Cantidad de puntos que no coincide con la grilla.
        let mut mala = Malla {
            columnas: 3,
            filas: 3,
            desplazamientos: vec![[0.0, 0.0]; 5],
        };
        assert!(!mala.normalizar());
        // Dimensiones fuera de rango.
        let mut enorme = Malla {
            columnas: 40,
            filas: 40,
            desplazamientos: vec![[0.0, 0.0]; 1600],
        };
        assert!(!enorme.normalizar());
        let mut chica = Malla {
            columnas: 1,
            filas: 1,
            desplazamientos: vec![[0.0, 0.0]],
        };
        assert!(!chica.normalizar());
    }

    #[test]
    fn cambiar_la_cantidad_de_puntos_conserva_la_deformacion() {
        let mut m = Malla::nueva(3, 3);
        m.poner(1, 1, [20.0, -10.0]);
        // De 3 a 5 puntos por lado: el nodo central sigue en el mismo lugar (u = 0,5).
        let mas = m.con_dimensiones(5, 5);
        assert_eq!((mas.columnas, mas.filas), (5, 5));
        assert!(casi(mas.desplazamiento(2, 2), [20.0, -10.0], 1e-3));
        // Y la deformación en cualquier punto es casi la misma.
        for (u, v) in [(0.3, 0.4), (0.5, 0.5), (0.8, 0.2)] {
            assert!(
                casi(m.desplazamiento_en(u, v), mas.desplazamiento_en(u, v), 2.0),
                "({u},{v})"
            );
        }
        // Cambiar a las mismas dimensiones no altera nada.
        assert_eq!(m.con_dimensiones(3, 3), m);
    }

    #[test]
    fn escalar_multiplica_todos_los_desplazamientos() {
        let mut m = Malla::nueva(3, 3);
        m.poner(1, 1, [10.0, -4.0]);
        let doble = m.escalada(2.0);
        assert_eq!(doble.desplazamiento(1, 1), [20.0, -8.0]);
        assert_eq!(m.escalada(1.0), m);
        assert!(m.escalada(0.0).es_neutra());
    }

    #[test]
    fn un_punto_desplazado_no_hace_rebotar_la_curva_hacia_el_otro_lado() {
        // Un punto del borde de arriba se sube 100; los demás quedan en 0. Con Catmull-Rom
        // la curva se metía por debajo de 0 junto a ese punto; ahora tiene que quedar siempre
        // entre 0 y 100 (en `y`, el desplazamiento es negativo hacia arriba: se mira el valor).
        let mut m = Malla::nueva(5, 5);
        m.poner(2, 0, [0.0, 100.0]);
        for k in 0..=200 {
            let u = k as f32 / 200.0;
            let d = m.desplazamiento_en(u, 0.0)[1];
            assert!((-1e-3..=100.0 + 1e-3).contains(&d), "u = {u}: {d}");
        }
        // Y en el interior tampoco se pasa de lo que tienen los puntos.
        for k in 0..=50 {
            for l in 0..=50 {
                let d = m.desplazamiento_en(k as f32 / 50.0, l as f32 / 50.0)[1];
                assert!((-1e-3..=100.0 + 1e-3).contains(&d), "({k}, {l}): {d}");
            }
        }
    }

    #[test]
    fn junto_a_una_esquina_la_curva_sigue_la_direccion_de_los_puntos() {
        // Los puntos del borde de arriba: esquina (0), 40, 80, 120, esquina (0 fija). Sale
        // de la esquina creciendo, sin hundirse antes.
        let mut m = Malla::nueva(5, 2);
        m.poner(1, 0, [0.0, 40.0]);
        m.poner(2, 0, [0.0, 80.0]);
        m.poner(3, 0, [0.0, 120.0]);
        let mut anterior = m.desplazamiento_en(0.0, 0.0)[1];
        for k in 1..=100 {
            let d = m.desplazamiento_en(k as f32 / 400.0, 0.0)[1]; // el primer tramo
            assert!(d >= anterior - 1e-3, "baja en k = {k}: {anterior} -> {d}");
            anterior = d;
        }
        assert!((anterior - 40.0).abs() < 1e-2);
    }
}
