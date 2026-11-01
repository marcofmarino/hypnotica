//! El reparto de la zona de abajo de Composición (columnas, alto y previsualizaciones).
//!
//! Matemática pura, sin egui ni Bevy. Todo es una FUNCIÓN del espacio disponible y de lo que el
//! usuario eligió arrastrando los divisores (guardado como fracciones): así, al redimensionar la
//! ventana el resultado de ese mismo cuadro ya es el correcto, sin depender de lo que se dibujó
//! antes (los paneles redimensionables de egui guardan su tamaño en memoria y lo reconcilian
//! un cuadro tarde, lo que al arrastrar el borde de la ventana se sentía como lag).

/// Cantidad de columnas de la zona de abajo: previsualizaciones, propiedades, clip y archivos.
pub(crate) const COLUMNAS: usize = 4;

/// Reparte `ancho` entre las columnas según sus `fracciones`, sin que ninguna baje de su mínimo.
/// Siempre suma `ancho` (si los mínimos no caben, se achican todos en la misma proporción).
pub(crate) fn repartir(
    ancho: f32,
    fracciones: [f32; COLUMNAS],
    minimos: [f32; COLUMNAS],
) -> [f32; COLUMNAS] {
    let ancho = ancho.max(0.0);
    let suma: f32 = fracciones.iter().map(|f| f.max(0.0)).sum();
    let fr: [f32; COLUMNAS] = if suma > 0.0 {
        fracciones.map(|f| f.max(0.0) / suma)
    } else {
        [1.0 / COLUMNAS as f32; COLUMNAS]
    };
    let suma_minimos: f32 = minimos.iter().sum();
    let minimos = if suma_minimos > ancho && suma_minimos > 0.0 {
        minimos.map(|m| m * ancho / suma_minimos)
    } else {
        minimos
    };

    let mut ancho_de = fr.map(|f| f * ancho);
    let mut fijas = [false; COLUMNAS];
    // Las que quedan por debajo de su mínimo se clavan en él y el resto se reparte lo que sobra
    // (como mucho hacen falta tantas vueltas como columnas).
    for _ in 0..COLUMNAS {
        let mut cambio = false;
        for i in 0..COLUMNAS {
            if !fijas[i] && ancho_de[i] < minimos[i] {
                ancho_de[i] = minimos[i];
                fijas[i] = true;
                cambio = true;
            }
        }
        let ocupado: f32 = (0..COLUMNAS)
            .filter(|&i| fijas[i])
            .map(|i| ancho_de[i])
            .sum();
        let peso_libre: f32 = (0..COLUMNAS).filter(|&i| !fijas[i]).map(|i| fr[i]).sum();
        if peso_libre > 0.0 {
            for i in 0..COLUMNAS {
                if !fijas[i] {
                    ancho_de[i] = (ancho - ocupado) * fr[i] / peso_libre;
                }
            }
        }
        if !cambio {
            break;
        }
    }
    ancho_de
}

/// Mueve el divisor `i` (entre la columna `i` y la `i + 1`) `delta` píxeles: la de la izquierda
/// crece lo que la de la derecha se achica, sin pasar de los mínimos. Devuelve las fracciones
/// nuevas.
pub(crate) fn mover_divisor(
    fracciones: [f32; COLUMNAS],
    i: usize,
    delta: f32,
    ancho: f32,
    minimos: [f32; COLUMNAS],
) -> [f32; COLUMNAS] {
    if i + 1 >= COLUMNAS || ancho <= 0.0 {
        return fracciones;
    }
    let mut a = repartir(ancho, fracciones, minimos);
    let juntas = a[i] + a[i + 1];
    let izquierda = (a[i] + delta).clamp(
        minimos[i].min(juntas),
        (juntas - minimos[i + 1]).max(minimos[i].min(juntas)),
    );
    a[i] = izquierda;
    a[i + 1] = juntas - izquierda;
    a.map(|x| x / ancho)
}

/// El alto de la zona de abajo: la `fraccion` del alto total, entre su mínimo y lo que deja libre
/// para la grilla de clips.
pub(crate) fn alto_de_la_zona(
    total: f32,
    fraccion: f32,
    minimo: f32,
    minimo_de_la_grilla: f32,
) -> f32 {
    let maximo = (total - minimo_de_la_grilla).max(minimo);
    (total * fraccion).clamp(minimo, maximo)
}

/// El tamaño que ocupa una imagen de `ancho` × `alto` dentro de una caja, sin deformarse y lo más
/// grande que entre.
pub(crate) fn encajar(caja: (f32, f32), ancho: f32, alto: f32) -> (f32, f32) {
    if ancho <= 0.0 || alto <= 0.0 || caja.0 <= 0.0 || caja.1 <= 0.0 {
        return (0.0, 0.0);
    }
    let escala = (caja.0 / ancho).min(caja.1 / alto);
    (ancho * escala, alto * escala)
}

/// Los tamaños de las dos previsualizaciones, siempre una arriba de la otra: la de la
/// COMPOSICIÓN ocupa todo el ancho (con la altura que le toque por su proporción) y la del CLIP
/// se achica para darle el lugar. Solo si la composición no entra a lo alto (le tiene que
/// quedar al clip al menos `alto_minimo_del_clip`) se la limita por la altura.
///
/// `disponible` es (ancho, alto) de la columna; cada título ocupa `alto_de_titulo`; entre los
/// bloques hay `separacion`. Devuelve `(caja de la composición, caja del clip)`.
pub(crate) fn medidas_de_previas(
    disponible: (f32, f32),
    aspecto_de_la_composicion: f32,
    aspecto_del_clip: f32,
    alto_de_titulo: f32,
    separacion: f32,
    alto_minimo_del_clip: f32,
) -> ((f32, f32), (f32, f32)) {
    let (ancho, alto) = disponible;
    // Lo que no es imagen: dos títulos y la separación.
    let fijo = 2.0 * alto_de_titulo + 2.0 * separacion;
    let alto_maximo_de_la_composicion = (alto - fijo - alto_minimo_del_clip).max(1.0);
    let composicion = encajar(
        (ancho, alto_maximo_de_la_composicion),
        aspecto_de_la_composicion,
        1.0,
    );
    let resto = (alto - fijo - composicion.1).max(0.0);
    let clip = encajar((ancho, resto), aspecto_del_clip, 1.0);
    (composicion, clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: [f32; COLUMNAS] = [200.0, 240.0, 250.0, 190.0];
    const FR: [f32; COLUMNAS] = [0.40, 0.25, 0.21, 0.14];

    fn suma(a: [f32; COLUMNAS]) -> f32 {
        a.iter().sum()
    }

    #[test]
    fn con_lugar_de_sobra_se_respetan_las_fracciones() {
        let a = repartir(2000.0, FR, MIN);
        assert!((suma(a) - 2000.0).abs() < 0.01);
        assert!((a[0] - 800.0).abs() < 0.01);
        assert!((a[3] - 280.0).abs() < 0.01);
    }

    #[test]
    fn al_achicar_la_ventana_ninguna_columna_baja_de_su_minimo_y_todo_sigue_sumando() {
        for ancho in [1600.0, 1200.0, 1000.0, 900.0] {
            let a = repartir(ancho, FR, MIN);
            assert!((suma(a) - ancho).abs() < 0.01, "{ancho}");
            for i in 0..COLUMNAS {
                assert!(
                    a[i] >= MIN[i] - 0.01,
                    "{ancho}: la columna {i} mide {}",
                    a[i]
                );
            }
        }
    }

    #[test]
    fn si_los_minimos_no_caben_se_achican_todos_parejo_y_sigue_sumando() {
        let a = repartir(600.0, FR, MIN);
        assert!((suma(a) - 600.0).abs() < 0.01);
        assert!(a.iter().all(|x| *x > 0.0));
    }

    #[test]
    fn el_reparto_es_una_funcion_del_ancho_no_de_lo_anterior() {
        // El mismo ancho da siempre lo mismo, vengas de donde vengas.
        let directo = repartir(1100.0, FR, MIN);
        let _ = repartir(1900.0, FR, MIN);
        let _ = repartir(800.0, FR, MIN);
        assert_eq!(repartir(1100.0, FR, MIN), directo);
    }

    #[test]
    fn mover_un_divisor_pasa_ancho_de_una_columna_a_la_vecina_y_no_toca_las_demas() {
        let antes = repartir(1600.0, FR, MIN);
        let fr = mover_divisor(FR, 0, 50.0, 1600.0, MIN);
        let despues = repartir(1600.0, fr, MIN);
        assert!((despues[0] - (antes[0] + 50.0)).abs() < 0.01);
        assert!((despues[1] - (antes[1] - 50.0)).abs() < 0.01);
        assert!((despues[2] - antes[2]).abs() < 0.01);
        assert!((despues[3] - antes[3]).abs() < 0.01);
    }

    #[test]
    fn un_divisor_no_puede_dejar_a_la_vecina_por_debajo_de_su_minimo() {
        let fr = mover_divisor(FR, 0, 5000.0, 1600.0, MIN);
        let a = repartir(1600.0, fr, MIN);
        assert!(a[1] >= MIN[1] - 0.01);
        assert!((suma(a) - 1600.0).abs() < 0.01);
        let fr = mover_divisor(FR, 2, -5000.0, 1600.0, MIN);
        let a = repartir(1600.0, fr, MIN);
        assert!(a[2] >= MIN[2] - 0.01);
    }

    #[test]
    fn el_alto_de_la_zona_deja_lugar_a_la_grilla() {
        assert_eq!(alto_de_la_zona(800.0, 0.55, 240.0, 190.0), 440.0);
        // Con poco alto: no pasa de lo que deja libre para la grilla.
        assert_eq!(alto_de_la_zona(400.0, 0.9, 240.0, 190.0), 240.0);
        assert_eq!(alto_de_la_zona(1000.0, 0.9, 240.0, 190.0), 810.0);
    }

    #[test]
    fn la_composicion_ocupa_todo_el_ancho_cuando_entra() {
        // Columna de 500 × 460: la composición 16:9 mide 500 × 281 y al clip le queda lugar.
        let (comp, clip) =
            medidas_de_previas((500.0, 460.0), 16.0 / 9.0, 16.0 / 9.0, 22.0, 6.0, 60.0);
        assert!((comp.0 - 500.0).abs() < 0.01, "{comp:?}");
        assert!(clip.1 > 0.0);
        // Y todo entra a lo alto.
        assert!(2.0 * 22.0 + 2.0 * 6.0 + comp.1 + clip.1 <= 460.0 + 0.01);
    }

    #[test]
    fn el_clip_se_achica_para_darle_lugar_a_la_composicion() {
        let (comp_a, clip_a) =
            medidas_de_previas((500.0, 600.0), 16.0 / 9.0, 16.0 / 9.0, 22.0, 6.0, 60.0);
        let (comp_b, clip_b) =
            medidas_de_previas((500.0, 440.0), 16.0 / 9.0, 16.0 / 9.0, 22.0, 6.0, 60.0);
        // Con menos alto, la composición sigue a todo el ancho y es el clip el que se achica.
        assert!((comp_a.0 - comp_b.0).abs() < 0.01);
        assert!(clip_b.1 < clip_a.1);
    }

    #[test]
    fn si_la_composicion_no_entra_a_lo_alto_se_limita_dejandole_su_minimo_al_clip() {
        let (comp, clip) =
            medidas_de_previas((800.0, 300.0), 16.0 / 9.0, 16.0 / 9.0, 22.0, 6.0, 60.0);
        assert!(comp.0 < 800.0);
        assert!(clip.1 >= 59.0, "{clip:?}");
        assert!(2.0 * 22.0 + 2.0 * 6.0 + comp.1 + clip.1 <= 300.0 + 0.5);
    }

    #[test]
    fn una_imagen_entra_sin_deformarse_en_la_caja() {
        let t = encajar((400.0, 100.0), 1920.0, 1080.0);
        assert!((t.1 - 100.0).abs() < 0.01);
        assert!((t.0 / t.1 - 1920.0 / 1080.0).abs() < 1e-3);
        assert_eq!(encajar((100.0, 100.0), 0.0, 10.0), (0.0, 0.0));
    }
}
