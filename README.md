# Taplet

A lightweight, Wayland-native autoclicker with a friendly GUI, inspired by OP Auto Clicker. Built with Rust and Iced. No X11 dependency and no root required.

## Status

- Backend: `zwlr_virtual_pointer_v1` (wlroots-style compositors such as Hyprland and Sway). Verified end to end against a headless Sway: press/release delivery, interval accuracy (99–101 ms at a 100 ms setting), fixed counts, double clicks, right button.
- Not yet verified on Hyprland itself, so test there before relying on it.
- Not implemented yet: `libei` / RemoteDesktop portal backend, `uinput` fallback, system tray.
- Clicks happen at the current cursor position. Fixed-position clicking is intentionally absent because this backend cannot read the cursor position.

## Build

```
cargo build --release
```

Requires a recent stable Rust toolchain (edition 2024) and Wayland client libraries.

## Global shortcut

Wayland has no portable global hotkeys, so Taplet is controlled through a local socket. Bind a key in your compositor to call the CLI:

```
bind = , F6, exec, taplet toggle
```

Other commands: `taplet start`, `taplet stop`. The Advanced page shows this line for the shortcut you choose, with a copy button. The in-window shortcut only works while the Taplet window is focused.

## Configuration

Stored at `~/.config/taplet/config.toml`:

```
interval_ms = 100
button = "Left"
click_type = "Single"
repeat_mode = "Continuous"
repeat_count = 100
theme = "Dark"
shortcut = "F6"
```

## Layout

- `src/main.rs`: entry point and CLI
- `src/app.rs`: Iced state, messages, update loop
- `src/config.rs`: settings and TOML persistence
- `src/engine/`: click scheduler on a dedicated thread, monotonic deadlines, immediate cancellation
- `src/input/`: backend trait and the virtual pointer implementation
- `src/ipc.rs`: Unix socket remote control
- `src/ui/`: theme and views

## Safety

Buttons are always released on stop, completion, errors, reconnect and shutdown.
