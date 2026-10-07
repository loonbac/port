#!/usr/bin/env bash
# Fuente unica de la version del proyecto.
#
# Antes la version estaba escrita a mano en Cargo.toml, build-portable.sh,
# package.sh, el ya retirado bundle.sh y dos scripts de ejemplo. Con eso,
# publicar un tag v0.1.2 generaba un artefacto llamado port-0.1.0: la release y
# el fichero que descarga el usuario no coincidian.
#
# Aqui la version sale de una sola parte, que ademas es la que ya usa cargo, de
# modo que no puede haber dos verdades.
#
# Uso:  source scripts/version.sh ; $PORT_VERSION
PORT_VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" | head -1)"
: "${PORT_VERSION:?no se pudo leer la version de Cargo.toml}"
