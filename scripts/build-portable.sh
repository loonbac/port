#!/usr/bin/env bash
# Compila el binario de release de PORT contra un glibc antiguo.
#
# Antes este script producia ademas un bundle portable autocontenido (un
# directorio con INSTALL.sh y las librerias al lado del binario). Ese formato se
# retiro por decision del usuario: PORT se instala como cualquier terminal, con
# el paquete nativo de la distribucion (.deb / .rpm), que deja el binario en
# /usr/bin y el .desktop y el icono en las rutas del sistema. El nombre del
# fichero se conserva porque sigue significando lo mismo: produce un binario
# "portable" en el sentido de que solo depende del glibc del sistema.
#
# Es el binario que consumen scripts/package.sh (ramas deb/rpm) y que prueba
# scripts/test-install.sh. Por que no se usa el de Nix: un binario enlazado por
# Nix lleva el intérprete del store grabado dentro
# (/nix/store/.../ld-linux-x86-64.so.2), y ese binario funciona dentro de Nix y
# en ningún otro sitio.
#
# Para que corra en la mayor parte de distribuciones hay dos reglas:
#
#  1. Enlazar contra el glibc MAS ANTIGUO que se quiera soportar, nunca contra
#     el del sistema que compila. Por eso se compila en Debian 12 (glibc 2.36) en
#     vez de en la máquina del desarrollador, que puede tener 2.42 y no arrancar
#     en Ubuntu 22.04. De ahi que los paquetes corran en Ubuntu 22.04+ y
#     Fedora 36+.
#  2. Dejar el binario sin dependencias del store y sin RUNPATH raro: las
#     librerias de X11/xkb/Wayland/Vulkan las aporta el sistema, y por eso
#     `package.sh` las declara como Depends/Requires.
#
# Requisitos: docker (o podman) y, solo si no hay docker, las librerías de
# desarrollo en el sistema.
set -euo pipefail

# shellcheck source=scripts/version.sh
source "$(dirname "${BASH_SOURCE[0]}")/version.sh"

VERSION="$PORT_VERSION"
# Debian 12 trae glibc 2.36, que es el suelo razonable: Ubuntu 22.04, Fedora 36
# yRHEL/Rocky 9.3 arrancan con el mismo binario.
# Nombre completamente cualificado. `debian:12-slim` a secas no lo resuelven
# ni docker ni podman en un entorno limpio: docker lo acepta por costumbre,
# podman exige el registro explicito y falla con "short-name did not resolve".
# Con el nombre completo funciona en los dos motores.
IMAGE="docker.io/library/debian:12-slim"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

have_docker() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }
have_podman() { command -v podman >/dev/null 2>&1 && podman info >/dev/null 2>&1; }

build_with_docker() {
  local engine="$1"
  echo "==> Compilando dentro de $IMAGE con $engine"
  # `-i` no es opcional: sin ella el motor no engancha stdin, el heredoc de
  # abajo nunca llega a `sh -s` y el contenedor sale con codigo 0 sin haber
  # compilado nada. El fallo se descubre mas tarde, cuando falta el binario,
  # y parece un problema de empaquetado.
  "$engine" run --rm -i \
    -v "$ROOT:/src:rw" \
    -w /src \
    -e PORT_VERSION="$VERSION" \
    "$IMAGE" \
    sh -s <<'DOCKER'
set -eu
export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl build-essential bash pkg-config cmake nasm \
  python3 patchelf xz-utils \
  libxcb1-dev libxcb-xkb-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libvulkan-dev libfreetype-dev \
  libfontconfig-dev libharfbuzz-dev

# Rust viene en rustup, que es la unica via de tener una version concreta
# sin depender del repositorio de la distro.
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"

# Las librerias de GPU van en el RUNPATH del propio binario. GPUI abre
# Wayland y Vulkan con dlopen, y sus rutas cambian entre distribuciones.
RUNTIME_LIBS="/usr/lib/x86_64-linux-gnu"
RUSTFLAGS=""
for lib in libxcb libxcb-xkb libxkbcommon libxkbcommon-x11 libfreetype libwayland-client libvulkan; do
  found="$(find /usr/lib /lib -name "${lib}.so*" -type f 2>/dev/null | head -1 || true)"
  dir="$(dirname "$found" 2>/dev/null || true)"
  if [ -n "$dir" ]; then
    RUSTFLAGS="$RUSTFLAGS -C link-arg=-Wl,-rpath,$dir"
  fi
done
export RUSTFLAGS
export LIBRARY_PATH="$RUNTIME_LIBS"
export PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig"

cargo fetch --locked || cargo fetch
python3 /src/scripts/patch-gpui.py /root/.cargo/registry /cargo
cargo build --release --locked -p port

mkdir -p /src/target/portable/bin
cp target/release/port /src/target/portable/bin/port

# El intérprete debe ser el del sistema, no uno del store.
interp="$(readelf -l target/portable/bin/port | sed -n 's/.*interpreter: \(.*\)\]/\1/p')"
case "$interp" in
  /nix/store/*)
    echo "ERROR: el intérprete sigue apuntando al store: $interp" >&2
    exit 1
    ;;
esac
echo "==> Interprete: $interp"

# El unico output es el binario. Se escribe como root dentro del contenedor,
# sobre un volumen montado del host, asi que queda con permisos de root y
# despues ningun paso del runner puede moverlo ni borrarlo:
#   rm: cannot remove 'target/portable/bin/port': Permission denied
# Se abren los permisos aqui, que es el ultimo momento en que se es root.
chmod -R a+rwX /src/target/portable
DOCKER
}

build_native() {
  echo "==> Compilando de forma nativa (sin docker)"
  local missing=0 lib
  for lib in libxcb libxcb-xkb libxkbcommon libxkbcommon-x11 freetype; do
    pkg-config --exists "$lib" 2>/dev/null || {
      echo "    falta $lib" >&2
      missing=1
    }
  done
  [ "$missing" -eq 0 ] || die "instala las librerias de desarrollo o usa docker"

  command -v patchelf >/dev/null 2>&1 || die "patchelf es necesario"
  cargo fetch --locked || cargo fetch
  python3 "$ROOT/scripts/patch-gpui.py"
  cargo build --release --locked -p port

  mkdir -p target/portable/bin
  cp target/release/port target/portable/bin/port
}

die() { echo "error: $*" >&2; exit 1; }

main() {
  if have_docker; then
    build_with_docker docker
  elif have_podman; then
    build_with_docker podman
  else
    echo "AVISO: no hay docker ni podman; se compila de forma nativa." >&2
    echo "       El binario solo sera portable si tu glibc es reciente." >&2
    build_native
  fi

  echo
  echo "Listo. Binario de release en:"
  echo "  target/portable/bin/port"
}

main "$@"
