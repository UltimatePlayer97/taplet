//! `taplet doctor`: environment report meant to be pasted into bug reports.
//!
//! Injects no input unless `--click` is given. Every probe that touches D-Bus, the
//! Wayland socket or a child process runs under a hard timeout, so the command
//! cannot hang on a broken portal.

use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::input::{
    InputBackend, LibeiBackend, ProbeResult, UinputBackend, VirtualPointerBackend, check_libei,
    probe_uinput, probe_wlr,
};
use crate::ipc::{self, RemoteCommand};

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Run `f` on a helper thread and give up after `PROBE_TIMEOUT`. A stuck probe
/// thread is abandoned; the process exits right after the report anyway.
fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok()
}

fn env_or_unset(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| "unset".to_string())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Presence of one portal interface plus its version. `Err` carries the reason.
fn probe_portal_interface(name: &'static str) -> Result<u32, String> {
    crate::portal_rt::run(async move {
        match name {
            "RemoteDesktop" => ashpd::desktop::remote_desktop::RemoteDesktop::new()
                .await
                .map(|p| p.version()),
            "GlobalShortcuts" => ashpd::desktop::global_shortcuts::GlobalShortcuts::new()
                .await
                .map(|p| p.version()),
            _ => ashpd::desktop::screencast::Screencast::new()
                .await
                .map(|p| p.version()),
        }
        .map_err(|e| e.to_string())
    })
    .unwrap_or_else(|| Err("no async runtime".to_string()))
}

fn probe_device_types() -> Result<String, String> {
    use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop};
    crate::portal_rt::run(async {
        let portal = RemoteDesktop::new().await.map_err(|e| e.to_string())?;
        let types = portal
            .available_device_types()
            .await
            .map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        if types.contains(DeviceType::KEYBOARD) {
            names.push("keyboard");
        }
        if types.contains(DeviceType::POINTER) {
            names.push("pointer");
        }
        if types.contains(DeviceType::TOUCHSCREEN) {
            names.push("touchscreen");
        }
        Ok(if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        })
    })
    .unwrap_or_else(|| Err("no async runtime".to_string()))
}

fn probe_tray_owner() -> Option<String> {
    use dbus::blocking::Connection;
    let conn = Connection::new_session().ok()?;
    let (owner,): (String,) = conn
        .with_proxy("org.freedesktop.DBus", "/", PROBE_TIMEOUT)
        .method_call(
            "org.freedesktop.DBus",
            "GetNameOwner",
            ("org.kde.StatusNotifierWatcher",),
        )
        .ok()?;
    Some(owner)
}

/// Group membership by id, from the kernel's view of this process.
fn in_input_group() -> Option<bool> {
    let out = std::process::Command::new("id").arg("-Gn").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .any(|g| g == "input"),
    )
}

/// Name of the group owning `path`, from /etc/group.
fn owner_group(path: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let gid = std::fs::metadata(path).ok()?.gid();
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    groups.lines().find_map(|line| {
        let mut parts = line.split(':');
        let name = parts.next()?;
        let _pw = parts.next()?;
        (parts.next()?.parse::<u32>().ok()? == gid).then(|| name.to_string())
    })
}

fn in_group(name: &str) -> Option<bool> {
    let out = std::process::Command::new("id").arg("-Gn").output().ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .any(|g| g == name)
    })
}

fn any_input_event_readable() -> bool {
    std::fs::read_dir("/dev/input")
        .map(|entries| {
            entries.flatten().any(|e| {
                e.file_name().to_string_lossy().starts_with("event")
                    && std::fs::File::open(e.path()).is_ok()
            })
        })
        .unwrap_or(false)
}

/// Why `/dev/uinput` is unusable, or `None` when it works.
fn uinput_problem(flatpak: bool) -> Option<String> {
    if probe_uinput() {
        return None;
    }
    let path = std::path::Path::new("/dev/uinput");
    let mut reason = if path.exists() {
        match owner_group("/dev/uinput") {
            Some(group) if in_group(&group) == Some(false) => format!(
                "/dev/uinput exists but is not writable: owned by group `{group}`, you are not in it (log out and in after adding)"
            ),
            Some(group) => format!(
                "/dev/uinput exists but is not writable (group `{group}`; udev rule may be missing)"
            ),
            None => "/dev/uinput exists but is not writable (add a udev rule or group access)"
                .to_string(),
        }
    } else {
        "/dev/uinput missing (load the uinput module; NixOS: hardware.uinput.enable = true;)"
            .to_string()
    };
    if flatpak {
        reason.push_str("; inside Flatpak also needs: flatpak override --user --device=all <app-id>");
    }
    Some(reason)
}

pub fn run(args: &[String]) -> ExitCode {
    let mut click_test = false;
    let mut hotkey_test = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--click" => click_test = true,
            "--hotkey" => hotkey_test = true,
            "-h" | "--help" => {
                println!(
                    "taplet doctor [--click] [--hotkey]\n  Diagnose backend and environment. Injects no input by default.\n  --click:  perform ONE left click with the chosen backend (libei may show a permission prompt)\n  --hotkey: test-bind F6 through xx_hotkey_v1 and the GlobalShortcuts portal, print exact errors"
                );
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("Unknown doctor flag: {arg}");
                return ExitCode::FAILURE;
            }
        }
    }

    let flatpak = std::path::Path::new("/.flatpak-info").exists();
    let desktop = env_or_unset("XDG_CURRENT_DESKTOP");

    println!("=== taplet doctor ===");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    if let Some(commit) = option_env!("GIT_COMMIT") {
        println!("Git commit: {commit}");
    }
    println!("XDG_CURRENT_DESKTOP: {desktop}");
    println!("XDG_SESSION_TYPE: {}", env_or_unset("XDG_SESSION_TYPE"));
    println!("WAYLAND_DISPLAY: {}", env_or_unset("WAYLAND_DISPLAY"));
    println!("Flatpak: {}", yes_no(flatpak));

    println!("\n--- Wayland ---");
    let wlr = bounded(probe_wlr);
    match wlr {
        Some(v) => println!("zwlr_virtual_pointer_manager_v1: {}", yes_no(v)),
        None => println!("zwlr_virtual_pointer_manager_v1: timeout"),
    }
    match bounded(crate::hotkey::xx_available) {
        Some(v) => println!("xx_hotkey_manager_v1 (zero-config hotkey): {}", yes_no(v)),
        None => println!("xx_hotkey_manager_v1 (zero-config hotkey): timeout"),
    }

    println!("\n--- Portals ---");
    // RemoteDesktop first and its verdict kept: a slow GlobalShortcuts or ScreenCast
    // backend can stall the portal frontend and make later probes time out.
    let libei_check = bounded(check_libei);
    let mut portal_timeout = false;
    for name in ["RemoteDesktop", "GlobalShortcuts", "ScreenCast"] {
        match bounded(move || probe_portal_interface(name)) {
            Some(Ok(version)) => println!("{name}: yes (version {version})"),
            Some(Err(reason)) => println!("{name}: no ({reason})"),
            None => {
                portal_timeout = true;
                println!("{name}: timeout");
            }
        }
    }
    match bounded(probe_device_types) {
        Some(Ok(types)) => println!("RemoteDesktop AvailableDeviceTypes: {types}"),
        Some(Err(_)) | None => println!("RemoteDesktop AvailableDeviceTypes: unavailable"),
    }
    if portal_timeout {
        println!(
            "note: a portal call timed out. The portal backend is probably dead or hung: check\n  systemctl --user status xdg-desktop-portal xdg-desktop-portal-hyprland\n  journalctl --user -u xdg-desktop-portal -b"
        );
    }

    println!("\n--- Input devices ---");
    let uinput_exists = std::path::Path::new("/dev/uinput").exists();
    println!(
        "/dev/uinput: exists={} writable={}",
        yes_no(uinput_exists),
        yes_no(probe_uinput())
    );
    println!(
        "/dev/input/event*: readable={}",
        yes_no(any_input_event_readable())
    );
    match in_input_group() {
        Some(v) => println!("input group member: {}", yes_no(v)),
        None => println!("input group member: unknown"),
    }
    if let Some(group) = owner_group("/dev/uinput") {
        match in_group(&group) {
            Some(v) => println!("/dev/uinput group `{group}` member: {}", yes_no(v)),
            None => println!("/dev/uinput group `{group}` member: unknown"),
        }
    }

    println!("\n--- Tray ---");
    match bounded(probe_tray_owner) {
        Some(Some(owner)) => println!("StatusNotifierWatcher: yes (owner {owner})"),
        Some(None) => println!("StatusNotifierWatcher: no"),
        None => println!("StatusNotifierWatcher: timeout"),
    }

    println!("\n--- Backend selection ---");
    let wlr_ok = wlr.unwrap_or(false);
    let libei_ok = matches!(libei_check, Some(Ok(())));
    let uinput_issue = uinput_problem(flatpak);
    let uinput_ok = uinput_issue.is_none();

    println!(
        "wlr:    {}",
        if wlr_ok {
            "usable".to_string()
        } else if wlr.is_none() {
            "rejected: probe timed out".to_string()
        } else if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            "rejected: no Wayland session (WAYLAND_DISPLAY unset)".to_string()
        } else {
            "rejected: zwlr_virtual_pointer_manager_v1 not advertised by compositor".to_string()
        }
    );
    println!(
        "libei:  {}",
        match &libei_check {
            Some(Ok(())) => "usable (portal advertises pointer; first use asks permission)".to_string(),
            Some(Err(e)) => format!("rejected: {e}"),
            None => "rejected: probe timed out".to_string(),
        }
    );
    println!(
        "uinput: {}",
        match &uinput_issue {
            None => "usable".to_string(),
            Some(reason) => format!("rejected: {reason}"),
        }
    );

    let backend = crate::input::select_backend(wlr_ok, libei_ok, uinput_ok);
    println!("Auto-detect picks: {backend:?}");

    if hotkey_test {
        println!("\n--- Hotkey bind tests ---");
        if bounded(crate::hotkey::xx_available).unwrap_or(false) {
            match crate::hotkey::probe_xx("F6", Duration::from_secs(10)) {
                Ok(()) => println!("xx_hotkey_v1 bind: OK (F6 accepted by compositor)"),
                Err(err) => println!("xx_hotkey_v1 bind: FAILED - {err}"),
            }
        } else {
            println!("xx_hotkey_v1 bind: skipped (compositor does not advertise it)");
        }
        match crate::hotkey::probe_portal("F6", Duration::from_secs(10)) {
            Ok(()) => println!("portal bind: OK (shortcut `taplet-toggle` registered)"),
            Err(err) => println!("portal bind: FAILED - {err}"),
        }
    }

    println!("\n--- Running instance ---");
    if ipc::instance_running() {
        match ipc::send_with_reply(RemoteCommand::Status, Duration::from_secs(2)) {
            Ok(reply) => println!("{reply}"),
            Err(e) => println!("query failed ({e})"),
        }
    } else {
        println!("not running");
    }

    if click_test {
        println!("\n--- Click test ---");
        match backend {
            ProbeResult::Libei => {
                println!("libei: a permission prompt may appear. Approve it to complete the test.");
            }
            ProbeResult::None => println!("no backend usable; trying uinput to show its error"),
            _ => {}
        }
        match click_once(backend) {
            Ok(()) => println!("Click test: OK"),
            Err(e) => println!("Click test: FAILED - {e}"),
        }
    }

    ExitCode::SUCCESS
}

fn click_once(backend: ProbeResult) -> Result<(), String> {
    use crate::config::Config;
    use crate::engine::ClickPlan;

    let config = Config::load().unwrap_or_default();
    let plan = ClickPlan::from_config(&config);

    fn run_click<B: InputBackend>(mut be: B, plan: &ClickPlan, label: &str) -> Result<(), String> {
        be.connect().map_err(|e| format!("{label} connect: {e}"))?;
        crate::engine::click(&mut be, plan).map_err(|e| format!("{label} click: {e}"))?;
        be.release_all().map_err(|e| format!("{label} release: {e}"))?;
        be.disconnect().map_err(|e| format!("{label} disconnect: {e}"))
    }

    match backend {
        ProbeResult::Wlr => run_click(VirtualPointerBackend::new(), &plan, "wlr"),
        ProbeResult::Libei => run_click(
            LibeiBackend::new(config.libei_restore_token.clone()),
            &plan,
            "libei",
        ),
        ProbeResult::Uinput | ProbeResult::None => run_click(UinputBackend::new(), &plan, "uinput"),
    }
}
