//! Análisis de audio para que la imagen **reaccione al sonido** (ver `docs/05`). Rust puro:
//! recibe muestras y devuelve cuatro números de 0 a 1. No reproduce nada.
//!
//! El camino es: muestras mono → ventana de Hann de 1024 puntos → FFT → energía de cada banda
//! (graves, medios, agudos) → escala en decibeles → suavizado (sube rápido, baja lento, para
//! que la imagen no tiemble).

use std::f32::consts::PI;

/// Puntos de cada análisis. A 48 kHz son 21 ms: sirve para seguir un golpe de bombo.
pub const TAMANO_DE_VENTANA: usize = 1024;
/// Cada cuántas muestras se hace un análisis nuevo (la mitad de la ventana: se solapan).
const SALTO: usize = TAMANO_DE_VENTANA / 2;

/// Los límites de las bandas, en Hz.
const GRAVES: (f32, f32) = (20.0, 250.0);
const MEDIOS: (f32, f32) = (250.0, 2000.0);
const AGUDOS: (f32, f32) = (2000.0, 16000.0);

/// Un sonido de este nivel (en dB respecto del máximo) o menos cuenta como silencio (0). El
/// máximo (0 dB) es una onda senoidal a fondo de escala (1,0).
const PISO_EN_DB: f32 = -60.0;

/// Cuánto tarda en subir (*ataque*) y en bajar (*caída*) cada señal, en segundos.
const ATAQUE: f32 = 0.03;
const CAIDA: f32 = 0.25;

/// Lo que sale del análisis: cada valor va de 0 (silencio) a 1 (a fondo).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Senales {
    /// El volumen general.
    pub nivel: f32,
    pub graves: f32,
    pub medios: f32,
    pub agudos: f32,
}

impl Senales {
    /// Las cuatro señales como una lista, en el orden `nivel, graves, medios, agudos`.
    pub fn como_lista(&self) -> [f32; 4] {
        [self.nivel, self.graves, self.medios, self.agudos]
    }

    /// Las señales después de `segundos` sin recibir sonido: caen hacia 0 con la caída normal.
    pub fn sin_sonido(&self, segundos: f32) -> Senales {
        let k = (-segundos.max(0.0) / CAIDA).exp();
        Senales {
            nivel: self.nivel * k,
            graves: self.graves * k,
            medios: self.medios * k,
            agudos: self.agudos * k,
        }
    }
}

/// Acumula muestras y entrega las señales cada vez que junta una ventana.
pub struct AnalizadorDeAudio {
    tasa: f32,
    ventana_de_hann: Vec<f32>,
    pendientes: Vec<f32>,
    senales: Senales,
}

impl AnalizadorDeAudio {
    /// `tasa` son las muestras por segundo (por ejemplo, 48000).
    pub fn nuevo(tasa: f32) -> Self {
        let ventana_de_hann = (0..TAMANO_DE_VENTANA)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / TAMANO_DE_VENTANA as f32).cos())
            .collect();
        AnalizadorDeAudio {
            tasa,
            ventana_de_hann,
            pendientes: Vec::new(),
            senales: Senales::default(),
        }
    }

    /// Agrega muestras mono (de -1 a 1) y devuelve las señales actuales. Cada
    /// `TAMANO_DE_VENTANA / 2` muestras nuevas se hace un análisis.
    pub fn entrar(&mut self, muestras: &[f32]) -> Senales {
        self.pendientes.extend_from_slice(muestras);
        while self.pendientes.len() >= TAMANO_DE_VENTANA {
            let objetivo = self.analizar_la_primera_ventana();
            let segundos = SALTO as f32 / self.tasa;
            self.senales = Senales {
                nivel: suavizar(self.senales.nivel, objetivo.nivel, segundos),
                graves: suavizar(self.senales.graves, objetivo.graves, segundos),
                medios: suavizar(self.senales.medios, objetivo.medios, segundos),
                agudos: suavizar(self.senales.agudos, objetivo.agudos, segundos),
            };
            self.pendientes.drain(..SALTO);
        }
        self.senales
    }

    /// Las señales actuales, sin agregar muestras.
    pub fn senales(&self) -> Senales {
        self.senales
    }

    /// Lo que dice la primera ventana de `pendientes` (sin suavizar).
    fn analizar_la_primera_ventana(&self) -> Senales {
        let mut datos: Vec<(f32, f32)> = self.pendientes[..TAMANO_DE_VENTANA]
            .iter()
            .zip(&self.ventana_de_hann)
            .map(|(m, v)| (m * v, 0.0))
            .collect();
        fft(&mut datos);

        // Una onda senoidal a fondo de escala da en su bin un módulo de N/4 con la ventana de
        // Hann (la mitad de N por la ventana, la mitad por repartirse en dos bins espejados).
        // Dividiendo por eso, 1,0 es «a fondo».
        let norma = TAMANO_DE_VENTANA as f32 / 4.0;
        let potencia: Vec<f32> = datos[..TAMANO_DE_VENTANA / 2]
            .iter()
            .map(|(re, im)| (re * re + im * im) / (norma * norma))
            .collect();
        let ancho_de_bin = self.tasa / TAMANO_DE_VENTANA as f32;
        let banda = |(desde, hasta): (f32, f32)| {
            let primero = (desde / ancho_de_bin).ceil() as usize;
            let ultimo = ((hasta / ancho_de_bin) as usize).min(potencia.len() - 1);
            let suma: f32 = potencia
                .get(primero..=ultimo)
                .map_or(0.0, |bins| bins.iter().sum());
            // Una senoidal reparte 1,5 de potencia relativa entre los 3 bins de su lóbulo.
            en_escala_de_0_a_1((suma / 1.5).sqrt())
        };
        // El nivel general sale de TODAS las frecuencias (sin el continuo en 0 Hz).
        let total: f32 = potencia[1..].iter().sum();
        Senales {
            nivel: en_escala_de_0_a_1((total / 1.5).sqrt()),
            graves: banda(GRAVES),
            medios: banda(MEDIOS),
            agudos: banda(AGUDOS),
        }
    }
}

/// Pasa una amplitud (1,0 = a fondo) a una escala de 0 a 1 en decibeles, con el piso en
/// `PISO_EN_DB`.
fn en_escala_de_0_a_1(amplitud: f32) -> f32 {
    if amplitud <= 1e-9 {
        return 0.0;
    }
    let db = 20.0 * amplitud.log10();
    ((db - PISO_EN_DB) / -PISO_EN_DB).clamp(0.0, 1.0)
}

/// Mueve `actual` hacia `objetivo`: rápido si sube, lento si baja. `segundos` es el tiempo
/// que pasó entre un análisis y el siguiente.
fn suavizar(actual: f32, objetivo: f32, segundos: f32) -> f32 {
    let constante = if objetivo > actual { ATAQUE } else { CAIDA };
    actual + (objetivo - actual) * (1.0 - (-segundos / constante).exp())
}

/// FFT de base 2, en el lugar: `datos` son pares (real, imaginario) y su largo tiene que ser
/// una potencia de 2.
pub fn fft(datos: &mut [(f32, f32)]) {
    let n = datos.len();
    assert!(
        n.is_power_of_two(),
        "el largo de la FFT tiene que ser potencia de 2"
    );
    // Se reordenan los elementos por el reverso de sus bits.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            datos.swap(i, j);
        }
    }
    // Se combinan de a pares, de a cuatro, de a ocho...
    let mut largo = 2;
    while largo <= n {
        let angulo = -2.0 * PI / largo as f32;
        for inicio in (0..n).step_by(largo) {
            for k in 0..largo / 2 {
                let (seno, coseno) = (angulo * k as f32).sin_cos();
                let (a_re, a_im) = datos[inicio + k];
                let (b_re, b_im) = datos[inicio + k + largo / 2];
                // b multiplicado por el giro (coseno, seno).
                let (t_re, t_im) = (b_re * coseno - b_im * seno, b_re * seno + b_im * coseno);
                datos[inicio + k] = (a_re + t_re, a_im + t_im);
                datos[inicio + k + largo / 2] = (a_re - t_re, a_im - t_im);
            }
        }
        largo *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASA: f32 = 48_000.0;

    /// Una senoidal de `frecuencia` Hz y `amplitud` (1,0 = a fondo), de `n` muestras.
    fn seno(frecuencia: f32, amplitud: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amplitud * (2.0 * PI * frecuencia * i as f32 / TASA).sin())
            .collect()
    }

    /// Alimenta al analizador con `segundos` de sonido y devuelve el resultado final.
    fn analizar(muestras: &[f32]) -> Senales {
        let mut a = AnalizadorDeAudio::nuevo(TASA);
        a.entrar(muestras)
    }

    // Un bin de la FFT de 1024 puntos a 48 kHz mide 46,875 Hz: estas frecuencias caen justo en uno.
    const EN_GRAVES: f32 = 93.75; // bin 2
    const EN_MEDIOS: f32 = 984.375; // bin 21
    const EN_AGUDOS: f32 = 8015.625; // bin 171

    #[test]
    fn la_fft_de_un_impulso_es_plana() {
        let mut datos = vec![(0.0, 0.0); 8];
        datos[0] = (1.0, 0.0);
        fft(&mut datos);
        for (re, im) in datos {
            assert!((re - 1.0).abs() < 1e-5 && im.abs() < 1e-5);
        }
    }

    #[test]
    fn la_fft_concentra_una_senoidal_en_su_bin() {
        let n = 64;
        let mut datos: Vec<(f32, f32)> = (0..n)
            .map(|i| ((2.0 * PI * 5.0 * i as f32 / n as f32).sin(), 0.0))
            .collect();
        fft(&mut datos);
        let modulo = |i: usize| (datos[i].0.powi(2) + datos[i].1.powi(2)).sqrt();
        // En una senoidal de amplitud 1 el bin vale N/2.
        assert!((modulo(5) - n as f32 / 2.0).abs() < 1e-3);
        assert!(modulo(5) > 100.0 * modulo(9));
    }

    #[test]
    fn el_silencio_da_cero_en_todo() {
        let s = analizar(&vec![0.0; 4096]);
        assert_eq!(s, Senales::default());
    }

    #[test]
    fn cada_tono_enciende_su_banda_y_casi_no_las_otras() {
        // 1 s alcanza para que el suavizado llegue al valor final.
        for (frecuencia, banda) in [(EN_GRAVES, 0), (EN_MEDIOS, 1), (EN_AGUDOS, 2)] {
            let s = analizar(&seno(frecuencia, 1.0, TASA as usize));
            let bandas = [s.graves, s.medios, s.agudos];
            assert!(
                bandas[banda] > 0.9,
                "{frecuencia} Hz: su banda debía estar a fondo y es {}",
                bandas[banda]
            );
            for (i, v) in bandas.iter().enumerate() {
                if i != banda {
                    assert!(*v < 0.5, "{frecuencia} Hz: la banda {i} se encendió: {v}");
                }
            }
            assert!(
                s.nivel > 0.9,
                "el nivel general debía estar a fondo: {}",
                s.nivel
            );
        }
    }

    #[test]
    fn la_escala_es_en_decibeles() {
        // Una senoidal a la mitad de amplitud está 6 dB abajo: 54/60 = 0,9.
        let s = analizar(&seno(EN_MEDIOS, 0.5, TASA as usize));
        assert!((s.medios - 0.9).abs() < 0.02, "medios = {}", s.medios);
        // A -40 dB (amplitud 0,01) queda en 20/60.
        let s = analizar(&seno(EN_MEDIOS, 0.01, TASA as usize));
        assert!((s.medios - 1.0 / 3.0).abs() < 0.03, "medios = {}", s.medios);
    }

    #[test]
    fn el_suavizado_sube_rapido_y_baja_lento() {
        let mut a = AnalizadorDeAudio::nuevo(TASA);
        // Se mide lo mismo en los dos sentidos: cuántos análisis tarda en cruzar el 50 %.
        let mut pasos_arriba = 0;
        let mut v = 0.0;
        while v < 0.5 && pasos_arriba < 1000 {
            v = a.entrar(&seno(EN_MEDIOS, 1.0, SALTO)).medios;
            pasos_arriba += 1;
        }
        // Se deja llegar al tope antes de cortar el sonido.
        for _ in 0..200 {
            v = a.entrar(&seno(EN_MEDIOS, 1.0, SALTO)).medios;
        }
        assert!(v > 0.95, "debía llegar al tope: {v}");
        let mut pasos_abajo = 0;
        while v > 0.5 && pasos_abajo < 1000 {
            v = a.entrar(&vec![0.0; SALTO]).medios;
            pasos_abajo += 1;
        }
        assert!(
            pasos_abajo > 2 * pasos_arriba,
            "subió en {pasos_arriba} pasos y bajó en {pasos_abajo}: debía bajar más lento"
        );
    }

    #[test]
    fn sin_sonido_las_senales_caen_hacia_cero() {
        let s = Senales {
            nivel: 1.0,
            graves: 0.8,
            medios: 0.6,
            agudos: 0.4,
        };
        assert_eq!(s.sin_sonido(0.0), s);
        let medio_segundo = s.sin_sonido(0.5);
        assert!(medio_segundo.nivel < 0.2 && medio_segundo.nivel > 0.0);
        assert!(s.sin_sonido(10.0).nivel < 1e-6);
    }

    #[test]
    fn juntar_las_muestras_de_a_pedazos_da_lo_mismo_que_todas_juntas() {
        let todo = seno(EN_GRAVES, 1.0, 8192);
        let junto = analizar(&todo);
        let mut a = AnalizadorDeAudio::nuevo(TASA);
        let mut ultimo = Senales::default();
        for pedazo in todo.chunks(300) {
            ultimo = a.entrar(pedazo);
        }
        assert!((junto.graves - ultimo.graves).abs() < 0.05);
    }
}
