mod app;
mod config;
mod engine;
mod error;
mod input;
mod ipc;
mod ui;

use std::process::ExitCode;

use ipc::RemoteCommand;

const HELP: &str = "\
Taplet - Wayland-native autoclicker

Usage:
  taplet            Open the GUI
  taplet toggle     Start or stop clicking in the running instance
  taplet start      Start clicking in the running instance
  taplet stop       Stop clicking in the running instance
  taplet --help     Show this help
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
