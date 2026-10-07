#!/usr/bin/env bash
# Prueba de instalacion de los paquetes nativos en un sistema limpio.
#
# Este es el porton de la release: instala el .deb/.rpm dentro de un contenedor
# de la distribucion destino y comprueba que la terminal queda instalada como
# cualquier otra (binario en /usr/bin, .desktop e icono en las rutas del
# sistema) y que arranca. Sustituye a las viejas comprobaciones del bundle
# portable, que ya no existe.
#
# La asercion 5 es la que importa: con LD_LIBRARY_PATH vacio, `ldd` no debe
# dejar ninguna libreria "not found". Es lo que demuestra que la lista
# `Depends:` del .deb y los `Requires:` del .rpm son correctos. Un paquete sin
# esas dependencias se instala sin error y solo se rompe al ejecutar.
#
# Uso: ./scripts/test-install.sh deb|rpm
set -euo pipefail

WHICH="${1:-}"
[ "$WHICH" = "deb" ] || [ "$WHICH" = "rpm" ] || {
  echo "uso: $0 deb|rpm" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/version.sh
source "$ROOT/scripts/version.sh"
VERSION="$PORT_VERSION"
DIST="$ROOT/dist"

have_docker() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }
have_podman() { command -v podman >/dev/null 2>&1 && podman info >/dev/null 2>&1; }

# Deteccion de motor igual que build-portable.sh. En esta maquina la sesion de
# login es anterior a la incorporacion al grupo docker, asi que un shell normal
# no tiene acceso al socket; si docker existe pero `docker info` falla, el
# mensaje lo explica antes de fallar de forma opaca.
if have_docker; then
  ENGINE="docker"
elif have_podman; then
  ENGINE="podman"
else
  echo "error: no hay motor de contenedores utilizable (docker o podman)" >&2
  if command -v docker >/dev/null 2>&1; then
    echo "       docker existe pero 'docker info' falla; ejecuta con" >&2
    echo "       sg docker -c '$0 $WHICH' o unete al grupo docker" >&2
  fi
  exit 1
fi

# Instala el paquete y ejecuta las cinco aserciones dentro de la imagen.
#
# El comando de instalacion se inyecta como variable de entorno porque cambia
# por distribucion (apt en Debian/Ubuntu, dnf en Fedora); las aserciones, que
# son la parte que importa, son las mismas para todas.
run_in_image() {
  local image="$1" install_cmd="$2"
  echo "==> $image"
  "$ENGINE" run --rm -i \
    -e INSTALL_CMD="$install_cmd" \
    -v "$DIST:/pkg:ro" \
    "$image" \
    sh -s <<'DOCKER'
set -eu
export DEBIAN_FRONTEND=noninteractive

echo "--- instalando: $INSTALL_CMD"
sh -c "$INSTALL_CMD"

echo "[1/5] binario presente"
command -v port >/dev/null || { echo "FALLO: 'port' no esta en el PATH" >&2; exit 1; }

echo "[2/5] port --help"
# La salida se captura sin pipe: `set -e` no ve el codigo de `port` si va
# encadenado a `head`.
if ! out="$(port --help 2>&1)"; then
  echo "FALLO: 'port --help' termino con error:" >&2
  echo "$out" >&2
  exit 1
fi
echo "$out" | head -3

echo "[3/5] entrada .desktop"
test -f /usr/share/applications/port.desktop \
  || { echo "FALLO: falta /usr/share/applications/port.desktop" >&2; exit 1; }

echo "[4/5] icono"
test -f /usr/share/icons/hicolor/256x256/apps/port-terminal.png \
  || { echo "FALLO: falta el icono en hicolor" >&2; exit 1; }

echo "[5/5] ldd sin librerias 'not found'"
libs="$(LD_LIBRARY_PATH= ldd /usr/bin/port 2>&1 || true)"
if echo "$libs" | grep -q "not found"; then
  echo "FALLO: el binario no resuelve estas librerias (revisa Depends/Requires):" >&2
  echo "$libs" | grep "not found" >&2
  exit 1
fi

echo "OK: port instalado, arranca y resuelve sus librerias"
DOCKER
}

case "$WHICH" in
  deb)
    # Debian 12 es el suelo de glibc y Ubuntu 22.04 el objetivo declarado en el
    # README; se prueban los dos.
    run_in_image "docker.io/library/debian:12-slim" \
      "apt-get update -qq && apt-get install -y -qq /pkg/port-$VERSION.deb"
    run_in_image "docker.io/library/ubuntu:22.04" \
      "apt-get update -qq && apt-get install -y -qq /pkg/port-$VERSION.deb"
    ;;
  rpm)
    run_in_image "docker.io/library/fedora:latest" \
      "dnf install -y /pkg/port-$VERSION.rpm"
    ;;
esac

echo
echo "INSTALACION OK: port-$VERSION.$WHICH"
