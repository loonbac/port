#!/usr/bin/env python3
"""Aplica las correcciones de congelamiento Wayland/Vulkan en GPUI 0.2.2 y Blade 0.7.1.

En GPUI 0.2.2 sobre Wayland/Vulkan, `blade_renderer.rs` ignora `frame.image_index: None`
(VK_ERROR_OUT_OF_DATE_KHR) cuando Wayland redimensiona o inicializa la ventana.
Para solucionarlo se necesitan dos parches:
1. En `blade-graphics`: hacer público `image_index` en `Frame` (`src/vulkan/mod.rs`).
2. En `gpui`: comprobar `frame.image_index.is_none()`, reconstruir la superficie y
   readquirir el frame antes de inicializar la textura.

Este script busca ambos archivos en las rutas dadas (o `/build`, `$CARGO_HOME`,
`~/.cargo/registry`) y los parchea de forma determinista y segura.
"""

import os
import sys

# 1. Parche para blade-graphics
BLADE_ORIG = """#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    swapchain: Swapchain,
    image_index: Option<u32>,"""

BLADE_REPL = """#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    swapchain: Swapchain,
    pub image_index: Option<u32>,"""

# 2. Parche para gpui
GPUI_ORIG = """        let frame = {
            profiling::scope!("acquire frame");
            self.surface.acquire_frame()
        };
        self.command_encoder.init_texture(frame.texture());"""

GPUI_REPL = """        let mut frame = {
            profiling::scope!("acquire frame");
            self.surface.acquire_frame()
        };
        if frame.image_index.is_none() {
            let width = DevicePixels(self.surface_config.size.width as i32);
            let height = DevicePixels(self.surface_config.size.height as i32);
            self.update_drawable_size_impl(Size { width, height }, true);
            frame = self.surface.acquire_frame();
        }

        self.command_encoder.init_texture(frame.texture());"""


def patch_file(path: str, orig: str, repl: str, name: str) -> bool:
    try:
        with open(path, "r", encoding="utf-8") as f:
            content = f.read()
    except Exception as e:
        print(f"Error leyendo {path}: {e}", file=sys.stderr)
        return False

    if orig in content:
        print(f"==> Parcheando {name} en: {path}")
        try:
            os.chmod(path, 0o644)
            with open(path, "w", encoding="utf-8") as f:
                f.write(content.replace(orig, repl))
            return True
        except Exception as e:
            print(f"Error escribiendo {path}: {e}", file=sys.stderr)
            return False
    elif repl in content or ("pub image_index" in content and name == "blade-graphics"):
        print(f"==> {name} ya estaba parcheado en: {path}")
        return True
    return False


def main():
    search_dirs = sys.argv[1:]
    if not search_dirs:
        search_dirs = [
            "/build",
            os.environ.get("CARGO_HOME", ""),
            os.path.expanduser("~/.cargo/registry"),
        ]

    blade_patched = 0
    gpui_patched = 0

    for base in search_dirs:
        if not base or not os.path.exists(base):
            continue
        for root, _dirs, files in os.walk(base):
            if "mod.rs" in files and "vulkan" in root and "blade-graphics" in root:
                p = os.path.join(root, "mod.rs")
                if patch_file(p, BLADE_ORIG, BLADE_REPL, "blade-graphics"):
                    blade_patched += 1
            if "blade_renderer.rs" in files:
                p = os.path.join(root, "blade_renderer.rs")
                if patch_file(p, GPUI_ORIG, GPUI_REPL, "gpui"):
                    gpui_patched += 1

    print(f"==> Total parcheados: blade-graphics={blade_patched}, gpui={gpui_patched}")
    if blade_patched == 0 or gpui_patched == 0:
        print("AVISO: No se encontraron todos los archivos necesarios para parchear.", file=sys.stderr)


if __name__ == "__main__":
    main()
