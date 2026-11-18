# Taplet

A lightweight, Wayland-native autoclicker with a friendly GUI, inspired by OP Auto Clicker. Built with Rust and Iced. No X11 dependency and no root required.

## Status

- Backends: `zwlr_virtual_pointer_v1` (wlroots-style compositors such as Hyprland and Sway) and `uinput`, selectable manually or auto-detected. Verified end to end against a headless Sway: press/release delivery, interval accuracy (99–101 ms at a 100 ms setting), fixed counts, double clicks, right button.
- Not yet verified on Hyprland itself, so test there before relying on it.
- Not implemented yet: `libei` / RemoteDesktop portal backend.
- Clicks follow the cursor by default; a fixed screen position is also available.
- The window resizes from 280×200 to maximized. Small sizes hide secondary controls; wide windows use two columns.
- The window maps with fixed-size hints so tiling compositors float it, then unlocks resizing after a short delay. Hyprland and Sway also get an explicit float request.

## Build

```
cargo build --release
```

Requires a recent stable Rust toolchain (edition 2024), Wayland client libraries, and `libdbus-1` development headers.

## System tray

When the desktop exposes a StatusNotifierItem host (KDE, Hyprland/waybar with a tray, sway with a tray plugin, GNOME with an AppIndicator extension), Taplet adds a tray icon. Closing the window does not quit the app; it lives on in the tray. Left click shows/hides the window; right click opens the menu:

- Show/hide window
- Start/Stop clicking (label and icon follow the current state)
- Speed: checkmarked presets from 1000 ms (1 cps) to 10 ms (100 cps). Applies to the next run, same as the interval field in the window. Items use checkmarks rather than a real slider because the StatusNotifierItem menu protocol has no slider control.
- Quit Taplet (actually quits)

Hide/show uses `xdg_toplevel.set_minimized` on Wayland and real hiding on X11. On Wayland, restoring from the tray requests focus but the compositor decides whether to unminimize; X11 restores reliably. If your compositor ignores minimize requests, the window will stay visible on close.

If no tray host is available the app logs a warning, the close button quits the app as usual, and everything else works unchanged. The tray icon uses the same `icon.png` (downscaled, sent as a pixmap, with `icon_name = taplet` for theme lookup).

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

## Install

The Nix flake installs the binary, `taplet.desktop` and the icon into the hicolor theme. For a manual install:

```
cargo build --release
install -Dm755 target/release/taplet ~/.local/bin/taplet
install -Dm644 packaging/taplet.desktop ~/.local/share/applications/taplet.desktop
install -Dm644 icon.png ~/.local/share/icons/hicolor/1024x1024/apps/taplet.png
```

The window itself carries the same icon, and the desktop entry sets `StartupWMClass=taplet` so Wayland taskbars match the window to the entry. The icon is not shown inside the app.

## Configuration

Stored at `~/.config/taplet/config.toml`:

```
interval_ms = 100
button = "Left"
click_type = "Single"
repeat_mode = "Continuous"
repeat_count = 100
theme = "Dark"
accent = "Violet"
custom_accent = [139, 92, 246]
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
- `src/tray.rs`: StatusNotifierItem system tray (start/stop, speed presets, quit)
- `src/ui/`: theme and views

## Safety

Buttons are always released on stop, completion, errors, reconnect and shutdown.
