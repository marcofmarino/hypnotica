#!/usr/bin/env bash
# Instala en Arch Linux lo que hypnotica necesita para compilar y correr.
# Solo instala lo que falta (`pacman -S --needed` + chequeo previo con `pacman -Qq`).
#
# Uso:
#   ./instalar_dependencias.sh              instala lo que falte
#   ./instalar_dependencias.sh --simular    solo muestra qué haría
#   ./instalar_dependencias.sh --camara     además instala v4l2loopback (cámara virtual de OBS)
set -euo pipefail

SIMULAR=0
CAMARA=0
for arg in "$@"; do
    case "$arg" in
        --simular) SIMULAR=1 ;;
        --camara) CAMARA=1 ;;
        *) echo "Opción desconocida: $arg" >&2; exit 1 ;;
    esac
done

if ! command -v pacman >/dev/null; then
    echo "Este script es solo para Arch Linux." >&2
    exit 1
fi

# Paquetes que vamos a pedir; después se filtran los que ya están.
paquetes=(
    # Compilar
    base-devel pkgconf clang mold
    # Bevy: audio, entrada, ventanas Wayland, gráficos
    alsa-lib systemd-libs wayland libxkbcommon vulkan-icd-loader
    # Video: GStreamer (los -dev vienen en el mismo paquete en Arch)
    gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav
    # Captura de pantalla: PipeWire + portal
    pipewire gst-plugin-pipewire xdg-desktop-portal
)

# --- Portal según el escritorio (el selector de archivos y la captura lo usan) ---
escritorio="${XDG_CURRENT_DESKTOP:-}"
case "${escritorio,,}" in
    *hyprland*) paquetes+=(xdg-desktop-portal-hyprland xdg-desktop-portal-gtk) ;;
    *kde*)      paquetes+=(xdg-desktop-portal-kde) ;;
    *gnome*)    paquetes+=(xdg-desktop-portal-gnome) ;;
    *sway*|*wlroots*) paquetes+=(xdg-desktop-portal-wlr xdg-desktop-portal-gtk) ;;
    *) paquetes+=(xdg-desktop-portal-gtk)
       echo "Escritorio no reconocido ('$escritorio'): instalo xdg-desktop-portal-gtk." ;;
esac

# --- GPU: se detecta por el fabricante que informa el kernel ---
hay_intel=0; hay_nvidia=0; hay_amd=0
for f in /sys/class/drm/card*/device/vendor; do
    [ -r "$f" ] || continue
    case "$(cat "$f")" in
        0x8086) hay_intel=1 ;;
        0x10de) hay_nvidia=1 ;;
        0x1002) hay_amd=1 ;;
    esac
done

if [ "$hay_intel" = 1 ]; then
    echo "GPU Intel detectada: vulkan-intel + intel-media-driver."
    paquetes+=(vulkan-intel intel-media-driver)
fi
if [ "$hay_amd" = 1 ]; then
    echo "GPU AMD detectada: vulkan-radeon."
    paquetes+=(vulkan-radeon)
fi

# --- Rust: hace falta 1.85 o más (el proyecto usa edition 2024) ---
if ! command -v cargo >/dev/null; then
    paquetes+=(rustup)
    hace_falta_rustup_default=1
else
    hace_falta_rustup_default=0
    version=$(rustc --version | awk '{print $2}')
    menor=$(echo "$version" | cut -d. -f2)
    if [ "$(echo "$version" | cut -d. -f1)" -lt 1 ] || [ "$menor" -lt 85 ]; then
        echo "AVISO: tu Rust es $version y se necesita 1.85 o más. Probá: rustup update stable" >&2
    fi
fi

if [ "$CAMARA" = 1 ]; then
    paquetes+=(v4l2loopback-dkms linux-headers)
fi

# --- Quedarse solo con lo que falta ---
faltan=()
for p in "${paquetes[@]}"; do
    # base-devel es un grupo: pacman -Qq no lo ve como paquete, se deja que pacman lo resuelva.
    if [ "$p" = base-devel ] || ! pacman -Qq "$p" >/dev/null 2>&1; then
        faltan+=("$p")
    fi
done

if [ "${#faltan[@]}" -eq 0 ]; then
    echo "Todo instalado: no falta ningún paquete."
else
    echo "Falta instalar: ${faltan[*]}"
    if [ "$SIMULAR" = 1 ]; then
        echo "(simulación: no se instala nada)"
    else
        # --needed: si algo ya estaba (p. ej. base-devel), pacman lo saltea.
        sudo pacman -S --needed "${faltan[@]}"
    fi
fi

if [ "$hace_falta_rustup_default" = 1 ] && [ "$SIMULAR" = 0 ]; then
    rustup default stable
fi

# --- NVIDIA: el driver depende del kernel, así que NO se instala solo ---
if [ "$hay_nvidia" = 1 ]; then
    echo
    echo "GPU NVIDIA detectada."
    if pacman -Qq nvidia-utils >/dev/null 2>&1; then
        echo "  nvidia-utils está instalado (da el Vulkan de NVIDIA)."
    else
        cat <<'EOF'
  Falta el driver. No lo instalo yo porque el módulo de kernel tiene que
  coincidir con tu kernel:
    - kernel 'linux':        sudo pacman -S nvidia-open nvidia-utils
    - otro kernel (zen, lts): sudo pacman -S nvidia-open-dkms nvidia-utils (y sus headers)
    - CachyOS: paquete linux-cachyos-nvidia-open (o nvidia-open-dkms)
  Después reiniciá y verificá con:  vulkaninfo --summary
EOF
    fi
    if [ "$hay_intel" = 1 ]; then
        echo "  Tiene Intel y NVIDIA: la ventana usa la GPU que elija wgpu. Para forzar una:"
        echo "    WGPU_ADAPTER_NAME=nvidia ./hypnotica   (o 'intel')"
    fi
fi

echo
echo "Listo. Verificá GStreamer con:  gst-inspect-1.0 pipewiresrc | head -3"
echo "Para correr: cargo run --release  (desde la carpeta del proyecto)"
