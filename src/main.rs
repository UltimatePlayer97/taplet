mod app;
mod config;
mod engine;
mod error;
mod hotkey;
mod input;
mod ipc;
mod linux;
mod tray;
mod ui;

use std::process::ExitCode;
use std::time::Duration;

use ipc::RemoteCommand;
use input::{auto_select_backend, probe_wlr, ProbeResult};

const HELP: &str = "\
Taplet - Wayland-native autoclicker

Usage:
  taplet                Open the GUI
  taplet toggle         Start or stop clicking in the running instance
  taplet start          Start clicking in the running instance
  taplet stop           Stop clicking in the running instance
  taplet doctor         Diagnose backend and environment
  taplet doctor --click Perform a test click (requires permission)
  taplet --help         Show this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            if ipc::instance_running() {
                eprintln!("Taplet is already running.");
                return ExitCode::FAILURE;
            }
            match app::run() {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("Taplet failed: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("doctor") => run_doctor(&args),
        Some(other) => match RemoteCommand::parse(other) {
            Some(command) => match ipc::send(command) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("Could not reach a running Taplet instance: {err}");
                    ExitCode::FAILURE
                }
            },
            None => {
                eprint!("Unknown command: {other}\n\n{HELP}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_doctor(args: &[String]) -> ExitCode {
    let mut click_test = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--click" => click_test = true,
            "-h" | "--help" => {
                println!("taplet doctor [--click]\n  Diagnose backend and environment.\n  --click: perform a test click (may show permission prompt)");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("Unknown doctor flag: {arg}");
                return ExitCode::FAILURE;
            }
        }
    }

    println!("=== taplet doctor ===");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    if let Some(commit) = option_env!("GIT_COMMIT") {
        println!("Git commit: {commit}");
    }
    println!("XDG_CURRENT_DESKTOP: {}", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unset".to_string()));
    println!("XDG_SESSION_TYPE: {}", std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unset".to_string()));
    println!("WAYLAND_DISPLAY: {}", std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "unset".to_string()));
    println!("Flatpak: {}", if std::path::Path::new("/.flatpak-info").exists() { "yes" } else { "no" });

    let wlr = probe_wlr();
    println!("zwlr_virtual_pointer_manager_v1: {}", if wlr { "yes" } else { "no" });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (portal_avail, portal_version, portal_devices) = rt.block_on(probe_portal());
    println!("RemoteDesktop portal: {}", if portal_avail { "yes" } else { "no" });
    if portal_avail {
        println!("  version: {portal_version}");
        println!("  AvailableDeviceTypes: {portal_devices:?}");
    }
    let (gs_avail, gs_version) = rt.block_on(probe_global_shortcuts());
    println!("GlobalShortcuts portal: {}", if gs_avail { "yes" } else { "no" });
    if gs_avail {
        println!("  version: {gs_version}");
    }
    let (sc_avail, sc_version) = rt.block_on(probe_screencast());
    println!("ScreenCast portal: {}", if sc_avail { "yes" } else { "no" });
    if sc_avail {
        println!("  version: {sc_version}");
    }

    let (uinput_exists, uinput_writable, input_readable, input_group) = probe_input_devices();
    println!("/dev/uinput: exists={uinput_exists} writable={uinput_writable}");
    println!("/dev/input/event*: readable={input_readable} input_group={input_group}");

    let (tray_avail, tray_owner) = rt.block_on(probe_tray());
    println!("StatusNotifierWatcher: {}", if tray_avail { "yes" } else { "no" });
    if tray_avail {
        println!("  owner: {tray_owner}");
    }

    let backend = auto_select_backend();
    println!("Auto-detect backend: {backend:?}");
    match backend {
        ProbeResult::Wlr => println!("  -> wlr-virtual-pointer selected (zwlr_virtual_pointer_manager_v1 present)"),
        ProbeResult::Libei => println!("  -> libei selected (RemoteDesktop portal available with POINTER device)"),
        ProbeResult::Uinput => println!("  -> uinput selected (/dev/uinput writable)"),
        ProbeResult::None => println!("  -> none (no suitable backend found)"),
    }

    if ipc::instance_running() {
        match ipc::send_with_reply(RemoteCommand::Status, Duration::from_secs(2)) {
            Ok(reply) => println!("Running instance: {reply}"),
            Err(e) => println!("Running instance: query failed ({e})"),
        }
    } else {
        println!("Running instance: not running");
    }

    if click_test {
        println!("\n--- Click test ---");
        let test_backend = if matches!(backend, ProbeResult::None) {
            ProbeResult::Uinput
        } else {
            backend
        };
        match run_click_test(test_backend) {
            Ok(()) => println!("Click test: OK"),
            Err(e) => println!("Click test: FAILED - {e}"),
        }
    }

    ExitCode::SUCCESS
}

async fn probe_portal() -> (bool, u32, ashpd::desktop::remote_desktop::DeviceType) {
    use ashpd::desktop::remote_desktop::RemoteDesktop;
    let portal = match RemoteDesktop::new().await {
        Ok(p) => p,
        Err(_) => return (false, 0, ashpd::desktop::remote_desktop::DeviceType::empty()),
    };
    let version = portal.version();
    let types = match portal.available_device_types().await {
        Ok(t) => t,
        Err(_) => return (true, version, ashpd::desktop::remote_desktop::DeviceType::empty()),
    };
    (true, version, types)
}

async fn probe_global_shortcuts() -> (bool, u32) {
    use ashpd::desktop::global_shortcuts::GlobalShortcuts;
    let portal = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(_) => return (false, 0),
    };
    (true, portal.version())
}

async fn probe_screencast() -> (bool, u32) {
    use ashpd::desktop::screencast::Screencast;
    let portal = match Screencast::new().await {
        Ok(p) => p,
        Err(_) => return (false, 0),
    };
    (true, portal.version())
}

async fn probe_tray() -> (bool, String) {
    use dbus::blocking::Connection;
    let conn = match Connection::new_session() {
        Ok(c) => c,
        Err(_) => return (false, "session bus unavailable".to_string()),
    };
    match conn.with_proxy("org.freedesktop.DBus", "/", Duration::from_secs(2)).method_call::<_, (String,), _, _>("org.freedesktop.DBus", "GetNameOwner", ("org.kde.StatusNotifierWatcher".to_string(),)) {
        Ok((owner,)) => (true, owner),
        Err(_) => (false, "no owner".to_string()),
    }
}

fn probe_input_devices() -> (bool, bool, bool, bool) {
    let uinput_exists = std::path::Path::new("/dev/uinput").exists();
    let uinput_writable = std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_ok();
    let mut input_readable = false;
    let mut input_group = false;
    if let Ok(entries) = std::fs::read_dir("/dev/input") {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("event") {
                if std::fs::OpenOptions::new().read(true).open(entry.path()).is_ok() {
                    input_readable = true;
                }
            }
        }
    }
    if let Ok(groups) = std::process::Command::new("groups").output() {
        let out = String::from_utf8_lossy(&groups.stdout);
        input_group = out.split_whitespace().any(|g| g == "input");
    }
    (uinput_exists, uinput_writable, input_readable, input_group)
}

fn run_click_test(backend: ProbeResult) -> Result<(), String> {
    use crate::config::Config;
    use crate::engine::ClickPlan;
    use crate::input::{InputBackend, LibeiBackend, UinputBackend, VirtualPointerBackend};

    let config = Config::load().unwrap_or_default();
    let plan = ClickPlan::from_config(&config);

    match backend {
        ProbeResult::Wlr => {
            let mut be = VirtualPointerBackend::new();
            be.connect().map_err(|e| format!("wlr connect: {e}"))?;
            crate::engine::click(&mut be, &plan).map_err(|e| format!("wlr click: {e}"))?;
            be.disconnect().map_err(|e| format!("wlr disconnect: {e}"))?;
        }
        ProbeResult::Uinput => {
            let mut be = UinputBackend::new();
            be.connect().map_err(|e| format!("uinput connect: {e}"))?;
            crate::engine::click(&mut be, &plan).map_err(|e| format!("uinput click: {e}"))?;
            be.disconnect().map_err(|e| format!("uinput disconnect: {e}"))?;
        }
        ProbeResult::Libei => {
            let mut be = LibeiBackend::new(config.libei_restore_token.clone());
            be.connect().map_err(|e| format!("libei connect: {e}"))?;
            crate::engine::click(&mut be, &plan).map_err(|e| format!("libei click: {e}"))?;
            be.disconnect().map_err(|e| format!("libei disconnect: {e}"))?;
        }
        ProbeResult::None => return Err("no backend available".to_string()),
    }
    Ok(())
}
