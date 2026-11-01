# Hypnotica

Software de **video mapping** para Linux (Wayland), escrito en Rust con [Bevy](https://bevyengine.org) 0.20.

> **Proyecto personal.** Hypnotica no está pensado como un programa listo para que lo use cualquiera.
>
> Estado: en desarrollo.

## Qué hace

- **Composición por capas y clips**: una grilla de capas × columnas, con disparo de clips, opacidad, modos de mezcla (normal, suma, multiplicar, pantalla) y fundido entre clips.
- **Fuentes**: videos, imágenes fijas (PNG, JPG, WebP), cámaras (V4L2, incluida la cámara virtual de OBS) y **captura de pantalla o ventana** por el portal de escritorio (PipeWire), con importación DMA-BUF cuando la GPU lo permite.
- **Superficies** por salida: corner pin exacto por píxel (homografía), **mesh warp** (grilla de hasta 16×16 con interpolación suave), recorte con polígonos, huecos, **máscaras** por proyector y **edge blending** (borde suave) para varios proyectores.
- **Una ventana fullscreen por proyector**, con patrones de prueba y calibración de color.

## Cómo está armado

Cada funcionalidad es un **plugin de Bevy** en su propio crate, y se agrega o quita con una línea en `app/src/main.rs`. La matemática y el modelo de datos están en código Rust puro, sin depender de Bevy.

| Crate | Qué hace |
|---|---|
| `vm_core` | Modelo de datos puro, guardado/carga, homografía, mallas, polígonos, audio (FFT), control. Con tests |
| `vm_video` | Decodificación con GStreamer, cámaras, captura de pantalla, DMA-BUF |
| `vm_composicion` | Dibuja las capas en una textura y ejecuta los disparos de clips |
| `vm_warp` | Superficies: warp, máscaras, edge blending y patrones (shaders propios) |
| `vm_output` | Ventanas y monitores |
| `vm_audio` | Captura y análisis de audio (PipeWire) |
| `vm_control` | OSC y MIDI |
| `vm_proyecto` | Proyecto actual, guardar/abrir, historial |
| `vm_ui` | Interfaz con egui |
| `app` | El ejecutable `hypnotica`, que une los plugins |

## Requisitos

- Linux con **Wayland** y una GPU con **Vulkan**.
- **Rust 1.85 o más** (usa la edición 2024).
- GStreamer (`gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav`), PipeWire y un `xdg-desktop-portal` con el backend de tu escritorio.

En **Arch Linux** hay un script que instala lo que falte según tu escritorio y tu GPU:

```bash
./instalar_dependencias.sh             # instala
./instalar_dependencias.sh --camara    # además v4l2loopback (cámara virtual de OBS)
```

En otras distribuciones, instalá los equivalentes de esos paquetes.

## Compilar y correr

```bash
cargo run                  # desarrollo (usa dynamic_linking de Bevy, compila más rápido)
o
cargo run --release   
o
env -u DISPLAY cargo run   # fuerza Wayland nativo (sin XWayland)
```

Para un ejecutable independiente (sin `dynamic_linking`):

```bash
cargo build --release --no-default-features
```

## Limitaciones conocidas

- Probado en GNOME con una GPU AMD..
- Con monitores de distinta frecuencia, el bucle de render se ata al más lento (la ventana de control intenta no frenar al proyector, pero no está verificado).
- Sin probar con controladores MIDI/OSC.
- En Wayland no se puede arrastrar y soltar para cargar archivos.

## Licencia

[GPL-3.0-or-later](LICENSE). 
Autor: Marco Marino.
