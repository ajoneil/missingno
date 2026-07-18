//! `missingno-debugger <rom> [--port N]`: recognise the ROM through the core
//! registry, put its console under the debugger, and serve it over HTTP.

use std::path::PathBuf;
use std::process::ExitCode;

use missingno_debugger::Session;
use missingno_debugger::factory;
use missingno_debugger::http;

/// Matches the GUI crate's headless server default.
const DEFAULT_PORT: u16 = 3333;

struct Args {
    rom: PathBuf,
    port: u16,
}

fn parse_args() -> Result<Args, String> {
    let mut rom = None;
    let mut port = DEFAULT_PORT;
    let mut iter = std::env::args().skip(1);
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--port" => {
                let value = iter.next().ok_or("--port needs a value")?;
                port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            "-h" | "--help" => return Err("usage: missingno-debugger <rom> [--port N]".to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if rom.replace(PathBuf::from(other)).is_some() {
                    return Err("more than one ROM path given".to_string());
                }
            }
        }
    }
    Ok(Args {
        rom: rom.ok_or("usage: missingno-debugger <rom> [--port N]")?,
        port,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let rom = std::fs::read(&args.rom)
        .map_err(|e| format!("failed to read {}: {e}", args.rom.display()))?;
    let console = factory::create_console(&args.rom, &rom)?
        .ok_or_else(|| format!("no core recognises {}", args.rom.display()))?;
    let debugger = console
        .into_debugger()
        .map_err(|_| "this system has no debugger backend".to_string())?;
    http::serve(Session::new(debugger), args.port).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
