//! `missingno-debugger [<rom>] [--port N] [--mcp] [--allow-attach] [--boot-rom
//! PATH]`: recognise the ROM through the core registry, put its console under
//! the debugger, and serve it — over HTTP by default, or as an MCP tool server
//! over stdio with `--mcp`. With `--mcp` and no ROM, the MCP server starts idle
//! and loads a ROM or attaches to a running session on request, so one static
//! server entry serves any ROM. `--allow-attach` additionally publishes this
//! process's own session for other clients to attach to.

use std::path::PathBuf;
use std::process::ExitCode;

use missingno_session::SharedSession;
use missingno_session::factory::{self, LoadOptions};
use missingno_debugger::http;

/// Matches the GUI crate's headless server default.
const DEFAULT_PORT: u16 = 3333;

const USAGE: &str =
    "usage: missingno-debugger [<rom>] [--port N] [--mcp] [--allow-attach] [--boot-rom PATH]";

struct Args {
    rom: Option<PathBuf>,
    port: u16,
    mcp: bool,
    allow_attach: bool,
    boot_rom: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut rom = None;
    let mut port = DEFAULT_PORT;
    let mut mcp = false;
    let mut allow_attach = false;
    let mut boot_rom = None;
    let mut iter = std::env::args().skip(1);
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--port" => {
                let value = iter.next().ok_or("--port needs a value")?;
                port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            "--boot-rom" => {
                boot_rom = Some(PathBuf::from(
                    iter.next().ok_or("--boot-rom needs a path")?,
                ));
            }
            "--mcp" => mcp = true,
            "--allow-attach" => allow_attach = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if rom.replace(PathBuf::from(other)).is_some() {
                    return Err("more than one ROM path given".to_string());
                }
            }
        }
    }
    Ok(Args {
        rom,
        port,
        mcp,
        allow_attach,
        boot_rom,
    })
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
    let boot_rom = match &args.boot_rom {
        Some(path) => Some(
            std::fs::read(path)
                .map_err(|e| format!("failed to read boot ROM {}: {e}", path.display()))?,
        ),
        None => None,
    };
    let options = LoadOptions {
        boot_rom,
        ..LoadOptions::default()
    };
    let console = factory::create_console_with(&rom_path, &rom, &options)?
        .ok_or_else(|| format!("no core recognises {}", rom_path.display()))?;
    let debugger = console
        .into_debugger()
        .map_err(|_| "this system has no debugger backend".to_string())?;
    let session = SharedSession::spawn(debugger);

    #[cfg(feature = "mcp")]
    let core_name = factory::factory_for(&rom_path, &rom)
        .map(|factory| factory.name)
        .unwrap_or("unknown");

    // Held for the lifetime of the server: dropping it removes the socket.
    #[cfg(all(unix, feature = "mcp"))]
    let _endpoint = if args.allow_attach {
        let title = session
            .handle()
            .with_session(|session| session.game_title());
        let publication = missingno_session::Publication {
            title,
            core: core_name.to_string(),
        };
        let endpoint = missingno_session::AttachEndpoint::open(session.handle(), publication)
            .map_err(|error| format!("could not publish the session: {error}"))?;
        eprintln!("session published at {}", endpoint.path().display());
        Some(endpoint)
    } else {
        None
    };
    #[cfg(not(all(unix, feature = "mcp")))]
    if args.allow_attach {
        return Err("this build cannot publish a session for attaching".to_string());
    }

    if args.mcp {
        #[cfg(feature = "mcp")]
        return missingno_debugger::mcp::serve(session, core_name).map_err(|e| e.to_string());
        #[cfg(not(feature = "mcp"))]
        return Err("this build has no MCP transport (enable the `mcp` feature)".to_string());
    }

    http::serve(session, args.port).map_err(|e| e.to_string())
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
