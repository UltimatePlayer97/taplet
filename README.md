# Taplet

A lightweight, Wayland-native autoclicker with a friendly GUI, inspired by OP Auto Clicker. Built with Rust and Iced. No X11 dependency and no root required.

## Status

- Backends: `zwlr_virtual_pointer_v1` (wlroots-style compositors such as Hyprland and Sway) and `uinput`, selectable manually or auto-detected. Verified end to end against a headless Sway: press/release delivery, interval accuracy (99–101 ms at a 100 ms setting), fixed counts, double clicks, right button.
- Not yet verified on Hyprland itself, so test there before relying on it.
- Not implemented yet: `libei` / RemoteDesktop portal backend, system tray.
- Clicks follow the cursor by default; a fixed screen position is also available.

## Build

```
cargo build --release
```

Requires a recent stable Rust toolchain (edition 2024) and Wayland client libraries.

## Global shortcut

Taplet registers the shortcut as a real desktop-wide hotkey. The priority depends on the input backend:

- With the `uinput` backend, the shortcut is read directly from `/dev/input/event*`.
- With the Wayland backend, Taplet first asks the XDG Desktop Portal `GlobalShortcuts` interface, which works regardless of the focused window.
- If the portal is missing or rejects the request, Taplet falls back to reading `/dev/input/event*`.

The fallback needs read access to input devices. On most distributions that means adding your user to the `input` group:

```
sudo usermod -aG input "$USER"
```

The Advanced page shows which path is active. You can still bind a key in your compositor manually as a last resort:

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
- `src/input/`: backend trait, `zwlr_virtual_pointer_v1`, and `uinput` implementations
- `src/hotkey/`: global hotkeys through the XDG Desktop Portal, with a `/dev/input` fallback
- `src/ipc.rs`: Unix socket remote control
- `src/ui/`: theme and views

## Safety

Buttons are always released on stop, completion, errors, reconnect and shutdown.
