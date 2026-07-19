//! `missingno-debugger [<rom>] [--port N] [--mcp]`: recognise the ROM through
//! the core registry, put its console under the debugger, and serve it — over
//! HTTP by default, or as an MCP tool server over stdio with `--mcp`. With
//! `--mcp` and no ROM, the MCP server starts idle and loads a ROM on request,
//! so one static server entry serves any ROM.

use std::path::PathBuf;
use std::process::ExitCode;

use missingno_debugger::Session;
use missingno_debugger::factory;
use missingno_debugger::http;

/// Matches the GUI crate's headless server default.
const DEFAULT_PORT: u16 = 3333;

const USAGE: &str = "usage: missingno-debugger [<rom>] [--port N] [--mcp]";

struct Args {
    rom: Option<PathBuf>,
    port: u16,
    mcp: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut rom = None;
    let mut port = DEFAULT_PORT;
    let mut mcp = false;
    let mut iter = std::env::args().skip(1);
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--port" => {
                let value = iter.next().ok_or("--port needs a value")?;
                port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            "--mcp" => mcp = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if rom.replace(PathBuf::from(other)).is_some() {
                    return Err("more than one ROM path given".to_string());
                }
            }
        }
    }
    Ok(Args { rom, port, mcp })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    // An idle MCP server needs no ROM up front; every other mode does.
    if args.mcp && args.rom.is_none() {
        #[cfg(feature = "mcp")]
        return missingno_debugger::mcp::serve_idle().map_err(|e| e.to_string());
        #[cfg(not(feature = "mcp"))]
        return Err("this build has no MCP transport (enable the `mcp` feature)".to_string());
    }

    let rom_path = args.rom.ok_or(USAGE)?;
    let rom = std::fs::read(&rom_path)
        .map_err(|e| format!("failed to read {}: {e}", rom_path.display()))?;
    let console = factory::create_console(&rom_path, &rom)?
        .ok_or_else(|| format!("no core recognises {}", rom_path.display()))?;
    let debugger = console
        .into_debugger()
        .map_err(|_| "this system has no debugger backend".to_string())?;

    if args.mcp {
        #[cfg(feature = "mcp")]
        {
            let core_name = factory::factory_for(&rom_path, &rom)
                .map(|factory| factory.name)
                .unwrap_or("unknown");
            return missingno_debugger::mcp::serve(Session::new(debugger), core_name)
                .map_err(|e| e.to_string());
        }
        #[cfg(not(feature = "mcp"))]
        return Err("this build has no MCP transport (enable the `mcp` feature)".to_string());
    }

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
