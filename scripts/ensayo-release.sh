#!/usr/bin/env bash
# Ensayo LOCAL de la cadena completa de release, con contenedores reales.
#
# Replica lo que hace `.github/workflows/release.yml` pero en esta maquina: es
# lo que evita gastar 30 minutos por iteracion en GitHub. Compila el binario de
# release, genera el .deb y el .rpm y comprueba que se instalan limpios en
# Debian 12, Ubuntu 22.04 y Fedora.
set -euo pipefail

SELF="$(readlink -f "$0")"

docker_ok() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }

# La sesion de login de esta maquina es anterior a la incorporacion al grupo
# docker, asi que los shells normales no tienen acceso al socket. Si `docker
# info` falla pero `sg docker -c 'docker info'` funciona, se re-lanza el script
# entero bajo `sg docker`. Sin esto, cada paso moria en el primer `docker run`
# con "permission denied ... docker.sock".
if ! docker_ok; then
  if command -v sg >/dev/null 2>&1 && sg docker -c 'docker info' >/dev/null 2>&1; then
    echo "==> docker sin acceso directo; re-lanzando bajo 'sg docker'"
    exec sg docker -c "bash $(printf '%q' "$SELF")"
  fi
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/version.sh
source "$ROOT/scripts/version.sh"
VERSION="$PORT_VERSION"

echo "=== 1. build-portable.sh (binario de release, contenedor real) ==="
./scripts/build-portable.sh

echo "=== 2. package.sh deb ==="
./scripts/package.sh deb

echo "=== 3. package.sh rpm ==="
./scripts/package.sh rpm

echo "=== 4. test-install.sh deb (Debian 12 + Ubuntu 22.04) ==="
./scripts/test-install.sh deb

echo "=== 5. test-install.sh rpm (Fedora) ==="
./scripts/test-install.sh rpm

echo "=== 6. los paquetes existen con el nombre versionado ==="
for f in "dist/port-$VERSION.deb" "dist/port-$VERSION.rpm"; do
  [ -e "$f" ] || { echo "FALLO: falta $f"; exit 1; }
done

echo
echo "dist/:"; ls -1 dist/
echo; echo "ENSAYO LOCAL COMPLETO: OK"
