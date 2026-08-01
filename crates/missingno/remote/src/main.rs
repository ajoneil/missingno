//! `missingno-remote [--mcp]`: an MCP-over-stdio server that discovers and drives
//! a running missingno app window through the UI-automation socket the app
//! publishes with `--allow-ui-automation`. Serving over stdio is the only mode,
//! so it serves regardless; `--mcp` is accepted for symmetry with
//! `missingno-debugger --mcp`.

mod mcp;
#[cfg(unix)]
mod ui_socket;

use std::process::ExitCode;

const USAGE: &str = "usage: missingno-remote [--mcp]";

fn main() -> ExitCode {
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--mcp" => {}
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("error: unknown option: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    match mcp::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
