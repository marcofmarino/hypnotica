//! Deshacer y rehacer, y saber si hay cambios sin guardar. Rust puro (no usa Bevy).
//!
//! El proyecto es un solo dato, así que el historial guarda copias de estados anteriores.
//! Un arrastre de mouse cambia el proyecto en cada frame, y no queremos un paso de
//! "deshacer" por frame: los cambios seguidos se **agrupan**. Cuando el proyecto lleva
//! `PAUSA` segundos sin cambiar, el estado se *confirma* y pasa a ser un paso del historial.

use vm_core::Proyecto;

/// Cuántos segundos sin cambios hacen falta para dar por terminada una edición.
pub const PAUSA: f64 = 0.5;

/// Cuántos pasos de deshacer se recuerdan como máximo.
const MAXIMO_DE_PASOS: usize = 100;

#[derive(Debug, Clone)]
pub struct Historial {
    pasado: Vec<Proyecto>,
    futuro: Vec<Proyecto>,
    /// El último estado estable: el que se apila cuando llega un cambio nuevo.
    confirmado: Proyecto,
    /// Cuándo se vio por última vez un cambio todavía sin confirmar.
    ultimo_cambio: Option<f64>,
    /// Cómo estaba el proyecto la última vez que se guardó o se cargó.
    guardado: Proyecto,
}

impl Historial {
    pub fn nuevo(proyecto: &Proyecto) -> Self {
        Historial {
            pasado: Vec::new(),
            futuro: Vec::new(),
            confirmado: proyecto.clone(),
            ultimo_cambio: None,
            guardado: proyecto.clone(),
        }
    }

    /// Empieza de cero con este proyecto (se cargó uno): sin pasos y sin cambios pendientes.
    pub fn reiniciar(&mut self, proyecto: &Proyecto) {
        *self = Historial::nuevo(proyecto);
    }

    /// Empieza de cero con este proyecto pero **conservando lo que se tiene por guardado**:
    /// sirve cuando se recupera un autoguardado (el proyecto cambió respecto del archivo).
    pub fn reiniciar_sobre(&mut self, actual: &Proyecto) {
        let guardado = self.guardado.clone();
        *self = Historial::nuevo(actual);
        self.guardado = guardado;
    }

    /// Recuerda que el proyecto, tal como está, quedó guardado en disco.
    pub fn marcar_guardado(&mut self, proyecto: &Proyecto) {
        self.guardado = proyecto.clone();
    }

    /// ¿Hay cambios que todavía no se guardaron?
    pub fn sin_guardar(&self, actual: &Proyecto) -> bool {
        *actual != self.guardado
    }

    /// Se llama en cada frame. `cambio` dice si el proyecto se modificó desde el frame
    /// anterior y `ahora` son los segundos transcurridos.
    pub fn observar(&mut self, actual: &Proyecto, cambio: bool, ahora: f64) {
        if cambio {
            self.ultimo_cambio = Some(ahora);
        }
        if let Some(t) = self.ultimo_cambio
            && ahora - t >= PAUSA
        {
            self.confirmar(actual);
        }
    }

    /// Da por terminada la edición en curso: si el proyecto es distinto del último estado
    /// estable, ese estado pasa al historial y se pierde lo que se podía rehacer.
    fn confirmar(&mut self, actual: &Proyecto) {
        self.ultimo_cambio = None;
        if *actual != self.confirmado {
            let anterior = std::mem::replace(&mut self.confirmado, actual.clone());
            self.pasado.push(anterior);
            if self.pasado.len() > MAXIMO_DE_PASOS {
                self.pasado.remove(0);
            }
            self.futuro.clear();
        }
    }

    pub fn puede_deshacer(&self) -> bool {
        !self.pasado.is_empty() || self.ultimo_cambio.is_some()
    }

    pub fn puede_rehacer(&self) -> bool {
        !self.futuro.is_empty()
    }

    /// El estado anterior, si lo hay. Lo que se estaba editando entra en la lista de lo que
    /// se puede rehacer.
    pub fn deshacer(&mut self, actual: &Proyecto) -> Option<Proyecto> {
        self.confirmar(actual);
        let anterior = self.pasado.pop()?;
        self.futuro.push(self.confirmado.clone());
        self.confirmado = anterior.clone();
        Some(anterior)
    }

    /// El estado siguiente, si se había deshecho algo.
    pub fn rehacer(&mut self, actual: &Proyecto) -> Option<Proyecto> {
        self.confirmar(actual);
        let siguiente = self.futuro.pop()?;
        self.pasado.push(self.confirmado.clone());
        self.confirmado = siguiente.clone();
        Some(siguiente)
    }
}

/// ¿Cambió la **estructura** de la composición entre dos estados? Es decir: la cantidad de
/// capas o de columnas, o qué clip hay en cada celda (mover o borrar capas y clips cambia
/// eso). En ese caso, quien reproduce tiene que empezar de nuevo; con cambios de opacidad,
/// nombre o esquinas, no hace falta.
pub fn cambio_estructural(a: &Proyecto, b: &Proyecto) -> bool {
    let (x, y) = (&a.composicion, &b.composicion);
    x.capas.len() != y.capas.len()
        || x.columnas.len() != y.columnas.len()
        || x.capas
            .iter()
            .zip(&y.capas)
            .any(|(c, d)| c.celdas != d.celdas)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un proyecto que se distingue por su ancho de composición.
    fn con_ancho(ancho: u32) -> Proyecto {
        let mut p = Proyecto::default();
        p.composicion.ancho = ancho;
        p
    }

    /// Simula una edición: el proyecto cambia en `p` y pasa la pausa.
    fn editar(h: &mut Historial, p: &Proyecto, t: &mut f64) {
        h.observar(p, true, *t);
        *t += PAUSA + 0.1;
        h.observar(p, false, *t);
    }

    #[test]
    fn los_cambios_seguidos_son_un_solo_paso() {
        let mut h = Historial::nuevo(&con_ancho(1000));
        let mut t = 0.0;
        // Un arrastre: cambia en cada frame durante un segundo.
        for i in 1..=60 {
            h.observar(&con_ancho(1000 + i), true, t);
            t += 1.0 / 60.0;
        }
        t += PAUSA + 0.1;
        h.observar(&con_ancho(1060), false, t);
        assert_eq!(h.pasado.len(), 1);
        // Un solo deshacer vuelve al estado de antes del arrastre.
        assert_eq!(h.deshacer(&con_ancho(1060)), Some(con_ancho(1000)));
        assert!(!h.puede_deshacer());
    }

    #[test]
    fn deshacer_y_rehacer_van_y_vienen() {
        let mut h = Historial::nuevo(&con_ancho(1));
        let mut t = 0.0;
        editar(&mut h, &con_ancho(2), &mut t);
        editar(&mut h, &con_ancho(3), &mut t);
        assert_eq!(h.deshacer(&con_ancho(3)), Some(con_ancho(2)));
        assert_eq!(h.deshacer(&con_ancho(2)), Some(con_ancho(1)));
        assert_eq!(h.deshacer(&con_ancho(1)), None);
        assert_eq!(h.rehacer(&con_ancho(1)), Some(con_ancho(2)));
        assert_eq!(h.rehacer(&con_ancho(2)), Some(con_ancho(3)));
        assert_eq!(h.rehacer(&con_ancho(3)), None);
    }

    #[test]
    fn un_cambio_nuevo_borra_lo_que_se_podia_rehacer() {
        let mut h = Historial::nuevo(&con_ancho(1));
        let mut t = 0.0;
        editar(&mut h, &con_ancho(2), &mut t);
        assert_eq!(h.deshacer(&con_ancho(2)), Some(con_ancho(1)));
        assert!(h.puede_rehacer());
        editar(&mut h, &con_ancho(9), &mut t);
        assert!(!h.puede_rehacer());
    }

    #[test]
    fn se_puede_deshacer_aun_antes_de_que_termine_la_pausa() {
        let mut h = Historial::nuevo(&con_ancho(1));
        h.observar(&con_ancho(2), true, 0.0);
        // Todavía no pasó la pausa, pero se ve que hay algo para deshacer.
        assert!(h.puede_deshacer());
        assert_eq!(h.deshacer(&con_ancho(2)), Some(con_ancho(1)));
    }

    #[test]
    fn volver_al_mismo_estado_no_agrega_un_paso() {
        let mut h = Historial::nuevo(&con_ancho(1));
        let mut t = 0.0;
        // Cambia y vuelve a lo mismo dentro de la misma edición.
        h.observar(&con_ancho(5), true, t);
        t += 0.1;
        h.observar(&con_ancho(1), true, t);
        t += PAUSA + 0.1;
        h.observar(&con_ancho(1), false, t);
        assert!(h.pasado.is_empty());
    }

    #[test]
    fn el_historial_tiene_un_tope() {
        let mut h = Historial::nuevo(&con_ancho(0));
        let mut t = 0.0;
        for i in 1..=(MAXIMO_DE_PASOS as u32 + 20) {
            editar(&mut h, &con_ancho(i), &mut t);
        }
        assert_eq!(h.pasado.len(), MAXIMO_DE_PASOS);
    }

    #[test]
    fn se_sabe_si_hay_cambios_sin_guardar() {
        let mut h = Historial::nuevo(&con_ancho(1));
        assert!(!h.sin_guardar(&con_ancho(1)));
        assert!(h.sin_guardar(&con_ancho(2)));
        h.marcar_guardado(&con_ancho(2));
        assert!(!h.sin_guardar(&con_ancho(2)));
        // Deshacer hasta lo guardado también cuenta como "sin cambios".
        assert!(h.sin_guardar(&con_ancho(1)));
    }

    #[test]
    fn reiniciar_borra_todo_y_toma_el_proyecto_como_guardado() {
        let mut h = Historial::nuevo(&con_ancho(1));
        let mut t = 0.0;
        editar(&mut h, &con_ancho(2), &mut t);
        h.reiniciar(&con_ancho(7));
        assert!(!h.puede_deshacer() && !h.puede_rehacer());
        assert!(!h.sin_guardar(&con_ancho(7)));
    }

    #[test]
    fn solo_los_cambios_de_capas_y_clips_son_estructurales() {
        use vm_core::ConfigClip;
        let base = Proyecto::default();
        // Opacidad, nombre y ancho no son estructurales.
        let mut otro = base.clone();
        otro.composicion.capas[0].opacidad = 0.3;
        otro.composicion.capas[0].nombre = "x".into();
        otro.composicion.ancho = 1280;
        assert!(!cambio_estructural(&base, &otro));
        // Una capa más sí.
        let mut con_capa = base.clone();
        con_capa.composicion.agregar_capa();
        assert!(cambio_estructural(&base, &con_capa));
        // Un clip nuevo en una celda también.
        let mut con_clip = base.clone();
        con_clip
            .composicion
            .poner_clip(0, 0, ConfigClip::de_video("a.mp4".into()));
        assert!(cambio_estructural(&base, &con_clip));
    }

    #[test]
    fn reiniciar_sobre_conserva_lo_guardado() {
        let mut h = Historial::nuevo(&con_ancho(1));
        h.reiniciar_sobre(&con_ancho(5));
        // Lo guardado sigue siendo el ancho 1: el proyecto está "con cambios".
        assert!(h.sin_guardar(&con_ancho(5)));
        assert!(!h.sin_guardar(&con_ancho(1)));
        assert!(!h.puede_deshacer());
    }
}
