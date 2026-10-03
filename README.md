# PORT — Plugin-Oriented Rust Terminal

A terminal emulator written in Rust on [GPUI](https://github.com/zed-industries/zed), built around one idea: **every core behaviour can be replaced by a plugin, without touching the terminal core.**

[Español](README.es.md)

```
┌─ port ───────────────────────────────┐
│ crates/term-core    PTY · VT grid ·   │
│                     input encoding   │
│ crates/plugin-api  public contracts  │
│ crates/port-app    GPUI frontend     │
└──────────────────────────────────────┘
        ▲ everything above is extensible
```

## Architecture

| Crate | Responsibility | Knows about |
|---|---|---|
| `port-term-core` | PTY lifecycle, VT emulation, frame building, key encoding | Nothing from the UI |
| `port-plugin-api` | Public contracts, configuration file, plugin registry | `term-core` only |
| `port-app` | GPUI window, renderer, key routing | Both of the above |

The emulation core is completely free of UI concerns. Every part of the interface
and every policy decision lives behind a trait in `port-plugin-api`, so a
plugin can replace core behaviour without the core knowing it exists.

## Features

- **Terminal emulation** — VT parser via `alacritty_terminal`, scrollback,
  alternate screen, SGR attributes, wide characters.
- **Real rendering performance** — cells are batched into runs, Braille and block
  elements are rasterised as GPU geometry instead of being shaped as glyphs, and
  frames are only rebuilt when the grid actually changes.
- **Multi-session** — every space and every tab owns an isolated PTY; background
  jobs keep running when you switch away.
- **Plugin system** — in-process Rust traits covering appearance, input, layout,
  session management and window lifecycle.
- **Human-readable configuration** — one plain-text file with Markdown code
  blocks per plugin, hot-reloaded as you save.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl` `Shift` `L` | Open the plugin manager |
| `Ctrl` `Shift` `T` | New tab in the current space |
| `Alt` `←` / `Alt` `→` | Switch tab |
| `Ctrl` `W` | Close current tab |
| `Ctrl` `Alt` `T` | New space (opens the sidebar) |
| `Alt` `1`..`9` | Switch space |
| `Ctrl` `+` / `Ctrl` `-` / `Ctrl` `0` | Zoom font in / out / reset |

## Configuration

`~/.config/port/config.md` is created automatically on first run. Each plugin
owns one code block, edited as `key = value`:

````markdown
```font-zoom
default_size = 14
step = 1
min_size = 6
max_size = 72
enabled = true
```

```transparency
opacity = 0.85
enabled = true
```
````

The file is watched: saving it applies changes immediately, including font size,
opacity and which plugins are enabled.

## Building

Requires Nix, or a Rust toolchain plus the Wayland/XCB/Vulkan development
libraries.

```bash
nix-shell                      # environment with native libraries linked
cargo test                     # 63 unit and contract tests
cargo build --release -p port
./target/release/port
```

## Writing a plugin

Add the plugin as a dependency and register it:

```rust
use port_plugin_api::{Plugin, PluginConfig, AppearanceHook};

struct MyPlugin;

impl AppearanceHook for MyPlugin {
    fn opacity(&self) -> Option<f32> { Some(0.90) }
}

impl Plugin for MyPlugin {
    fn id(&self) -> &'static str { "my-plugin" }
    fn name(&self) -> &'static str { "My Plugin" }
    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> { Some(self) }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("enabled", true);
        Some(cfg)
    }
}
```

Available hooks: `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook`,
`PluginManagerHook` and `LifecycleHook`.

## Appendix: plugins

Official plugins live in **[loonbac/port-plugins](https://github.com/loonbac/port-plugins)**
— a separate repository so the terminal and its extensions evolve independently.

| Plugin | Hooks | What it does |
|---|---|---|
| `transparency` | `AppearanceHook` | Window background opacity |
| `font` | `AppearanceHook` | Font family, size and fallbacks |
| `font-zoom` | `AppearanceHook`, `InputHook` | Interactive font zoom with `Ctrl` `+` / `-` / `0` |
| `shortcuts` | — | Bind custom key combinations to callbacks |
| `menu-customizer` | `PluginManagerHook` | Restyle or fully replace the plugin manager |
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Spaces sidebar, tabs, live process detection, resizable layout, wallpaper accent colour |
| `close-guard` | `LifecycleHook` | Asks for confirmation before closing with running processes |

See the [plugins repository](https://github.com/loonbac/port-plugins) for
detailed documentation of each one.

## License

MIT
