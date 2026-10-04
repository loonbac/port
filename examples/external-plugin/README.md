# PORT external plugin example

A complete, working plugin that runs as its own process.

This is what a plugin looks like when it does not need to draw with GPUI: it
adjusts values, answers a key combination and reports what the core asked it.

## Install it

`port plugin add` accepts a repository URL or a local directory, which is what
you want while writing a plugin:

```sh
port plugin add https://github.com/you/port-plugin-example   # a repository
port plugin add ./my-plugin                                   # a local directory
```

From the keyboard: open the plugin manager with `Ctrl` `Shift` `L`, pick the
install entry, paste the path, press `Enter`.

Either way it clones or copies, builds, installs and loads the plugin. Once it
is loaded, `Ctrl` `Alt` `H` is bound to this plugin's `toggle` action.

## Why this works

`port-plugin-sdk` depends only on `serde`. It does **not** pull in GPUI, and
neither do you: your binary is linked against the protocol, not against PORT's
internals. Installing is therefore about as heavy as building any small Rust
binary, not like rebuilding the terminal.

## The whole plugin

```rust
use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::{serve, Plugin};
use std::collections::BTreeMap;

struct Example {
    opacity: f32,
    triggers: u32,
}

impl Plugin for Example {
    fn name(&self) -> &'static str {
        "example"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Appearance, Capability::Input]
    }

    fn appearance(&self) -> Appearance {
        Appearance {
            opacity: Some(self.opacity),
            ..Default::default()
        }
    }

    fn bindings(&self) -> Vec<Binding> {
        vec![Binding {
            key: "h".into(),
            ctrl: true,
            alt: true,
            shift: false,
            action: "toggle".into(),
        }]
    }

    fn invoke(&self, action: &str, _params: &serde_json::Value) -> Option<serde_json::Value> {
        (action == "toggle").then(|| serde_json::json!({ "triggers": self.triggers }))
    }

    fn configure(&mut self, values: &BTreeMap<String, String>) {
        if let Some(op) = values.get("opacity").and_then(|v| v.parse::<f32>().ok()) {
            self.opacity = op;
        }
    }
}

fn main() {
    serve(Example { opacity: 0.94, triggers: 0 });
}
```

Two details worth copying:

- **Declare your key bindings.** The core resolves them locally, so typing does
  not touch a pipe. Only a real match asks this process anything.
- **Return `None` for actions you do not handle.** It means "not mine", not
  "something went wrong".

## This example is tested

`crates/port-app/tests/external_plugin.rs` builds this exact directory, installs
it, starts it as a child process and exercises it. If it stops working, the test
suite says so instead of the example quietly rotting in the repository.

## Design and measurements

See [`../../docs/plugin-runtime.md`](../../docs/plugin-runtime.md) for the
protocol, why plugins are separate processes, and the measured cost.