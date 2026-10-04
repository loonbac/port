#!/usr/bin/env bash
# Empaqueta el binario de PORT con el gestor nativo de una distribución.
#
# Se espera que el binario ya esté construido en `result/bin/port` (ver flake.nix).
# El script solo lo envuelve: no recompila, para que la release y el
# empaquetado no diverjan.
set -euo pipefail

PKG="${1:-}"
# shellcheck source=scripts/version.sh
source "$(dirname "${BASH_SOURCE[0]}")/version.sh"
VERSION="$PORT_VERSION"
BIN="port"
BINARY="result/bin/$BIN"
DIST="dist"
PKG_NAME="port"

die() { echo "error: $*" >&2; exit 1; }

[ -n "$PKG" ] || die "uso: $0 <deb|rpm|arch|appimage>"
[ -x "$BINARY" ] || die "falta $BINARY; ejecuta 'nix build .#port' primero"

mkdir -p "$DIST"
rm -rf "$DIST"/*

# Un `.desktop` para que aparezca en los menús de las distros con GUI.
cat > "$DIST/$BIN.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=PORT
Comment=Plugin-Oriented Rust Terminal
Exec=$BIN
Terminal=false
Categories=System;TerminalEmulator;
DESKTOP

install_desktop() {
  local dest="$1"
  mkdir -p "$dest"
  install -Dm644 "$DIST/$BIN.desktop" "$dest/$BIN.desktop"
}

install_icon() {
  local size="$1" dest="$2"
  mkdir -p "$dest"
  # Un icono mínimo y válido, generado sin depender de fuentes del sistema.
  # Si ImageMagick está disponible se pinta una "P"; si no, un PNG sólido.
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

case "$PKG" in
  deb)
    ROOT="$DIST/deb-root"
    rm -rf "$ROOT"
    mkdir -p "$ROOT/DEBIAN"
    install -Dm755 "$BINARY" "$ROOT/usr/bin/$BIN"
    install_desktop "$ROOT/usr/share/applications"
    mkdir -p "$ROOT/usr/share/icons/hicolor/256x256/apps"
    install_icon 256 "$ROOT/usr/share/icons/hicolor/256x256/apps"
    mkdir -p "$ROOT/usr/share/doc/$PKG_NAME"
    cp README.md "$ROOT/usr/share/doc/$PKG_NAME/README.md"

    cat > "$ROOT/DEBIAN/control" <<CONTROL
Package: $PKG_NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
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

  rpm)
    ROOT="$DIST/rpm-root"
    rm -rf "$ROOT"
    mkdir -p "$ROOT/usr/bin" "$ROOT/usr/share/applications"
    install -Dm755 "$BINARY" "$ROOT/usr/bin/$BIN"
    install_desktop "$ROOT/usr/share/applications"

    SPEC="$DIST/$PKG_NAME.spec"
    cat > "$SPEC" <<SPECFILE
Name:           $PKG_NAME
Version:        $VERSION
Release:        1%{?dist}
Summary:        Plugin-Oriented Rust Terminal
License:        MIT
BuildArch:      x86_64

%description
Terminal emulator written in Rust on GPUI, with a plugin system where any
core behaviour can be replaced without touching the core.

%install
mkdir -p %{buildroot}
cp -a %{_sourcedir}/rpm-root/* %{buildroot}/

%files
/usr/bin/$BIN
/usr/share/applications/$BIN.desktop
SPECFILE

    command -v rpmbuild >/dev/null 2>&1 \
      && rpmbuild -bb --define "_topdir $PWD/$DIST/rpmbuild" "$SPEC" \
      || echo "rpmbuild no disponible; se emite el .spec para empaquetado externo"
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
