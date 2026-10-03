#!/usr/bin/env bash
# Prepara el workspace para una build fuera del entorno de desarrollo local.
#
# Dos cosas hace:
#
#  1. `Cargo.toml` trae un bloque `[patch]` que apunta a rutas locales
#     (`../port-plugins/...` y `./crates/...`). Ese repositorio hermano no está
#     en el runner ni en un `nix build` de un paquete publicado, así que el
#     parche se retira y las dependencias se resuelven desde GitHub.
#
#  2. `Cargo.lock` tiene entradas `path+file://` para esos mismos paquetes. Con
#     el parche retirado, cargo no puede resolverlas y `--locked` falla, así que
#     el lock se regenera contra las versiones publicadas.
#
# El cambio es local al runner: nunca se commitea.
set -euo pipefail

cd "$(dirname "$0")/.."

python3 - <<'PY'
import pathlib
import re

# 1. Retira los bloques [patch] locales de Cargo.toml.
p = pathlib.Path("Cargo.toml")
s = p.read_text()
for url in ("port-plugins.git", "port.git"):
    s = re.sub(
        r'\n# [^\n]*\n(?:# [^\n]*\n)*\[patch\."https://github\.com/loonbac/'
        + re.escape(url)
        + r'"\]\n(?:(?!\[).*\n)*',
        "\n",
        s,
    )
    s = re.sub(
        r'\n\[patch\."https://github\.com/loonbac/'
        + re.escape(url)
        + r'"\]\n(?:(?!\[).*\n)*',
        "\n",
        s,
    )
p.write_text(s)

# 2. Regenera el lock: las entradas locales dejan de ser resolubles.
lock = pathlib.Path("Cargo.lock")
if lock.exists():
    lock.unlink()
PY

echo "listo: parches locales retirados, lock regenerado"
