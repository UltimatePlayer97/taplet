mod app;
mod config;
mod doctor;
mod portal_rt;
mod engine;
mod error;
mod hotkey;
mod input;
mod ipc;
mod linux;
mod tray;
mod ui;

use std::process::ExitCode;

use ipc::RemoteCommand;

/// Portal application ID. Host (non-Flatpak) apps must register it, or portals
/// reject calls with "An app id is required". Matches the installed desktop file
/// name and the Flatpak ID used later.
pub const APP_ID: &str = "io.github.UltimatePlayer97.taplet";

const HELP: &str = "\
Taplet - Wayland-native autoclicker

Usage:
  taplet                Open the GUI
  taplet toggle         Start or stop clicking in the running instance
  taplet start          Start clicking in the running instance
  taplet stop           Stop clicking in the running instance
  taplet doctor         Diagnose backend and environment
  taplet doctor --click Same, plus one test click (libei may ask permission)
  taplet doctor --hotkey Also test-binds F6 via xx_hotkey_v1 and the portal
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
        Some("doctor") => doctor::run(&args),
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
