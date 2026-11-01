//! Los temas de color de la interfaz: una paleta con nombre (Tierra, Gruvbox, Catppuccin,
//! Nord) y un tema "en vivo" que toma los colores del esquema de Caelestia (el que usa
//! tu escritorio) y se actualiza solo cuando ese esquema cambia.
//!
//! Lógica casi pura: solo usa `Color32` de egui. El tema en uso vive en una variable global
//! (`tema()`), así cualquier parte de la interfaz pide `azul()`, `verde()`... sin recibir
//! el tema como parámetro.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use bevy_egui::egui::Color32;

/// Los colores de un tema. Los roles siguen a Material (los de Caelestia): fondo y capas
/// de superficie, bordes, texto, el color principal y unos acentos para los íconos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tema {
    /// ¿Es un tema oscuro? (egui parte de su base oscura o clara según esto.)
    pub oscuro: bool,
    pub fondo: Color32,
    /// Paneles y barras.
    pub panel: Color32,
    /// Botones y tarjetas.
    pub contenedor: Color32,
    /// Botones con el mouse encima.
    pub contenedor_alto: Color32,
    /// Campos de texto y fondos hundidos.
    pub campo: Color32,
    pub borde: Color32,
    pub texto: Color32,
    pub texto_suave: Color32,
    /// El color principal (selección, foco).
    pub primario: Color32,
    /// Fondo de lo seleccionado.
    pub seleccion: Color32,
    /// El color que marca lo ELEGIDO en la grilla (el clip y la capa): el principal, más vivo y
    /// sin pasarse de saturado (ver `acento_de`).
    pub acento: Color32,
    /// Acentos de los íconos, con un papel cada uno (ver las funciones de abajo).
    pub azul: Color32,
    pub verde: Color32,
    pub naranja: Color32,
    pub rosa: Color32,
    pub rojo: Color32,
    pub amarillo: Color32,
}

/// Un color a partir de `0xRRGGBB`.
const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Tema {
    /// Marrón, naranja y tonos tierra: los colores del esquema de Caelestia del escritorio.
    pub fn tierra() -> Tema {
        Tema {
            oscuro: true,
            fondo: rgb(0x120d08),
            panel: rgb(0x19120b),
            contenedor: rgb(0x271e15),
            contenedor_alto: rgb(0x352a1f),
            campo: rgb(0x0e0a06),
            borde: rgb(0x52463a),
            texto: rgb(0xf5e2d2),
            texto_suave: rgb(0xb9a899),
            primario: rgb(0xedbe8b),
            seleccion: rgb(0x6e4b22),
            acento: acento_de(rgb(0xedbe8b), true),
            azul: rgb(0xedbe8b),
            verde: rgb(0xb5ccba),
            naranja: rgb(0xea975a),
            rosa: rgb(0xffad85),
            rojo: rgb(0xf97758),
            amarillo: rgb(0xeec36c),
        }
    }

    pub fn gruvbox() -> Tema {
        Tema {
            oscuro: true,
            fondo: rgb(0x1d2021),
            panel: rgb(0x282828),
            contenedor: rgb(0x3c3836),
            contenedor_alto: rgb(0x504945),
            campo: rgb(0x1d2021),
            borde: rgb(0x665c54),
            texto: rgb(0xfbf1c7),
            texto_suave: rgb(0xa89984),
            primario: rgb(0xfabd2f),
            seleccion: rgb(0x6b5a1f),
            acento: acento_de(rgb(0xfabd2f), true),
            azul: rgb(0x83a598),
            verde: rgb(0xb8bb26),
            naranja: rgb(0xfe8019),
            rosa: rgb(0xd3869b),
            rojo: rgb(0xfb4934),
            amarillo: rgb(0xfabd2f),
        }
    }

    pub fn catppuccin() -> Tema {
        Tema {
            oscuro: true,
            fondo: rgb(0x11111b),
            panel: rgb(0x1e1e2e),
            contenedor: rgb(0x313244),
            contenedor_alto: rgb(0x45475a),
            campo: rgb(0x181825),
            borde: rgb(0x585b70),
            texto: rgb(0xcdd6f4),
            texto_suave: rgb(0xa6adc8),
            primario: rgb(0xcba6f7),
            seleccion: rgb(0x4a3f73),
            acento: acento_de(rgb(0xcba6f7), true),
            azul: rgb(0x89b4fa),
            verde: rgb(0xa6e3a1),
            naranja: rgb(0xfab387),
            rosa: rgb(0xf5c2e7),
            rojo: rgb(0xf38ba8),
            amarillo: rgb(0xf9e2af),
        }
    }

    pub fn nord() -> Tema {
        Tema {
            oscuro: true,
            fondo: rgb(0x242933),
            panel: rgb(0x2e3440),
            contenedor: rgb(0x3b4252),
            contenedor_alto: rgb(0x4c566a),
            campo: rgb(0x242933),
            borde: rgb(0x4c566a),
            texto: rgb(0xeceff4),
            texto_suave: rgb(0xd8dee9),
            primario: rgb(0x88c0d0),
            seleccion: rgb(0x3d5a6a),
            acento: acento_de(rgb(0x88c0d0), true),
            azul: rgb(0x81a1c1),
            verde: rgb(0xa3be8c),
            naranja: rgb(0xd08770),
            rosa: rgb(0xb48ead),
            rojo: rgb(0xbf616a),
            amarillo: rgb(0xebcb8b),
        }
    }
}

/// Qué tema se eligió.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Esquema {
    Tierra,
    Gruvbox,
    Catppuccin,
    Nord,
    /// Los colores del esquema de Caelestia, en vivo.
    Caelestia,
}

impl Esquema {
    pub const TODOS: [Esquema; 5] = [
        Esquema::Tierra,
        Esquema::Gruvbox,
        Esquema::Catppuccin,
        Esquema::Nord,
        Esquema::Caelestia,
    ];

    pub fn nombre(self) -> &'static str {
        match self {
            Esquema::Tierra => "Tierra",
            Esquema::Gruvbox => "Gruvbox",
            Esquema::Catppuccin => "Catppuccin Mocha",
            Esquema::Nord => "Nord",
            Esquema::Caelestia => "Caelestia (en vivo)",
        }
    }

    pub fn desde_nombre(nombre: &str) -> Option<Esquema> {
        Esquema::TODOS
            .into_iter()
            .find(|e| e.nombre().eq_ignore_ascii_case(nombre.trim()))
    }

    /// El tema de este esquema. Si Caelestia no está (o su archivo no se entiende), se usa
    /// Tierra, que es el mismo estilo.
    pub fn tema(self) -> Tema {
        match self {
            Esquema::Tierra => Tema::tierra(),
            Esquema::Gruvbox => Tema::gruvbox(),
            Esquema::Catppuccin => Tema::catppuccin(),
            Esquema::Nord => Tema::nord(),
            Esquema::Caelestia => leer_caelestia().unwrap_or_else(Tema::tierra),
        }
    }
}

// ---------------------------------------------------------------------------------------
// El tema en uso
// ---------------------------------------------------------------------------------------

static ACTUAL: LazyLock<RwLock<Tema>> = LazyLock::new(|| RwLock::new(Tema::tierra()));

/// El tema en uso.
pub(crate) fn tema() -> Tema {
    ACTUAL.read().map_or_else(|e| *e.into_inner(), |t| *t)
}

/// Cambia el tema en uso.
pub(crate) fn poner(nuevo: Tema) {
    match ACTUAL.write() {
        Ok(mut t) => *t = nuevo,
        Err(e) => *e.into_inner() = nuevo,
    }
}

// Los colores de los íconos, por el papel que cumplen (no por su tono: en cada tema el
// "azul" es lo que ese tema usa para lo principal).
pub(crate) fn azul() -> Color32 {
    tema().azul
}
pub(crate) fn verde() -> Color32 {
    tema().verde
}
pub(crate) fn naranja() -> Color32 {
    tema().naranja
}
pub(crate) fn rosa() -> Color32 {
    tema().rosa
}
pub(crate) fn rojo() -> Color32 {
    tema().rojo
}
pub(crate) fn amarillo() -> Color32 {
    tema().amarillo
}
pub(crate) fn texto_suave() -> Color32 {
    tema().texto_suave
}
pub(crate) fn acento() -> Color32 {
    tema().acento
}

// ---------------------------------------------------------------------------------------
// Caelestia
// ---------------------------------------------------------------------------------------

/// El archivo donde Caelestia deja el esquema de colores actual.
pub(crate) fn ruta_caelestia() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("caelestia/scheme.json"))
}

/// Lee el esquema de Caelestia, si existe y se entiende.
pub(crate) fn leer_caelestia() -> Option<Tema> {
    let texto = std::fs::read_to_string(ruta_caelestia()?).ok()?;
    desde_caelestia(&texto)
}

/// El valor de `"clave": "RRGGBB"` en el texto del esquema (es un JSON plano: no hace falta
/// un lector de JSON completo para esto).
fn color_de(texto: &str, clave: &str) -> Option<Color32> {
    let buscado = format!("\"{clave}\"");
    let desde = texto.find(&buscado)? + buscado.len();
    let resto = texto[desde..].trim_start().strip_prefix(':')?.trim_start();
    let resto = resto.strip_prefix('"')?;
    let hex = resto.get(..6)?;
    u32::from_str_radix(hex, 16).ok().map(rgb)
}

/// Un tema a partir del texto de `scheme.json` de Caelestia. Devuelve `None` si falta algún
/// color necesario.
pub(crate) fn desde_caelestia(texto: &str) -> Option<Tema> {
    let c = |clave: &str| color_de(texto, clave);
    let oscuro = !texto.contains("\"mode\": \"light\"") && !texto.contains("\"mode\":\"light\"");
    let fondo = c("background")?;
    Some(Tema {
        oscuro,
        fondo,
        panel: c("surfaceContainerLow")?,
        contenedor: c("surfaceContainerHigh")?,
        contenedor_alto: c("surfaceBright")?,
        campo: c("surfaceContainerLowest").map_or(fondo, |x| mezcla(x, fondo, 0.5)),
        borde: c("outlineVariant")?,
        texto: c("onSurface")?,
        texto_suave: c("onSurfaceVariant")?,
        primario: c("primary")?,
        seleccion: c("primaryContainer")?,
        acento: acento_de(c("primary")?, oscuro),
        azul: c("primary")?,
        verde: c("success").or_else(|| c("tertiary"))?,
        naranja: c("term5").or_else(|| c("secondary"))?,
        rosa: c("mauve").or_else(|| c("tertiary"))?,
        rojo: c("error")?,
        amarillo: c("term6").or_else(|| c("tertiary"))?,
    })
}

/// Un tema intermedio entre `a` (`t = 0`) y `b` (`t = 1`), para la transición de color al
/// cambiar de esquema. Si uno es claro y el otro oscuro, el cambio de base pasa a mitad.
pub(crate) fn entre(a: &Tema, b: &Tema, t: f32) -> Tema {
    let m = |x: Color32, y: Color32| mezcla(x, y, t);
    Tema {
        oscuro: if t < 0.5 { a.oscuro } else { b.oscuro },
        fondo: m(a.fondo, b.fondo),
        panel: m(a.panel, b.panel),
        contenedor: m(a.contenedor, b.contenedor),
        contenedor_alto: m(a.contenedor_alto, b.contenedor_alto),
        campo: m(a.campo, b.campo),
        borde: m(a.borde, b.borde),
        texto: m(a.texto, b.texto),
        texto_suave: m(a.texto_suave, b.texto_suave),
        primario: m(a.primario, b.primario),
        seleccion: m(a.seleccion, b.seleccion),
        acento: m(a.acento, b.acento),
        azul: m(a.azul, b.azul),
        verde: m(a.verde, b.verde),
        naranja: m(a.naranja, b.naranja),
        rosa: m(a.rosa, b.rosa),
        rojo: m(a.rojo, b.rojo),
        amarillo: m(a.amarillo, b.amarillo),
    }
}

/// Mezcla dos colores: `t = 0` es `a` y `t = 1` es `b`.
/// El acento de un tema a partir de su color principal: el mismo tono pero VIVO (saturación al
/// menos `SATURACION_DEL_ACENTO`) y con una luminosidad que se ve bien sobre el fondo (más claro en
/// los temas oscuros, más oscuro en los claros). Así en Tierra, cuyo principal es un durazno
/// pálido, el acento es un naranja que se nota sin llegar a un naranja chillón.
pub(crate) fn acento_de(primario: Color32, oscuro: bool) -> Color32 {
    let (tono, saturacion, _) = a_hsl(primario);
    let luminosidad = if oscuro { 0.60 } else { 0.42 };
    desde_hsl(
        tono,
        saturacion.clamp(SATURACION_DEL_ACENTO, 0.9),
        luminosidad,
    )
}

/// La saturación mínima del acento: vivo, pero lejos del 100 %.
const SATURACION_DEL_ACENTO: f32 = 0.78;

/// De RGB a (tono en grados 0..360, saturación 0..1, luminosidad 0..1).
fn a_hsl(c: Color32) -> (f32, f32, f32) {
    let (r, g, b) = (
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
    );
    let maximo = r.max(g).max(b);
    let minimo = r.min(g).min(b);
    let luminosidad = (maximo + minimo) / 2.0;
    let delta = maximo - minimo;
    if delta < 1e-6 {
        return (0.0, 0.0, luminosidad);
    }
    let saturacion = delta / (1.0 - (2.0 * luminosidad - 1.0).abs());
    let tono = if maximo == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if maximo == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (tono, saturacion, luminosidad)
}

/// De (tono, saturación, luminosidad) a RGB.
fn desde_hsl(tono: f32, saturacion: f32, luminosidad: f32) -> Color32 {
    let croma = (1.0 - (2.0 * luminosidad - 1.0).abs()) * saturacion;
    let h = tono.rem_euclid(360.0) / 60.0;
    let x = croma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (croma, x, 0.0),
        1 => (x, croma, 0.0),
        2 => (0.0, croma, x),
        3 => (0.0, x, croma),
        4 => (x, 0.0, croma),
        _ => (croma, 0.0, x),
    };
    let m = luminosidad - croma / 2.0;
    let canal = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(canal(r), canal(g), canal(b))
}

fn mezcla(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

// ---------------------------------------------------------------------------------------
// Qué esquema se eligió: se recuerda entre sesiones
// ---------------------------------------------------------------------------------------

fn ruta_de_la_eleccion() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hypnotica/tema.txt"))
}

/// El esquema que se eligió la vez anterior (Tierra si no hay ninguno).
pub(crate) fn esquema_guardado() -> Esquema {
    ruta_de_la_eleccion()
        .and_then(|r| std::fs::read_to_string(r).ok())
        .and_then(|t| Esquema::desde_nombre(&t))
        .unwrap_or(Esquema::Tierra)
}

/// Recuerda el esquema elegido (si no se puede escribir, simplemente no se recuerda).
pub(crate) fn guardar_esquema(esquema: Esquema) {
    if let Some(ruta) = ruta_de_la_eleccion() {
        if let Some(carpeta) = ruta.parent() {
            let _ = std::fs::create_dir_all(carpeta);
        }
        let _ = std::fs::write(ruta, esquema.nombre());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MUESTRA: &str = r#"{"name": "dynamic", "mode": "dark", "colours": {"background": "120d08", "onBackground": "f5e2d2", "surfaceBright": "352a1f", "surfaceContainerLowest": "000000", "surfaceContainerLow": "19120b", "surfaceContainerHigh": "271e15", "outlineVariant": "52463a", "onSurface": "f5e2d2", "onSurfaceVariant": "b9a899", "primary": "edbe8b", "primaryDim": "deb07e", "primaryContainer": "6e4b22", "error": "f97758", "term5": "ea975a", "term6": "eec36c", "mauve": "ffad85", "success": "B5CCBA", "tertiary": "f9febb"}}"#;

    #[test]
    fn se_lee_un_color_por_su_clave_exacta() {
        assert_eq!(color_de(MUESTRA, "primary"), Some(rgb(0xedbe8b)));
        // "primary" no se confunde con "primaryDim" ni con "onPrimary".
        assert_eq!(color_de(MUESTRA, "primaryDim"), Some(rgb(0xdeb07e)));
        assert_eq!(color_de(MUESTRA, "onPrimary"), None);
        // Mayúsculas en el valor.
        assert_eq!(color_de(MUESTRA, "success"), Some(rgb(0xb5ccba)));
        assert_eq!(color_de(MUESTRA, "noExiste"), None);
    }

    #[test]
    fn el_esquema_de_caelestia_se_convierte_en_tema() {
        let t = desde_caelestia(MUESTRA).expect("tema");
        assert!(t.oscuro);
        assert_eq!(t.fondo, rgb(0x120d08));
        assert_eq!(t.primario, rgb(0xedbe8b));
        assert_eq!(t.seleccion, rgb(0x6e4b22));
        assert_eq!(t.rojo, rgb(0xf97758));
        assert_eq!(t.verde, rgb(0xb5ccba));
    }

    #[test]
    fn un_esquema_incompleto_no_se_acepta() {
        assert_eq!(
            desde_caelestia(r#"{"colours": {"primary": "ffffff"}}"#),
            None
        );
        assert_eq!(desde_caelestia("esto no es json"), None);
    }

    #[test]
    fn el_modo_claro_se_detecta() {
        let claro = MUESTRA.replace("\"mode\": \"dark\"", "\"mode\": \"light\"");
        assert!(!desde_caelestia(&claro).unwrap().oscuro);
    }

    #[test]
    fn los_nombres_de_esquema_ida_y_vuelta() {
        for e in Esquema::TODOS {
            assert_eq!(Esquema::desde_nombre(e.nombre()), Some(e));
        }
        assert_eq!(Esquema::desde_nombre("  gruvbox\n"), Some(Esquema::Gruvbox));
        assert_eq!(Esquema::desde_nombre("otro"), None);
    }

    #[test]
    fn la_mezcla_va_de_un_color_al_otro() {
        let (a, b) = (rgb(0x000000), rgb(0xfffefd));
        assert_eq!(mezcla(a, b, 0.0), a);
        assert_eq!(mezcla(a, b, 1.0), b);
    }

    #[test]
    fn el_tema_intermedio_va_de_un_tema_al_otro() {
        let (a, b) = (Tema::tierra(), Tema::nord());
        assert_eq!(entre(&a, &b, 0.0), a);
        assert_eq!(entre(&a, &b, 1.0), b);
        let medio = entre(&a, &b, 0.5);
        assert_ne!(medio.fondo, a.fondo);
        assert_ne!(medio.fondo, b.fondo);
    }

    #[test]
    fn el_acento_de_tierra_es_un_naranja_vivo_sin_pasarse() {
        let a = Tema::tierra().acento;
        let (tono, saturacion, luminosidad) = a_hsl(a);
        // Naranja: el mismo tono del durazno del tema (entre 25° y 40°).
        assert!((25.0..=40.0).contains(&tono), "tono {tono}");
        // Vivo pero no chillón.
        assert!(
            (0.75..=0.9).contains(&saturacion),
            "saturación {saturacion}"
        );
        assert!(
            (0.55..=0.65).contains(&luminosidad),
            "luminosidad {luminosidad}"
        );
        // Más vivo que el principal.
        assert!(a_hsl(Tema::tierra().primario).1 <= saturacion + 0.02);
        assert!(a.r() > a.g() && a.g() > a.b(), "{a:?}");
    }

    #[test]
    fn hsl_ida_y_vuelta_conserva_el_color() {
        for hex in [
            0xedbe8b, 0x88c0d0, 0xcba6f7, 0xfabd2f, 0x808080, 0x000000, 0xffffff,
        ] {
            let c = rgb(hex);
            let (h, s, l) = a_hsl(c);
            let v = desde_hsl(h, s, l);
            assert!(
                (c.r() as i32 - v.r() as i32).abs() <= 1
                    && (c.g() as i32 - v.g() as i32).abs() <= 1
                    && (c.b() as i32 - v.b() as i32).abs() <= 1,
                "{hex:x}: {c:?} -> {v:?}"
            );
        }
    }

    #[test]
    fn todos_los_temas_tienen_un_acento_distinto_del_fondo() {
        for t in [
            Tema::tierra(),
            Tema::gruvbox(),
            Tema::catppuccin(),
            Tema::nord(),
        ] {
            assert_ne!(t.acento, t.fondo);
            assert_ne!(t.acento, t.contenedor);
        }
    }
}
