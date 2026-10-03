#!/usr/bin/env bash
# Prepara el workspace para una build fuera del entorno de desarrollo local.
#
# Dos cosas hace:
#
#  1. `Cargo.toml` trae un bloque `[patch]` que apunta a rutas locales
#     (`../port-plugins/...` y `./crates/...`). Ese repositorio hermano no está
#     en el runner ni en un `nix build` de un paquete publicado, así que el
#     parche se retira y las dependencias se resuelven desde el tag publicado.
#
#  2. `Cargo.lock` tiene entradas `path+file://` para esos mismos paquetes. Con
#     el parche retirado, cargo no puede resolverlas y `--locked` falla, así que
#     el lock se regenera contra el tag.
#
# Escribido en shell POSIX a propósito: las imágenes base de los contenedores
# de CI no traen python3, y este script corre antes de instalar nada.
#
# El cambio es local al runner: nunca se commitea.
set -euo pipefail

cd "$(dirname "$0")/.."

MANIFEST="Cargo.toml"

# Solo se retira el parche de port-plugins. El de port.git se conserva a
# proposito: los plugins llegan como dependencias git y deben enlazar contra el
# API y el core de ESTE repo. Sin el, `Plugin` se resuelve a dos crates
# distintos y el registro no compila.
if ! grep -q 'patch\."https://github.com/loonbac/port-plugins.git"' "$MANIFEST"; then
  echo "no hay parche local de port-plugins que retirar"
else
  # awk imprime todo lo que NO pertenece al bloque del parche retirado.
  tmp="$(mktemp)"
  awk '
    /^\[patch\."https:\/\/github\.com\/loonbac\/port-plugins\.git"\]/ { skipping = 1; next }
    skipping && /^\[/       { skipping = 0 }
    !skipping               { print }
  ' "$MANIFEST" > "$tmp"

  # Colapsa las líneas en blanco que quedaron al disappear el bloque.
  awk '
    /^[[:space:]]*$/ { if (blank) next }
    { blank = ($0 ~ /^[[:space:]]*$/) }
    { print }
  ' "$tmp" > "$MANIFEST"

  rm -f "$tmp"
  echo "parches locales retirados"
fi

# El lock se regenera porque sus entradas locales de port-plugins ya no son
# resolubles una vez retirado el parche.
if [ -f Cargo.lock ]; then
  rm -f Cargo.lock
  echo "lock eliminado: se regenera contra el tag publicado"
fi
