#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{env, process::ExitCode};

mod app;
mod catalog;
mod core;

const USAGE: &str =
    "Usage: fflume [--help | --version]\n\nRun without arguments to open the FFlume desktop app.";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    match (args.next(), args.next()) {
        (None, _) => match app::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fflume: {error}");
                ExitCode::FAILURE
            }
        },
        (Some(arg), None) if arg == "--version" || arg == "-V" => {
            println!("fflume {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (Some(arg), None) if arg == "--help" || arg == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("fflume: unsupported arguments\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
