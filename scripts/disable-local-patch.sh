#!/usr/bin/env bash
# Elimina el bloque [patch] local de port-plugins.
#
# El workspace usa parches a rutas locales (`../port-plugins/...`) para
# desarrollar sin publicar. En CI y en releases ese repositorio hermano no está
# presente, así que el parche debe retirarse antes de resolver dependencias.
set -euo pipefail

cd "$(dirname "$0")/.."

python3 - <<'PY'
import pathlib, re
p = pathlib.Path("Cargo.toml")
s = p.read_text()
# Borra el bloque [patch."https://github.com/loonbac/port-plugins.git"] y
# todas sus líneas hasta la siguiente sección de nivel superior.
s = re.sub(
    r'\n\[patch\."https://github\.com/loonbac/port-plugins\.git"\]\n(?:(?!\[).*\n)*',
    '\n',
    s,
)
p.write_text(s)
print("patch local retirado")
PY
