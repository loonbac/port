#!/usr/bin/env bash
# Compila PORT contra el glibc del sistema y produce un bundle portable.
#
# Por qué esto existe en vez de usar el binario de Nix: un binario enlazado por
# Nix lleva el intérprete del store grabado dentro
# (/nix/store/.../ld-linux-x86-64.so.2). Ese binario funciona dentro de Nix y en
# ningún otro sitio, por muy autocontenido que parezca el directorio lib/.
#
# Para que el binario corra en la mayor parte de distribuciones hay dos reglas:
#
#  1. Enlazar contra el glibc MAS ANTIGUO que se quiera soportar, nunca contra
#     el del sistema que compila. Por eso se compila en Debian 12 (glibc 2.36) en
#     vez de en la máquina del desarrollador, que puede tener 2.42 y no arrancar
#     en Ubuntu 22.04.
#  2. Empaquetar las librerías de GPU/X11 al lado del binario con RUNPATH
#     relativo, porque GPUI las carga por dlopen y sus nombres de soname
#     varían entre distribuciones.
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

# El empaquetado corre DENTRO del contenedor, y no por gusto: las librerias
# que hay que empaquetar se instalaron aqui con apt. En el host (el runner de
# Ubuntu) no estan, y el fallo aparecia como "no encuentro libxcb-xkb.so.1"
# dos pasos mas tarde, muy lejos de su causa: libxcb.so.1 si esta en Ubuntu
# por casualidad, asi que una se copiaba y la otra no, y parecian un problema
# de nombres en vez de de sitio.
# Con `bash` explicito: `/bin/sh` en Debian es dash y no conoce `pipefail`,
  # que bundle.sh necesita. Es el mismo motivo por el que el job de CI declara
  # `shell: bash`.
  echo "==> Empaquetando"
  bash /src/scripts/bundle.sh --from portable

  # El bundle se escribe como root dentro del contenedor, sobre un volumen
  # montado del host. Ahi queda con permisos de root y despues ningun paso
  # del runner puede moverlo ni borrarlo:
  #   rm: cannot remove 'dist/.../bin/port': Permission denied
  # Se abre el permisos aqui, que es el ultimo momento en que se es root.
  chmod -R a+rwX /src/dist
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
  "$ROOT/scripts/bundle.sh" --from portable
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
  echo "Listo. Bundle en:"
  ls -d dist/port-"$VERSION"-linux-x86_64 2>/dev/null || true
}

main "$@"
