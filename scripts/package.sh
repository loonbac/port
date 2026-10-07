#!/usr/bin/env bash
# Empaqueta el binario de PORT con el gestor nativo de una distribución.
#
# Se espera que el binario ya esté construido: `target/portable/bin/port` (el
# que produce scripts/build-portable.sh dentro de un contenedor, contra un glibc
# antiguo) o `result/bin/port` si se construyó con Nix. El script solo lo
# envuelve: no recompila, para que la release y el empaquetado no diverjan.
set -euo pipefail

PKG="${1:-}"
# shellcheck source=scripts/version.sh
source "$(dirname "${BASH_SOURCE[0]}")/version.sh"
VERSION="$PORT_VERSION"
BIN="port"
DIST="dist"
PKG_NAME="port"

die() { echo "error: $*" >&2; exit 1; }

# El binario puede venir de tres sitios. El de `target/portable/` va PRIMERO a
# proposito: es el que compila build-portable.sh dentro de un contenedor contra
# un glibc antiguo, y es el unico correcto para un paquete nativo que debe
# arrancar en Ubuntu 22.04 o Fedora 36. `result/` es el de Nix y lleva el
# interprete del store; solo vale si no hay nada mejor, y las ramas deb/rpm lo
# rechazan con reject_nix_binary.
BINARY=""
for candidate in "target/portable/bin/$BIN" "result/bin/$BIN" "target/release/$BIN"; do
  if [ -x "$candidate" ]; then
    BINARY="$candidate"
    break
  fi
done

# Ultimo recurso: el artefacto descargado puede traer el binario un nivel mas
# abajo segun como se creo. Antes de adivinar rutas, se busca.
if [ -z "$BINARY" ]; then
  BINARY="$(find . -maxdepth 4 -type f -name "$BIN" -perm -u+x 2>/dev/null | head -1)"
fi

[ -n "$PKG" ] || die "uso: $0 <deb|spec|rpm|arch|appimage>"
[ -n "$BINARY" ] || die "no encuentro el binario en target/portable/bin, result/bin ni target/release; construye antes de empaquetar"

# Un binario compilado con Nix lleva grabado el interprete del store
# (/nix/store/.../ld-linux-x86-64.so.2) y solo arranca dentro de Nix. Publicado
# como .deb o .rpm, instala sin error y muere al ejecutar en una maquina limpia.
# Es la misma comprobacion que hacia bundle.sh antes de dar un paquete por
# bueno; el paquete nativo es el unico entregable, asi que ahora vive aqui.
reject_nix_binary() {
  local interp
  interp="$(readelf -l "$BINARY" 2>/dev/null \
    | sed -n 's/.*interpreter: \(.*\)\]/\1/p')"
  case "$interp" in
    /nix/store/*)
      die "el binario viene de Nix (interprete del store: $interp): no sirve para paquetes nativos; compila con scripts/build-portable.sh"
      ;;
  esac
}

# Deteccion de motor de contenedores, igual que scripts/build-portable.sh.
have_docker() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }
have_podman() { command -v podman >/dev/null 2>&1 && podman info >/dev/null 2>&1; }

echo "==> Empaquetando desde $BINARY"

# No se limpia `dist` entero. En el workflow de release varios pasos escriben
# ahi (el binario, los paquetes) y cada invocacion de este script se lleva por
# delante lo que dejaron los anteriores: se construia un .deb y acto seguido
# se borraba. Cada empaquetado usa su propio subdirectorio de trabajo y solo
# sustituye los ficheros que le corresponden.
mkdir -p "$DIST"

# Un `.desktop` para que aparezca en los menús de las distros con GUI.
cat > "$DIST/$BIN.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=PORT
GenericName=Terminal Emulator
Comment=Plugin-Oriented Rust Terminal
Exec=$BIN
Icon=port-terminal
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;prompt;command;
StartupNotify=true
DESKTOP

install_desktop() {
  local dest="$1"
  mkdir -p "$dest"
  install -Dm644 "$DIST/$BIN.desktop" "$dest/$BIN.desktop"
}

install_icon() {
  local size="$1" dest="$2"
  mkdir -p "$dest"

  # El icono del proyecto, si esta en el repositorio. Se copia directamente:
  # ya viene al tamaño que usa hicolor y no hace falta ImageMagick para nada,
  # que es lo que hacia fallar el empaquetado en maquinas sin esas herramientas.
  # ROOT se reutiliza como staging en cada formato, asi que la raiz del
  # repositorio se saca de la ubicacion del propio script y no de ROOT.
  local repo_root
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  local source_icon="$repo_root/assets/icon-256.png"
  if [ -f "$source_icon" ]; then
    cp "$source_icon" "$dest/$BIN.png"
    cp "$source_icon" "$dest/port-terminal.png"
    return 0
  fi

  # Sin icono en el repositorio: se genera uno, antes a mano.
  local magick=""
  command -v magick >/dev/null 2>&1 && magick="magick"
  [ -z "$magick" ] && command -v convert >/dev/null 2>&1 && magick="convert"

  if [ -n "$magick" ]; then
    if $magick -size "${size}x${size}" xc:'#0d1117' \
         -fill '#58a6ff' -gravity center \
         -draw "roundrectangle $((size/6)),$((size/6)) $((size - size/6)),$((size - size/6)) $((size/12)),$((size/12))" \
         "$dest/$BIN.png" 2>/dev/null; then
      return 0
    fi
  fi
  # PNG 1x1 sólido como respaldo: prefiero un icono feo a ninguno.
  printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\x60\00\x00\x00\x02\x00\x01\xe2\x21\xbc\x33\x00\x00\x00\x00IEND\xaeB`\x82' > "$dest/$BIN.png"
}

# Construye el .rpm a partir de `$SPEC` y del arbol `dist/rpm-root`.
#
# rpmbuild no viene ni en los runners de GitHub ni en NixOS, asi que cuando no
# esta en el PATH el build se hace dentro de un contenedor Fedora. La imagen
# `fedora:latest` tampoco trae rpm-build en la base: hay que instalarlo con dnf.
build_rpm() {
  local have_rpmbuild=0
  command -v rpmbuild >/dev/null 2>&1 && have_rpmbuild=1

  # El motor solo hace falta si no hay rpmbuild nativo.
  local engine=""
  if [ "$have_rpmbuild" -eq 0 ]; then
    if have_docker; then
      engine="docker"
    elif have_podman; then
      engine="podman"
    else
      # Sin rpmbuild ni motor de contenedores solo queda el .spec. No es un
      # error: el workflow y el ensayo comprueban ellos mismos que el .rpm
      # exista, y ahi si habra contenedor.
      echo "AVISO: solo se genero el .spec (sin rpmbuild)"
      return 0
    fi
  fi

  # `_topdir` exige que sus subdirectorios existan y que SOURCES/rpm-root este
  # sembrado antes de invocar rpmbuild, en los dos caminos. Se limpia antes para
  # que un .rpm de una ejecucion anterior en RPMS/ no se confunda con el recien
  # construido.
  rm -rf "$DIST/rpmbuild"
  mkdir -p "$DIST/rpmbuild/SOURCES/rpm-root" \
           "$DIST/rpmbuild/SPECS" "$DIST/rpmbuild/BUILD" \
           "$DIST/rpmbuild/BUILDROOT" "$DIST/rpmbuild/RPMS" \
           "$DIST/rpmbuild/SRPMS"
  cp -a "$DIST/rpm-root/." "$DIST/rpmbuild/SOURCES/rpm-root/"
  cp "$SPEC" "$DIST/rpmbuild/SPECS/port.spec"

  if [ "$have_rpmbuild" -eq 1 ]; then
    # Maquina de desarrollo con rpmbuild nativo: se construye directo, como
    # hacia este script antes.
    rpmbuild -bb --define "_topdir $PWD/$DIST/rpmbuild" "$SPEC"
  else
    echo "==> Construyendo el .rpm dentro de fedora:latest con $engine"
    # Nombre completamente cualificado: los nombres cortos no se resuelven en
    # un entorno limpio (podman exige el registro explicito) y docker solo los
    # acepta por costumbre.
    "$engine" run --rm -i \
      -v "$PWD:/src:rw" \
      -w /src \
      docker.io/library/fedora:latest \
      sh -s <<'DOCKER'
set -eu
# rpm-build NO viene en la imagen base.
dnf -y install rpm-build
rpmbuild -bb --define "_topdir /src/dist/rpmbuild" /src/dist/port.spec
# rpmbuild corre como root dentro del contenedor sobre el bind mount: los
# ficheros quedan root-owned y despues el host no puede ni moverlos ni
# borrarlos. Se abren permisos aqui, que es el ultimo momento en que se es root.
chmod -R a+rwX /src/dist/rpmbuild
DOCKER
  fi

  # rpmbuild nombra el fichero segun el spec; se normaliza a un nombre estable
  # porque el workflow de publicacion espera exactamente
  # dist/port-<version>.rpm.
  local built
  built="$(find "$DIST/rpmbuild/RPMS" -type f -name '*.rpm' | head -1)"
  [ -n "$built" ] || die "rpmbuild no produjo ningun .rpm en $DIST/rpmbuild/RPMS"
  cp "$built" "$DIST/$PKG_NAME-$VERSION.rpm"
  chmod a+rw "$DIST/$PKG_NAME-$VERSION.rpm"
  echo "construido $DIST/$PKG_NAME-$VERSION.rpm"
}

case "$PKG" in
  deb)
    reject_nix_binary
    ROOT="$DIST/deb-root"
    rm -rf "$ROOT"
    mkdir -p "$ROOT/DEBIAN"
    install -Dm755 "$BINARY" "$ROOT/usr/bin/$BIN"
    install_desktop "$ROOT/usr/share/applications"
    mkdir -p "$ROOT/usr/share/icons/hicolor/256x256/apps"
    install_icon 256 "$ROOT/usr/share/icons/hicolor/256x256/apps"
    mkdir -p "$ROOT/usr/share/doc/$PKG_NAME"
    cp README.md "$ROOT/usr/share/doc/$PKG_NAME/README.md"

    # dpkg no calcula `Depends:` a partir de los ELF cuando el .deb se arma a
    # mano con `ar` (ese camino no pasa por dpkg-shlibdeps), asi que la lista va
    # explicita. Incluye lo que enlaza el binario (libxcb, libxkbcommon) y lo
    # que GPUI abre con dlopen (Wayland, Vulkan, FreeType, fontconfig). No se
    # listan libc6 ni libgcc-s1: son de prioridad required y estan siempre. Lo
    # que prueba esta lista de verdad es scripts/test-install.sh, que instala el
    # .deb en Debian 12 y Ubuntu 22.04 limpios y comprueba que `ldd` no deja
    # ningun 'not found'.
    cat > "$ROOT/DEBIAN/control" <<CONTROL
Package: $PKG_NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
Depends: libxcb1, libxcb-xkb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libfreetype6, libfontconfig1
Maintainer: loonbac <loonbac@users.noreply.github.com>
Description: Plugin-Oriented Rust Terminal
 Terminal emulator written in Rust on GPUI, with a plugin system where any
 core behaviour can be replaced without touching the core.
CONTROL

    # dpkg-deb no viene en NixOS; se construye el .deb a mano, que es un `ar`
    # con tres entradas: control.tar.gz, data.tar.gz y debian-binary.
    if command -v dpkg-deb >/dev/null 2>&1; then
      dpkg-deb --build "$ROOT" "$DIST/$PKG_NAME-$VERSION.deb"
    elif command -v ar >/dev/null 2>&1 && command -v tar >/dev/null 2>&1; then
      WORK="$DIST/.deb-build"
      rm -rf "$WORK"; mkdir -p "$WORK/control" "$WORK/data"
      cp -a "$ROOT/DEBIAN/." "$WORK/control/"
      cp -a "$ROOT/." "$WORK/data/"
      # Las compresiones que Debian espera: control en gzip, data en xz.
      tar -C "$WORK/control" -czf "$WORK/control.tar.gz" .
      tar -C "$WORK/data"   -cJf "$WORK/data.tar.xz"   .
      printf '2.0\n' > "$WORK/debian-binary"
      DEB_ABS="$(cd "$DIST" && pwd)/$PKG_NAME-$VERSION.deb"
      ( cd "$WORK" && ar rc "$DEB_ABS" debian-binary control.tar.gz data.tar.xz )
      rm -rf "$WORK"
      echo "construido $DIST/$PKG_NAME-$VERSION.deb"
    else
      echo "sin dpkg-deb ni ar: se omite el .deb"
    fi
    ;;

  # `spec` genera el .spec de RPM sin construirlo. El workflow lo pide por
  # separado porque los runners de GitHub no traen `rpmbuild`, igual que no
  # traen `dpkg-deb`. Antes la matriz pedia `spec` y el script no lo tenia:
  # el job moria con "paquete desconocido: spec".
  spec)
    SPEC="$DIST/$PKG_NAME.spec"
    cat > "$SPEC" <<SPECFILE
Name:           port
Version:        $VERSION
Release:        1%{?dist}
Summary:        PORT: Plugin-Oriented Rust Terminal
License:        MIT
URL:            https://github.com/loonbac/port

%description
Terminal con GPUI y sistema de plugins.

%files
%license LICENSE
/usr/bin/$BIN
/usr/share/applications/$PKG_NAME.desktop

%changelog
* Thu Jan 01 1970 loonbac - $VERSION-1
- Initial package
SPECFILE
    echo "construido $SPEC"
    ;;

  rpm)
    reject_nix_binary
    ROOT="$DIST/rpm-root"
    rm -rf "$ROOT"
    mkdir -p "$ROOT/usr/bin" "$ROOT/usr/share/applications"
    install -Dm755 "$BINARY" "$ROOT/usr/bin/$BIN"
    install_desktop "$ROOT/usr/share/applications"
    # Mismos iconos que el .deb. Van en las dos rutas que usan las distros
    # (hicolor y pixmaps) y despues se declaran en `%files`: rpmbuild falla si
    # el buildroot trae ficheros que el spec no empaqueta, y eso es a proposito.
    mkdir -p "$ROOT/usr/share/icons/hicolor/256x256/apps" "$ROOT/usr/share/pixmaps"
    install_icon 256 "$ROOT/usr/share/icons/hicolor/256x256/apps"
    install_icon 256 "$ROOT/usr/share/pixmaps"

    SPEC="$DIST/$PKG_NAME.spec"
    cat > "$SPEC" <<SPECFILE
Name:           $PKG_NAME
Version:        $VERSION
Release:        1%{?dist}
Summary:        Plugin-Oriented Rust Terminal
License:        MIT
BuildArch:      x86_64

# Las librerias enlazadas (xcb, xkbcommon) las añade sola el generador de
# dependencias ELF de rpmbuild. Las que GPUI abre con dlopen no dejan rastro en
# el ELF, asi que van explicitas por capacidad de soname: el nombre del paquete
# cambia entre distros, el soname no.
Requires:       libwayland-client.so.0()(64bit)
Requires:       libvulkan.so.1()(64bit)
Requires:       libfreetype.so.6()(64bit)
Requires:       libfontconfig.so.1()(64bit)

%description
Terminal emulator written in Rust on GPUI, with a plugin system where any
core behaviour can be replaced without touching the core.

%install
mkdir -p %{buildroot}
cp -a %{_sourcedir}/rpm-root/* %{buildroot}/

%files
/usr/bin/$BIN
/usr/share/applications/$BIN.desktop
/usr/share/icons/hicolor/256x256/apps/$BIN.png
/usr/share/icons/hicolor/256x256/apps/port-terminal.png
/usr/share/pixmaps/$BIN.png
/usr/share/pixmaps/port-terminal.png
SPECFILE

    build_rpm
    ;;

  arch)
    command -v makepkg >/dev/null 2>&1 || die "makepkg no disponible"
    PKGDIR="$DIST/pkgbuild"
    rm -rf "$PKGDIR"; mkdir -p "$PKGDIR"
    install -Dm755 "$BINARY" "$PKGDIR/$BIN"
    install_desktop "$PKGDIR/usr/share/applications"
    mkdir -p "$PKGDIR/usr/share/icons/hicolor/256x256/apps"
    install_icon 256 "$PKGDIR/usr/share/icons/hicolor/256x256/apps"

    cat > "$PKGDIR/PKGBUILD" <<PKGBUILD
pkgname=port
pkgver=$VERSION
pkgrel=1
pkgdesc="Plugin-Oriented Rust Terminal"
arch=('x86_64')
license=('MIT')
package() {
  cd "\$srcdir/$PKGDIR"
  install -Dm755 "$BIN" "\$pkgdir/usr/bin/$BIN"
  install -Dm644 "$BIN.desktop" "\$pkgdir/usr/share/applications/$BIN.desktop"
}
PKGBUILD
    (cd "$PKGDIR" && PKGDEST="$PWD" makepkg --nodeps --noconfirm)
    ;;

  appimage)
    command -v linuxdeploy >/dev/null 2>&1 \
      || die "linuxdeploy no disponible; instala appimagetool"
    cp "$BINARY" "$DIST/AppRun"
    chmod +x "$DIST/AppRun"
    install_desktop "$DIST"
    linuxdeploy --appdir="$DIST" --output=appimage "$DIST/AppRun"
    ;;

  *)
    die "paquete desconocido: $PKG"
    ;;
esac

echo "empaquetado en $DIST/:"
ls -la "$DIST" | tail -n +2
