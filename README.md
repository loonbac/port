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
- **Plugin system** — two kinds, one contract. In-process Rust traits for
  anything that needs to draw with GPUI, and standalone processes for plugins
  you install without rebuilding PORT.
- **Install plugins from a repository URL** — `Ctrl` `Shift` `L`, then the
  install entry, then paste the URL. It clones, builds and loads the plugin
  without restarting the terminal.
- **A plugin cannot take down the terminal** — a panicking hook is isolated,
  and a hook that overruns its time budget is disabled instead of freezing the
  session forever.
- **Human-readable configuration** — one plain-text file with Markdown code
  blocks per plugin, hot-reloaded as you save.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl` `Shift` `L` | Open the plugin manager (browse, enable, install) |
| `Ctrl` `Shift` `T` | New tab in the current space |
| `Alt` `←` / `Alt` `→` | Switch tab |
| `Ctrl` `W` | Close current tab |
| `Ctrl` `Alt` `T` | New space (opens the sidebar) |
| `Alt` `1`..`9` | Switch space |
| `Ctrl` `+` / `Ctrl` `-` / `Ctrl` `0` | Zoom font in / out / reset (via `shortcuts`) |
| `Ctrl` `Shift` `C` | Copy the terminal selection to the clipboard |
| `Ctrl` `Shift` `V` | Paste the clipboard into the terminal |

Mouse: drag to select, double-click a word, triple-click a line. Releasing the
button copies the selection to the clipboard.

Programs that capture the mouse — pi, vim, less — receive the click, the drag
and the release as terminal mouse reports (SGR), so their own dragging works:
the drag belongs to the program. Hold `Shift` while dragging to give it back to
PORT and select text locally. A plain shell still selects with a plain drag.

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

## Installation

### Native packages (recommended)

Download the `.deb` or `.rpm` from the
[GitHub releases page](https://github.com/loonbac/port/releases) and install it
like any other terminal:

```bash
# Debian / Ubuntu
sudo apt install ./port-<version>.deb

# Fedora / RHEL
sudo dnf install ./port-<version>.rpm
```

The packages install the binary in `/usr/bin` and the `.desktop` entry and icon
in the system paths, so PORT shows up in your applications menu. They are built
inside Debian 12 against glibc 2.36, which is why they run on Ubuntu 22.04 or
later, Fedora 36 or later, and RHEL/Rocky 9.

### Nix (optional)

```bash
nix profile install github:loonbac/port
```

Useful on NixOS, and for reproducible builds. It is **not** required: a binary
built through Nix carries the store's dynamic loader inside it, so it runs on
NixOS and nowhere else.

### Requirements

A Wayland or X11 session and a working Vulkan driver.

## Building

Requires Nix, or a Rust toolchain plus the Wayland/XCB/Vulkan development
libraries.

```bash
nix-shell                      # environment with native libraries linked
cargo test                     # 67 unit and contract tests
cargo build --release -p port
./target/release/port
```

## Installing a plugin

Plugins that do not need to draw with GPUI install as standalone processes.
Give PORT the repository URL and it clones, builds and loads the plugin:

```sh
port plugin add https://github.com/you/port-plugin-example   # a URL de un repositorio
port plugin list
port plugin remove example
```

The same thing works from the keyboard: `Ctrl` `Shift` `L`, pick the install
entry, paste the URL, `Enter`. The build runs in the background — about six
seconds for a small plugin — and the menu stays usable while it runs.

A repository is a plugin when its `Cargo.toml` produces a binary and
optionally says how to register it:

```toml
[package.metadata.port]
id = "example"
capabilities = ["appearance", "input"]
```

Without that section the identifier is derived from the package name.

There is a complete, buildable example in
[`examples/external-plugin/`](examples/external-plugin). The end-to-end test
builds and runs that very example, so it cannot go stale.

Loading a plugin costs 0.71 ms and each call costs 12 µs, both measured on
this machine; see [`docs/plugin-runtime.md`](docs/plugin-runtime.md) for the
design and the numbers.

## Writing a plugin

### In-process, when you need to draw

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

Available hooks: `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` and
`LifecycleHook`.

There is deliberately no hook for the plugin manager itself. That menu is core:
it is the way into everything PORT can do, and it must not depend on an
extension loading in order to be usable.

### Standalone, when you do not

A plugin that only adjusts values, answers key bindings or vetoes the close
does not need to be inside the process. It ships as its own binary that talks
JSON over stdin/stdout, and it never compiles against GPUI:

```rust
use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};

struct MyPlugin;

impl Plugin for MyPlugin {
    fn name(&self) -> &'static str { "my-plugin" }
    fn version(&self) -> &'static str { "0.1.0" }
    fn capabilities(&self) -> Vec<Capability> { vec![Capability::Appearance] }
    fn appearance(&self) -> Appearance {
        Appearance { opacity: Some(0.90), ..Default::default() }
    }
    fn bindings(&self) -> Vec<Binding> {
        vec![Binding { key: "k".into(), ctrl: true, alt: false,
                       shift: false, action: "toggle".into() }]
    }
    fn invoke(&self, action: &str, _p: &serde_json::Value) -> Option<serde_json::Value> {
        (action == "toggle").then(|| serde_json::json!({ "ok": true }))
    }
}

fn main() { serve(MyPlugin); }
```

[`docs/plugin-runtime.md`](docs/plugin-runtime.md) has the protocol, the
reasoning behind it and the trade-offs.

### Which one should I write?

Reach for in-process when the plugin draws: a sidebar, a status bar, anything
that composes GPUI elements. It is the only path that can, and it is also the
faster one.

Reach for a standalone process when the plugin only moves values. It gets a
hard kill the in-process path cannot offer, and it cannot be linked against
PORT's internals at all. It costs one idle process, about 1 MB and 0 % CPU.

Neither is a downgrade. The split exists so that plugins with UI and plugins
from strangers can both work without compromise.

### Calling one plugin from another

A plugin can publish a capability with `Plugin::services()` and another can
invoke it by identifier, with no shared types and no cloning of plugin internals:

```rust
// In the providing plugin.
impl Plugin for MyPlugin {
    fn services(&self) -> Vec<Arc<dyn Service>> {
        vec![Arc::new(MyService)]
    }
}

// In the consuming plugin.
shortcuts.bind_service("ctrl+=", "my-plugin", "do_thing");
```

`bind_service` resolves the target at keypress time, so registration order does
not matter.

## Appendix: plugins

Official plugins live in **[loonbac/port-plugins](https://github.com/loonbac/port-plugins)**
— a separate repository so the terminal and its extensions evolve independently.

| Plugin | Hooks | What it does |
|---|---|---|
| `transparency` | `AppearanceHook` | Window background opacity |
| `font` | `AppearanceHook` | Font family, size and fallbacks |
| `font-zoom` | `AppearanceHook` | Font size state and zoom operations (no key bindings) |
| `shortcuts` | — | Bind custom key combinations to callbacks |
| `herdr` | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | Spaces sidebar, tabs, live process detection, resizable layout, wallpaper accent colour |
| `close-guard` | `LifecycleHook` | Asks for confirmation before closing with running processes |

See the [plugins repository](https://github.com/loonbac/port-plugins) for
detailed documentation of each one.

All of these are in-process. A standalone plugin is not listed here: it is
installed by URL and does not ship as a dependency, exactly like any plugin you
would write yourself.

## License

MIT
