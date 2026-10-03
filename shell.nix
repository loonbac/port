# Entorno de desarrollo de PORT en NixOS.
#
# Existe por una razón concreta: GPUI enlaza contra xcb y xkbcommon, y además
# carga por `dlopen` la biblioteca de Wayland y el cargador de Vulkan. En NixOS
# nada de eso está en la ruta por defecto, así que una build limpia falla al
# enlazar y, si enlaza, el binario tampoco arranca.
#
# En vez de escribir rutas del nix store en los comandos, se declaran aquí como
# dependencias y de ellas salen los `-L` del enlazador y los `-rpath` que el
# binario necesita en runtime.
#
# Uso:
#   nix-shell --run 'cargo test'
#   nix-shell --run 'cargo build && ./target/debug/port'

{ pkgs ? import <nixpkgs> { } }:

let
  inherit (pkgs) lib;
  # Bibliotecas que hacen falta al compilar y también al ejecutar.
  runtimeLibs = with pkgs; [
    libxcb
    libxkbcommon
    freetype
    wayland
    vulkan-loader
  ];

  libDirs = lib.concatMapStringsSep ":" (package: "${package}/lib") runtimeLibs;

  # Un `-rpath` por directorio: el enlazador no parte la lista por `:`.
  rpathFlags = lib.concatMapStringsSep " " (package:
    "-C link-arg=-Wl,-rpath,${package}/lib") runtimeLibs;
in
pkgs.mkShell {
  name = "port-dev";

  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    pkg-config
  ];

  shellHook = ''
    echo "PORT: shell listo (fonte $(command -v cargo))"

    # El enlazador busca aquí las bibliotecas de desarrollo.
    export LIBRARY_PATH="${libDirs}''${LIBRARY_PATH:+:$LIBRARY_PATH}"

    # Y el binario las busca a sí mismo en runtime, que es como las encuentra
    # `dlopen` de GPUI para Wayland y Vulkan.
    export RUSTFLAGS="${rpathFlags} ''${RUSTFLAGS:+ $RUSTFLAGS}"
  '';
}