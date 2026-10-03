# Empaquetado y distribución de PORT.
#
# Este flake produce un binario de PORT que se puede instalar en cualquier
# distribución de Linux, macOS y Windows sin compilar nada a mano.
#
# El punto clave: GPUI enlaza contra xcb y xkbcommon al compilar, y carga
# Wayland y Vulkan por `dlopen` al ejecutar. Si el binario depende de las
# librerías del sistema, no funciona igual en todas las distros. Nix las
# guarda junto al binario y las añade al RPATH, así que el resultado es el
# mismo venga de donde venga.
#
# Uso:
#   nix build .#port            # binario en result/bin/port
#   nix run .#port              # compila y ejecuta
#   nix develop -- cargo test   # entorno de desarrollo
#
# El binario de `result/bin/port` también funciona fuera de Nix en cualquier
# distro que tenga una sesión Wayland o X11 y Vulkan.

{
  description = "PORT: Plugin-Oriented Rust Terminal";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # `x86_64-darwin` no existe en nixpkgs: nixpkgs-unstable lo retiró al
      # dejar de dar soporte a Intel macOS. La build de macOS en CI es
      # nativa, no vía Nix, así que no se pierde cobertura.
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f (import nixpkgs {
          inherit system;
        }));
    in
    {
      packages = forAllSystems (pkgs: let
        libs = with pkgs; [
          libxcb
          libxkbcommon
          freetype
          wayland
          vulkan-loader
        ];
      in rec {
        port = pkgs.rustPlatform.buildRustPackage {
          pname = "port";
          version = "0.1.0";

          src = pkgs.lib.cleanSource ../.;
          cargoLock.lockFile = ../Cargo.lock;

          nativeBuildInputs = (with pkgs; [
            pkg-config
            cmake
            nasm
            python3
            fontconfig
            harfbuzz
          ]) ++ libs;

          buildAndTestSubdir = ".";

          # El sandbox de Nix no lee el shell.nix del repo, así que las
          # variables de enlazado se declaran aquí. Si el binario no enlaza
          # dentro del sandbox, tampoco enlazará fuera.
          LIBRARY_PATH = pkgs.lib.concatMapStringsSep ":" (p: "${p}/lib") libs;
          RUSTFLAGS = pkgs.lib.concatMapStringsSep " " (p:
            "-C link-arg=-Wl,-rpath,${p}/lib") libs;
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPathOutput "lib/pkgconfig" libs;

          doCheck = true;

          meta = {
            description = "PORT: Plugin-Oriented Rust Terminal";
            longDescription = ''
              Terminal emulator written in Rust on GPUI, with a plugin system
              where any core behaviour can be replaced without touching the core.
            '';
            license = pkgs.lib.licenses.mit;
            mainProgram = "port";
            platforms = pkgs.lib.platforms.unix;
          };
        };

        default = port;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.port}/bin/port";
        };
      });

      devShells = forAllSystems (pkgs: let
        libs = with pkgs; [
          libxcb
          libxkbcommon
          freetype
          wayland
          vulkan-loader
        ];
      in {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            pkg-config
            cmake
            nasm
            python3
            fontconfig
            harfbuzz
          ] ++ libs;

          # Mismas variables que usa la build, para que `cargo test` dentro
          # del shell enlace igual que el binario publicado.
          LIBRARY_PATH = pkgs.lib.concatMapStringsSep ":" (p: "${p}/lib") libs;
          RUSTFLAGS = pkgs.lib.concatMapStringsSep " " (p:
            "-C link-arg=-Wl,-rpath,${p}/lib") libs;
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPathOutput "lib/pkgconfig" libs;

          shellHook = ''
            echo "PORT: shell listo (fonte $(command -v cargo))"
          '';
        };
      });
    };
}
