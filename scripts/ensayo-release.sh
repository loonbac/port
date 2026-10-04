#!/usr/bin/env bash
# Ensayo LOCAL de la cadena completa de release, con contenedor real.
# Es lo que evita gastar 30 minutos por iteracion en GitHub.
set -euo pipefail
cd /home/loonbac/Proyectos/port

echo "=== 1. nix build ==="
out="$(nix build .#port --no-link --print-out-paths 2>/dev/null | tail -1)"
echo "   $out"

echo "=== 2. build-portable.sh (contenedor real) ==="
./scripts/build-portable.sh 2>&1 | tail -6

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
echo "=== 3. el bundle ARRANCA? ==="
./"dist/port-$VERSION-linux-x86_64/bin/port" --help | head -2

echo "=== 4. package.sh deb ==="
bash scripts/package.sh deb 2>&1 | tail -2
echo "=== 5. package.sh spec ==="
bash scripts/package.sh spec 2>&1 | tail -2

echo "=== 6. nada se borro por el camino ==="
for f in "port-$VERSION.deb" "port.spec" "port-$VERSION-linux-x86_64/bin/port"; do
  [ -e "dist/$f" ] || { echo "FALLO: falta dist/$f"; exit 1; }
done
echo
echo "dist/:"; ls -1 dist/
echo; echo "ENSAYO LOCAL COMPLETO: OK"
