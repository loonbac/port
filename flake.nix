# Empaquetado y distribución de PORT.
#
# Este flake produce un binario de PORT instalable en cualquier distribución de
# Linux y macOS, sin compilar a mano.
#
# El punto clave: GPUI enlaza contra xcb y xkbcommon al compilar, y carga
# Wayland y Vulkan por `dlopen` al ejecutar. Si el binario depende de las
# librerías del sistema, no funciona igual en todas las distros. Nix las
# guarda en el store y las mete en el RPATH, así que el resultado es el mismo
# venga de donde venga.
#
# Uso:
#   nix build .#port           # binario en result/bin/port
#   nix run .#port             # compila y ejecuta
#   nix develop -- cargo test  # entorno de desarrollo
#
# El binario de `result/bin/port` también funciona fuera de Nix en cualquier
# distro que tenga una sesión Wayland o X11 y Vulkan.

{
  description = "PORT: Plugin-Oriented Rust Terminal";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      lib = (import nixpkgs { system = "x86_64-linux"; }).lib;

      # `x86_64-darwin` no esta en nixpkgs: lo retiraron al dejar de dar soporte
      # a Intel macOS. La cobertura de macOS la da el job nativo del CI.
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = f:
        lib.genAttrs systems (system: f (import nixpkgs {
          inherit system;
        }));
    in
    {
      packages = forAllSystems (pkgs:
        let
          # Enlazadas al compilar y cargadas por dlopen al ejecutar.
          # GPUI enlaza de forma explicita contra `libxkbcommon-x11`. En
          # nixpkgs ese `.so` va DENTRO del paquete `libxkbcommon`, asi que no
          # hay un atributo separado que declarar. En Debian, en cambio, si es
          # un paquete aparte: de ahi que el CI lo instale explicitly.
          runtimeLibs = with pkgs; [
            libxcb
            libxkbcommon
            freetype
            wayland
            vulkan-loader
          ];

          # concatStringsSep trabaja con strings, no con derivaciones: de ahi
          # el `toString` en cada rama.
          libPaths = lib.concatMapStringsSep ":" (p: "${p}/lib") runtimeLibs;
          libNames = lib.concatMapStringsSep " " (p: toString p) runtimeLibs;

          port = pkgs.rustPlatform.buildRustPackage {
            pname = "port";
            version = "0.1.0";

            # El filtro deja fuera `target/`, `result/` y `dist/`. Sin eso Nix
            # recorre esos arboles y acaba intentando copiar /nix/store desde si
            # mismo: "path /nix/store/ is not in the Nix store".
            src = lib.cleanSourceWith {
              src = ./.;
              name = "port-source";
              filter = path: type:
                let base = lib.baseNameOf (toString path);
                in type == "directory"
                  || !(lib.elem base [ "target" "result" "dist" ]);
            };

            # El sandbox de Nix no lee el shell.nix del repo, asi que las
            # variables de enlazado se declaran aqui. Si el binario no enlaza
            # dentro del sandbox, tampoco enlazara fuera.
            LIBRARY_PATH = libPaths;
            RUSTFLAGS = lib.concatMapStringsSep " " (p:
              "-C link-arg=-Wl,-rpath,${p}/lib") runtimeLibs;
            PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" runtimeLibs;

            nativeBuildInputs = (with pkgs; [
              pkg-config
              cmake
              nasm
              python3
              fontconfig
              harfbuzz
              patchelf
            ]) ++ runtimeLibs;

            buildAndTestSubdir = ".";

            # Los plugins son dependencias git desde un tag. `importCargoLock`
            # los vendoriza y exige un hash por commit; todos vienen del mismo
            # commit de port-plugins, asi que comparten hash. Si se publica un
            # tag nuevo hay que recalcularlo con:
            #   nix hash path --sri <port-plugins sin .git>
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "port-plugin-close-guard-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
                "port-plugin-font-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
                "port-plugin-font-zoom-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
                "port-plugin-herdr-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
                "port-plugin-shortcuts-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
                "port-plugin-transparency-0.1.0" = "sha256-A6LCkUE4PpN9p8qJhcyAHxSvzeg46C52KPxDDDky0fY=";
              };
            };

            # `RUSTFLAGS` solo llega al binario principal. Las .so que cargo
            # baja (blade-graphics, gpu-alloc) tambien abren Wayland y Vulkan por
            # dlopen, asi que necesitan las rutas en su propio RPATH. Sin esto
            # el binario compila bien pero al arrancar falla con NoWaylandLib.
            postFixup = ''
              for l in ${libNames}; do
                patchelf --set-rpath "$l/lib:$out/lib" \
                  $out/lib/*.so* 2>/dev/null || true
              done
              patchelf --set-rpath "${libPaths}:$out/lib" "$out/bin/port"
            '';

            # portable-pty resuelve el cwd por defecto a traves de $HOME. El
            # sandbox de Nix deja esa variable en un directorio temporal que
            # luego borra, y el PTY falla con ENOENT al arrancar el shell.
            HOME = "/tmp/port-home";
            preBuild = ''
              mkdir -p "$HOME"
            '';

            # `doCheck` queda desactivado a proposito: casi toda la suite de
            # term-core arranca un PTY de verdad, y el sandbox de Nix no monta
            # /dev/ptmx, asi que los tests de sesion fallarian por el entorno y
            # no por el codigo. La suite completa corre en GitHub Actions, donde
            # si hay PTY.
            doCheck = false;

            meta = {
              description = "PORT: Plugin-Oriented Rust Terminal";
              longDescription = ''
                Terminal emulator written in Rust on GPUI, with a plugin system
                where any core behaviour can be replaced without touching the
                core.
              '';
              license = lib.licenses.mit;
              mainProgram = "port";
              platforms = lib.platforms.unix;
            };
          };
        in
        {
          inherit port;
          default = port;
        });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.port}/bin/port";
        };
      });

      devShells = forAllSystems (pkgs:
        let
          # GPUI enlaza de forma explicita contra `libxkbcommon-x11`. En
          # nixpkgs ese `.so` va DENTRO del paquete `libxkbcommon`, asi que no
          # hay un atributo separado que declarar. En Debian, en cambio, si es
          # un paquete aparte: de ahi que el CI lo instale explicitly.
          runtimeLibs = with pkgs; [
            libxcb
            libxkbcommon
            freetype
            wayland
            vulkan-loader
          ];
        in {
          default = pkgs.mkShell {
            packages = (with pkgs; [
              cargo
              rustc
              pkg-config
              cmake
              nasm
              python3
              fontconfig
              harfbuzz
            ]) ++ runtimeLibs;

            # Mismas variables que la build, para que `cargo test` dentro del
            # shell enlace igual que el binario publicado.
            LIBRARY_PATH =
              lib.concatMapStringsSep ":" (p: "${p}/lib") runtimeLibs;
            RUSTFLAGS = lib.concatMapStringsSep " " (p:
              "-C link-arg=-Wl,-rpath,${p}/lib") runtimeLibs;
            PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" runtimeLibs;

            shellHook = ''
              echo "PORT: shell listo (fonte $(command -v cargo))"
            '';
          };
        });
    };
}