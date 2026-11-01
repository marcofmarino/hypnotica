//! Selección múltiple de puntos con un marco (arrastrar sobre espacio vacío) y movimiento
//! de grupos. Lógica pura: se prueba con tests, sin egui ni Bevy.
//!
//! "Punto" quiere decir, según el modo del editor: una esquina de la superficie, un punto
//! de su forma o un punto de control de su malla. Cada uno se identifica con un `Asa`.

use vm_core::{Malla, Punto};
use vm_warp::deformacion;
use vm_warp::homografia::Homografia;

/// Algo que se puede agarrar y mover en el editor de la salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asa {
    /// Una esquina de la superficie (0 a 3).
    Esquina(usize),
    /// Un punto de la forma.
    Forma(usize),
    /// Un punto de control de la malla `(columna, fila)`.
    Nodo(usize, usize),
}

/// El rectángulo con esquinas opuestas `a` y `b`, en cualquier orden: `(mínimo, máximo)`.
pub fn rectangulo(a: [f32; 2], b: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    (
        [a[0].min(b[0]), a[1].min(b[1])],
        [a[0].max(b[0]), a[1].max(b[1])],
    )
}

/// ¿El punto está dentro del rectángulo `(mínimo, máximo)` (bordes incluidos)?
pub fn dentro(marco: ([f32; 2], [f32; 2]), p: [f32; 2]) -> bool {
    p[0] >= marco.0[0] && p[0] <= marco.1[0] && p[1] >= marco.0[1] && p[1] <= marco.1[1]
}

/// La selección al **empezar a arrastrar** sobre un punto. Si ya estaba seleccionado, se
/// arrastra todo el grupo; si no, pasa a ser el único elegido (o se suma, con Shift).
pub fn al_agarrar(actual: &[Asa], agarrada: Asa, sumar: bool) -> Vec<Asa> {
    if actual.contains(&agarrada) {
        actual.to_vec()
    } else if sumar {
        let mut nueva = actual.to_vec();
        nueva.push(agarrada);
        nueva
    } else {
        vec![agarrada]
    }
}

/// La selección tras un **clic** (sin arrastrar). Sobre un punto: pasa a ser el único
/// elegido, o con Shift se agrega o se quita. Sobre espacio vacío: se limpia, salvo con Shift.
pub fn al_hacer_clic(actual: &[Asa], sobre: Option<Asa>, sumar: bool) -> Vec<Asa> {
    match sobre {
        Some(asa) if sumar => {
            if actual.contains(&asa) {
                actual.iter().copied().filter(|a| *a != asa).collect()
            } else {
                let mut nueva = actual.to_vec();
                nueva.push(asa);
                nueva
            }
        }
        Some(asa) => vec![asa],
        None if sumar => actual.to_vec(),
        None => Vec::new(),
    }
}

/// La selección al **soltar el marco**: los puntos de dentro reemplazan a los elegidos, o
/// se suman a ellos con Shift. Sin repetidos.
pub fn al_soltar_marco(actual: &[Asa], del_marco: &[Asa], sumar: bool) -> Vec<Asa> {
    let mut nueva: Vec<Asa> = if sumar { actual.to_vec() } else { Vec::new() };
    for asa in del_marco {
        if !nueva.contains(asa) {
            nueva.push(*asa);
        }
    }
    nueva
}

/// Las esquinas con las elegidas desplazadas `delta`.
pub fn mover_esquinas(esquinas: [[f32; 2]; 4], elegidos: &[Asa], delta: [f32; 2]) -> [[f32; 2]; 4] {
    let mut nuevas = esquinas;
    for asa in elegidos {
        if let Asa::Esquina(i) = asa
            && *i < 4
        {
            nuevas[*i][0] += delta[0];
            nuevas[*i][1] += delta[1];
        }
    }
    nuevas
}

/// La malla con los puntos de control elegidos desplazados `delta` (en unidades de la
/// salida). Las esquinas de la malla no se mueven (`Malla::poner` lo impide).
pub fn mover_nodos(malla: &Malla, elegidos: &[Asa], delta: [f32; 2]) -> Malla {
    let mut nueva = malla.clone();
    for asa in elegidos {
        if let Asa::Nodo(columna, fila) = asa
            && *columna < malla.columnas
            && *fila < malla.filas
        {
            let d = malla.desplazamiento(*columna, *fila);
            nueva.poner(*columna, *fila, [d[0] + delta[0], d[1] + delta[1]]);
        }
    }
    nueva
}

/// La forma con los puntos elegidos movidos `delta` **en pantalla**. Cada punto de la forma
/// vive en coordenadas de la superficie, así que se lleva a la salida, se desplaza y se
/// vuelve a llevar a la superficie (limitado a 0..1).
pub fn mover_forma(
    forma: &[Punto],
    elegidos: &[Asa],
    delta: [f32; 2],
    h: &Homografia,
    malla: Option<&Malla>,
) -> Vec<Punto> {
    let mut nueva = forma.to_vec();
    for asa in elegidos {
        if let Asa::Forma(i) = asa
            && *i < forma.len()
        {
            let [x, y] = deformacion::proyectar(h, malla, forma[*i][0], forma[*i][1]);
            let [u, v] = deformacion::desproyectar(h, malla, x + delta[0], y + delta[1]);
            nueva[*i] = [u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)];
        }
    }
    nueva
}

#[cfg(test)]
mod tests {
    use super::*;
    use Asa::*;

    #[test]
    fn el_rectangulo_se_ordena_sin_importar_desde_que_esquina_se_arrastre() {
        assert_eq!(rectangulo([5.0, 9.0], [1.0, 2.0]), ([1.0, 2.0], [5.0, 9.0]));
        assert_eq!(rectangulo([1.0, 9.0], [5.0, 2.0]), ([1.0, 2.0], [5.0, 9.0]));
    }

    #[test]
    fn dentro_incluye_los_bordes() {
        let marco = ([0.0, 0.0], [10.0, 10.0]);
        assert!(dentro(marco, [5.0, 5.0]));
        assert!(dentro(marco, [0.0, 10.0]));
        assert!(!dentro(marco, [10.1, 5.0]));
        assert!(!dentro(marco, [5.0, -0.1]));
    }

    #[test]
    fn agarrar_un_punto_elegido_arrastra_todo_el_grupo() {
        let grupo = [Esquina(0), Esquina(2)];
        assert_eq!(al_agarrar(&grupo, Esquina(2), false), grupo.to_vec());
    }

    #[test]
    fn agarrar_un_punto_no_elegido_lo_deja_solo_o_lo_suma_con_shift() {
        let grupo = [Esquina(0), Esquina(2)];
        assert_eq!(al_agarrar(&grupo, Esquina(1), false), vec![Esquina(1)]);
        assert_eq!(
            al_agarrar(&grupo, Esquina(1), true),
            vec![Esquina(0), Esquina(2), Esquina(1)]
        );
    }

    #[test]
    fn un_clic_sobre_un_punto_lo_elige_solo_y_con_shift_lo_alterna() {
        let grupo = [Forma(0), Forma(1)];
        assert_eq!(al_hacer_clic(&grupo, Some(Forma(3)), false), vec![Forma(3)]);
        assert_eq!(
            al_hacer_clic(&grupo, Some(Forma(3)), true),
            vec![Forma(0), Forma(1), Forma(3)]
        );
        // Con Shift sobre uno ya elegido, se quita.
        assert_eq!(al_hacer_clic(&grupo, Some(Forma(0)), true), vec![Forma(1)]);
    }

    #[test]
    fn un_clic_en_espacio_vacio_limpia_la_seleccion_salvo_con_shift() {
        let grupo = [Nodo(1, 1), Nodo(2, 1)];
        assert!(al_hacer_clic(&grupo, None, false).is_empty());
        assert_eq!(al_hacer_clic(&grupo, None, true), grupo.to_vec());
    }

    #[test]
    fn soltar_el_marco_reemplaza_o_suma_sin_repetir() {
        let actual = [Esquina(0)];
        let del_marco = [Esquina(1), Esquina(0)];
        assert_eq!(
            al_soltar_marco(&actual, &del_marco, false),
            vec![Esquina(1), Esquina(0)]
        );
        assert_eq!(
            al_soltar_marco(&actual, &del_marco, true),
            vec![Esquina(0), Esquina(1)]
        );
        // Un marco vacío sin Shift limpia la selección.
        assert!(al_soltar_marco(&actual, &[], false).is_empty());
        assert_eq!(al_soltar_marco(&actual, &[], true), vec![Esquina(0)]);
    }

    #[test]
    fn mover_esquinas_desplaza_solo_las_elegidas() {
        let base = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let r = mover_esquinas(base, &[Esquina(1), Esquina(2)], [3.0, -1.0]);
        assert_eq!(r, [[0.0, 0.0], [13.0, -1.0], [13.0, 9.0], [0.0, 10.0]]);
        // Las asas de otro tipo se ignoran.
        assert_eq!(mover_esquinas(base, &[Forma(0)], [5.0, 5.0]), base);
    }

    #[test]
    fn mover_nodos_desplaza_los_elegidos_y_respeta_las_esquinas() {
        let mut malla = Malla::nueva(4, 4);
        malla.poner(1, 1, [10.0, 0.0]);
        let r = mover_nodos(&malla, &[Nodo(1, 1), Nodo(2, 1), Nodo(0, 0)], [5.0, -2.0]);
        assert_eq!(r.desplazamiento(1, 1), [15.0, -2.0]); // sumó al que ya tenía
        assert_eq!(r.desplazamiento(2, 1), [5.0, -2.0]);
        assert_eq!(r.desplazamiento(0, 0), [0.0, 0.0]); // una esquina: no se mueve
        assert_eq!(malla.desplazamiento(1, 1), [10.0, 0.0]); // la original no cambió
    }

    #[test]
    fn mover_la_forma_desplaza_en_pantalla_los_puntos_elegidos() {
        // Un cuadrado sin perspectiva: mover 100 px en pantalla = 0,25 en la superficie.
        let h = Homografia::desde_esquinas([
            [-200.0, 200.0],
            [200.0, 200.0],
            [200.0, -200.0],
            [-200.0, -200.0],
        ]);
        let forma = vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]];
        let r = mover_forma(&forma, &[Forma(0), Forma(3)], [100.0, 0.0], &h, None);
        assert!((r[0][0] - 0.5).abs() < 1e-3 && (r[0][1] - 0.25).abs() < 1e-3);
        assert!((r[3][0] - 0.5).abs() < 1e-3 && (r[3][1] - 0.75).abs() < 1e-3);
        // Los no elegidos no cambian.
        assert_eq!(r[1], forma[1]);
        assert_eq!(r[2], forma[2]);
    }

    #[test]
    fn mover_la_forma_no_la_saca_de_la_superficie() {
        let h = Homografia::desde_esquinas([
            [-200.0, 200.0],
            [200.0, 200.0],
            [200.0, -200.0],
            [-200.0, -200.0],
        ]);
        let forma = vec![[0.9, 0.5], [0.5, 0.9], [0.5, 0.1]];
        let r = mover_forma(&forma, &[Forma(0)], [5000.0, 0.0], &h, None);
        assert_eq!(r[0][0], 1.0);
    }
}
