#!/usr/bin/env bash
# Empaquetado portable de PORT: un directorio autocontenido que no necesita Nix.
#
# La razón de que esto exista: GPUI enlaza contra xcb y xkbcommon en tiempo de
# compilacion, y ademas carga Wayland, Vulkan y freetype por `dlopen` cuando
# arranca. Un binario pelado depende de que la distribucion tenga esas librerias
# con el mismo nombre de soname, y en la practica eso no se cumple de forma
# uniforme (Fedora y Arch versiones distintas de libxkbcommon, Alpine musl, etc).
#
# La solucion es empaquetarlas al lado del binario y apuntar el RUNPATH a
# `$ORIGIN/lib`, que es donde caen las dos cuando el usuario mueve el directorio.
# El driver de Vulkan (el ICD concreto de la GPU) SI tiene que estar en el
# sistema: es lo queconnects la GPU, y no tiene sentido empaquetarlo.
set -euo pipefail

VERSION="0.1.0"
BIN="port"
DIST="dist"
BUNDLE="$DIST/$BIN-$VERSION-linux-x86_64"

die() { echo "error: $*" >&2; exit 1; }

# Librerias que GPUI abre con dlopen en lugar de enlazarlas. No aparecen en
# `ldd`, asi que no se pueden deducir: si faltan, es un problema de verdad.
DLOPEN_LIBS="libwayland-client.so.0 libvulkan.so.1"

is_dlopen_lib() {
  case " $DLOPEN_LIBS " in *" $1 "*) return 0 ;; *) return 1 ;; esac
}

# `true` si el binario enlaza (o el linker cargara) contra ese soname.
binary_links() {
  ldd "$BUNDLE/bin/$BIN" 2>/dev/null | grep -q "$1"
}

# Localiza el binario: se acepta tanto el de `cargo build --release` como el
# que produce la flake de Nix.
resolve_binary() {
  for candidate in target/release/port result/bin/port; do
    if [ -x "$candidate" ]; then
      echo "$candidate"
      return 0
    fi
  done
  die "no encuentro el binario; compila con 'cargo build --release' o 'nix build'"
}

# Las librerias que el binario necesita. Se resuelven por nombre de soname,
# no por ruta, para que funcione tanto en NixOS como en una distro normal.
REQUIRED_LIBS=(
  libxcb.so.1
  libxcb-xkb.so.1
  libxkbcommon.so.0
  libxkbcommon-x11.so.0
  libwayland-client.so.0
  libvulkan.so.1
)

# Dependencias transitivas que las anteriores arrastran consigo. Sin ellas,
# `ldd` las encuentra en el sistema y el bundle no es portable de verdad.
BUNDLED_EXTRA_LIBS=(
  libxau.so.6
  libxdmcp.so.6
  libbsd.so.0
  libmd.so.0
  libffi.so.8
  libwayland-server.so.0
)

# Rutas donde buscar. Se añade LIBRARY_PATH porque en NixOS las librerias estan
# en el store y no en /usr/lib, y se ahí donde se resuelve el resto del build.
search_dirs() {
  echo "/usr/lib/$(uname -m)-linux-gnu"
  echo "/usr/lib64"
  echo "/usr/lib"
  echo "/lib/$(uname -m)-linux-gnu"
  echo "/lib64"
  echo "/lib"
  echo "${LIBRARY_PATH:-}" | tr ':' '\n'
}

find_lib() {
  local name="$1" dir dirs
  # La lista se calcula ANTES de buscar. Con sustitucion de procesos, el
  # `return 0` del acierto cerraba el pipe mientras `search_dirs` seguia
  # escribiendo: aparecian errores de "Broken pipe" y la busquedadel resto de
  # librerias se quedaba sin directorios, de ahi el "no encuentro
  # libxcb-xkb.so.1" aunque la libreria estuviese instalada.
  dirs="$(search_dirs)"
  while read -r dir; do
    [ -n "$dir" ] || continue
    if [ -e "$dir/$name" ]; then
      echo "$dir/$name"
      return 0
    fi
  done <<< "$dirs"
  return 1
}

main() {
  local binary
  if [ "${1:-}" = "--from" ] && [ "${2:-}" = "portable" ]; then
    if [ ! -x target/portable/bin/port ]; then
      die "no encuentro target/portable/bin/port"
    fi
    binary="target/portable/bin/port"
  else
    binary="$(resolve_binary)"
  fi

  echo "==> Empaquetando desde $binary"

  rm -rf "$BUNDLE"
  mkdir -p "$BUNDLE/bin" "$BUNDLE/lib" "$BUNDLE/share/applications" \
           "$BUNDLE/share/doc/port"

  cp "$binary" "$BUNDLE/bin/$BIN"
  chmod +x "$BUNDLE/bin/$BIN"

  echo "==> Copiando librerias necesarias a lib/"
  local name src
  for name in "${REQUIRED_LIBS[@]}"; do
    if src="$(find_lib "$name")"; then
      cp -L "$src" "$BUNDLE/lib/$name"
      echo "    $name"
      continue
    fi

    # No esta en el sistema. Antes aqui se moria siempre, y con ella caia la
    # construccion entera por una libreria que el binario quiza ni usa: la
    # lista de sonames es una suposicion sobre lo que hace falta. La verdad
    # la dice `ldd`.
    #
    # Salvedad: GPUI abre Wayland, Vulkan y FreeType con dlopen, y eso NO
    # aparece en `ldd`. Para esas la ausencia si es un problema real, asi que
    # se siguen exigiendo siempre.
    if is_dlopen_lib "$name"; then
      die "no encuentro $name y GPUI la abre con dlopen; instala el paquete que lo provee"
    fi
    if binary_links "$name"; then
      die "el binario enlaza contra $name pero no esta en el sistema"
    fi
    echo "    (omitida) $name: el binario no la necesita"
  done

  echo "==> Copiando dependencias transitivas"
  for name in "${BUNDLED_EXTRA_LIBS[@]}"; do
    if src="$(find_lib "$name")"; then
      cp -L "$src" "$BUNDLE/lib/$name"
    else
      echo "    (opcional) $name no encontrada, se omite"
    fi
  done

  echo "==> Fijando RUNPATH a \$ORIGIN/../lib"
  if command -v patchelf >/dev/null 2>&1; then
    patchelf --set-rpath '$ORIGIN/../lib' "$BUNDLE/bin/$BIN"
  else
    die "patchelf es necesario para fijar el RUNPATH (paquete: patchelf)"
  fi

  echo "==> Comprobando el intérprete dinámico"
  local interp
  interp="$(readelf -l "$BUNDLE/bin/$BIN" 2>/dev/null \
    | sed -n 's/.*interpreter: \(.*\)\]/\1/p')"
  case "$interp" in
    /nix/store/*)
      echo "   ATENCION: el binario fue construido con Nix y lleva el"
      echo "   intérprete del store grabado ($interp)."
      echo "   Ese binario NO arranca fuera de Nix, por portable que sea el"
      echo "   directorio lib/. Para un binario portable hay que compilarlo"
      echo "   contra el glibc del sistema, que es lo que hace"
      echo "   'scripts/build-portable.sh' en Docker."
      die "usa el bundle generado por build-portable.sh, no este"
      ;;
    *)
      echo "    $interp"
      ;;
  esac

  echo "==> Comprobando que resuelve contra el bundle y no contra el sistema"
  # La comprobacion va con LD_LIBRARY_PATH vacio a proposito: si el entorno
  # del desarrollador apunta al store de Nix, `ldd` mentiria saying que
  # resuelve bien cuando en realidad esta usando las librerias del sistema.
  local resolved
  resolved="$(LD_LIBRARY_PATH= ldd "$BUNDLE/bin/$BIN" 2>/dev/null || true)"

  # ldd imprime rutas ABSOLUTAS. $BUNDLE es relativa ("dist/port-..."), asi que
  # compararlas directamente hacia que ninguna libreria pareciera resolverse
  # desde el bundle aunque lo hiciera: el bundle se construia bien y la
  # comprobacion lo rechazaba por un fallo de comparacion de cadenas.
  local bundle_abs
  bundle_abs="$(cd "$BUNDLE" && pwd)"
  echo "    bundle: $bundle_abs"

  local leaked
  leaked="$(echo "$resolved" | grep -oE '/nix/store|/usr/lib|/lib/[a-z0-9_-]+/' \
    | grep -v "^$bundle_abs" | head -3 || true)"
  if [ -n "$leaked" ]; then
    echo "   AVISO: quedan dependencias del sistema:"
    echo "$resolved" | grep -E "$leaked" | sed 's/^/      /'
    echo "    El bundle funciona, pero no es 100% autocontenido."
  fi
  # Y la prueba que de verdad importa: que las cuatro Criticas resuelvan desde lib/.
  local missing=0
  for name in libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1; do
    if echo "$resolved" | grep -q "$name => $bundle_abs"; then
      echo "    $name -> bundle"
    else
      echo "    $name -> NO resuelve desde el bundle"
      missing=1
    fi
  done
  [ "$missing" -eq 0 ] || die "el bundle no resuelve las librerias criticas"

  cat > "$BUNDLE/share/applications/$BIN.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=PORT
Comment=Plugin-Oriented Rust Terminal
Exec=$BUNDLE/bin/$BIN
Terminal=false
Categories=System;TerminalEmulator;
DESKTOP

  cp README.md "$BUNDLE/share/doc/port/" 2>/dev/null || true

  cat > "$BUNDLE/INSTALL.sh" <<'INSTALL'
#!/usr/bin/env sh
# Instala PORT en ~/.local. El directorio se mueve entero: las librerias van
# en lib/ al lado del binario y el RUNPATH es relativo, asi que mientras el
# arbol no se rompa, funciona.
set -eu
SRC="$(cd "$(dirname "$0")" && pwd)"
DEST="${HOME}/.local/opt/${BIN:-port}"
mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -r "$SRC" "$DEST"
mkdir -p "$HOME/.local/bin"
ln -sf "$DEST/bin/port" "$HOME/.local/bin/port"
echo "PORT instalado. Añade ~/.local/bin al PATH si no lo está."
INSTALL
  chmod +x "$BUNDLE/INSTALL.sh"

  echo
  echo "Bundle listo en: $BUNDLE"
  echo "  $(du -sh "$BUNDLE" | cut -f1) total"
  echo
  echo "Probar sin instalar:"
  echo "  $BUNDLE/bin/$BIN"
  echo
  echo "Instalar:"
  echo "  $BUNDLE/INSTALL.sh"
}

main "$@"
