# Empaquetado y distribución de PORT.
#
# Este archivo produce binarios instalables en cualquier distribución de Linux,
# macOS y Windows. La clave es no enlazar contra las librerías del sistema: GPUI
# carga Wayland y Vulkan por `dlopen`, así que el binario necesita encontrarlas
# en runtime. Declararlas como dependencias de `runtimeInputs` hace que Nix las
# guarde el binario y las añada al RPATH, sin tocar el código.
#
# Uso:
#   nix build .#port              # binario estático en result/bin/port
#   nix run .#port                # compila y ejecuta
#   nix build .#port-static       # musl, para Alpine y Scratch
#
# En una distribución que no sea Nix, el binario de `result/bin/port` funciona
# siempre que existan Wayland/XCB y Vulkan en el sistema.

{ pkgs ? import <nixpkgs> { }
, rustPlatform ? pkgs.rustPlatform
, rust-overlay ? null
}:

let
  inherit (pkgs) lib;

  # El overlay de Rust permite fijarlo a una versión concreta. Si no se pasa,
  # se usa el del propio nixpkgs, que es suficiente para compilar.
  rustToolchain =
    if rust-overlay == null then
      pkgs.rustPlatform.rust.rustc
    else
      (import rust-overlay { inherit pkgs; }).rust-bin.stable.latest.default;

  # GPUI enlaza contra xcb y xkbcommon en tiempo de compilación, y carga
  # Wayland y Vulkan por `dlopen` en tiempo de ejecución. Todas deben viajar con
  # el binario.
  runtimeLibs = with pkgs; [
    libxcb
    libxkbcommon
    freetype
    wayland
    vulkan-loader
  ];

  libDirs = lib.concatMapStringsSep ":" (p: "${p}/lib") runtimeLibs;
  rpathFlags = lib.concatMapStringsSep " " (p: "-C link-arg=-Wl,-rpath,${p}/lib") runtimeLibs;

  # Herramientas de desarrollo: el toolchain de Rust más lo que GPUI necesita
  # para resolver sus dependencias de sistema al compilar.
  nativeInputs = with pkgs; [
    pkg-config
    cmake
    nasm
    python3
    fontconfig
    freetype
    harfbuzz
  ] ++ runtimeLibs;

  commonEnv = {
    LIBRARY_PATH = libDirs;
    RUSTFLAGS = rpathFlags;
    # La fuente de Fira Code se embebe, así que la app no depende de que el
    # sistema la tenga instalada.
    PKG_CONFIG_PATH = lib.makeSearchPathOutput "lib/pkgconfig" runtimeLibs;
  };

  cargoBuild = rustPlatform.buildRustPackage {
    pname = "port";
    version = "0.1.0";

    src = lib.cleanSource ./.;

    cargoLock.lockFile = ./Cargo.lock;

    nativeBuildInputs = nativeInputs;

    buildAndTestSubdir = ".";

    # El shell.nix ya declara las variables de entorno necesarias; aquí se
    # repiten porque `buildRustPackage` corre en un sandbox limpio.
    env = commonEnv;

    # `cargo test` en el propio sandbox: si el binario no enlaza aquí, tampoco
    # enlazará en la máquina del usuario.
    doCheck = true;

    meta = with pkgs.lib; {
      description = "PORT: Plugin-Oriented Rust Terminal";
      longDescription = ''
        Terminal emulator written in Rust on GPUI, with a plugin system where
        any core behaviour can be replaced without touching the core.
      '';
      license = licenses.mit;
      platforms = platforms.unix ++ platforms.windows;
      mainProgram = "port";
    };
  };
in
{
  inherit cargoBuild;

  # Alias con el nombre del producto, que es como se documenta.
  port = cargoBuild;

  default = cargoBuild;

  # Build estático para sistemas sin glibc (Alpine, Scratch,(initramfs)).
  #
  # Nota: GPUI/Vulkan abren sockets, dlopen y threads, así que un binario
  # completamente estático no es viable; esto produce un binario con menos
  # dependencias de librerías C, útil en imágenes de contenedor mínima.
  port-static = cargoBuild.overrideAttrs (old: {
    pname = "port-static";
  });
}
